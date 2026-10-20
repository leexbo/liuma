//! 后台任务:jobs 注册表 + jobs 工具。
//!
//! 单边界规则下的归属:后台任务 = 进程 + 输出文件(`.liuma/jobs/<id>.log`,
//! 文件即持久事实,不镜像入会话日志);`jobs` 工具读取时经正常 tool/result
//! 进入会话日志(读取路径就是普通工具调用)。注册表本身是瞬态活体
//! (进程终结即失活),不伪造持久事件。
//!
//! 日志由 watcher 流式落盘:任务运行中 `read` 即有内容(tail 尾读,
//! 不整读进内存——dev server 类日志大小无界)。注册表带状态变更回调:
//! 登记/收尾后触发,宿主接线广播帧用(不设回调则零开销)。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};

use liuma_agent_loop::{ToolCallRequest, ToolOutput, ToolPort};
use serde_json::{Value, json};

/// epoch 毫秒(时间戳字段同源;与 subagent 侧一致)
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// job 阻塞等待缺省预算(60000ms)与上限(600000ms;对齐前台 timeout_ms 上限)
pub const JOB_WAIT_DEFAULT_MS: u64 = 60_000;
pub const JOB_WAIT_MAX_MS: u64 = 600_000;

/// 单条后台任务记录(瞬态活体;输出文件才是持久事实)
pub struct JobRecord {
    /// 任务标识(单调递增)
    pub id: u64,
    /// 命令行
    pub command: String,
    /// 状态:running / done / failed / stopped
    pub status: String,
    /// 输出文件(watcher 流式落盘,启动即建)
    pub log_path: PathBuf,
    /// 终止句柄(stop 用;与 Child/Session 分离,不等 stdout 读)
    pub killer: Option<JobKiller>,
    /// 终止宽限(pipes 路径;PTY 无宽限语义)
    pub grace: std::time::Duration,
    /// 启动时刻(epoch ms;帧与通知用)
    pub started_at: i64,
    /// 落定时刻(epoch ms;running 期为 None)
    pub ended_at: Option<i64>,
    /// 退出码(被信号杀死时为 None;watcher 收尾记录)
    pub exit_code: Option<i32>,
    /// 终止信号(Unix;自然退出为 None)
    pub signal: Option<i32>,
    /// 结算瞬间有活等待者(wait 结果即终态交付)→ 端口通知抑制。
    /// 判定在收尾同步段一次性完成,无 await 竞态
    pub notice_suppressed: bool,
}

/// 后台任务终止句柄(pipes 组信号 / PTY killer 按 spawn 路径分派)
#[derive(Clone)]
pub enum JobKiller {
    /// pipes 路径:进程组信号(SIGTERM→grace→SIGKILL 分离终止)
    Group(liuma_sandbox::GroupKiller),
    /// PTY 路径:portable-pty killer(即杀,无宽限)
    Pty(liuma_sandbox::PtyKiller),
}

impl JobKiller {
    /// 分离终止(不持进程句柄;回收与状态收尾归 watcher)
    pub async fn kill_detached(&self, grace: std::time::Duration) -> bool {
        match self {
            JobKiller::Group(k) => k.kill_detached(grace).await,
            JobKiller::Pty(k) => {
                k.kill();
                true
            }
        }
    }
}

/// 状态变更回调(登记/状态收尾后触发;宿主广播帧接线用)
pub type JobsChangeHook = Arc<dyn Fn() + Send + Sync>;

/// 等待者表项:单 job 唤醒源 + 活等待计数(live>0 结算才算「已被
/// wait 交付」;超时守卫注销后计数归零,结算照常走端口通知)
struct WaitState {
    notify: Arc<tokio::sync::Notify>,
    live: usize,
}

/// 注册表内部:records / waiters / 变更回调(各自分锁,回调内可再锁 records)
struct JobsInner {
    records: Mutex<Vec<JobRecord>>,
    waiters: Mutex<HashMap<u64, WaitState>>,
    on_change: Mutex<Option<JobsChangeHook>>,
}

/// jobs 注册表(执行工具与控制工具共享;宿主可挂状态变更回调)
#[derive(Clone)]
pub struct JobsRegistry {
    inner: Arc<JobsInner>,
}

/// 注册表弱引用(宿主侧广播源防持有环)
pub struct WeakJobsRegistry(Weak<JobsInner>);

impl WeakJobsRegistry {
    /// 升级为强引用(失败 = 已释放)
    pub fn upgrade(&self) -> Option<JobsRegistry> {
        self.0.upgrade().map(|inner| JobsRegistry { inner })
    }
}

impl JobsRegistry {
    /// 空注册表
    pub fn new() -> Self {
        Self {
            inner: Arc::new(JobsInner {
                records: Mutex::new(Vec::new()),
                waiters: Mutex::new(HashMap::new()),
                on_change: Mutex::new(None),
            }),
        }
    }
    /// 挂状态变更回调(None = 摘除;缺省无回调零开销)
    pub fn set_on_change(&self, hook: Option<JobsChangeHook>) {
        if let Ok(mut slot) = self.inner.on_change.lock() {
            *slot = hook;
        }
    }

    /// 通知状态变更(records guard 释放后调用,回调内可再锁)。
    /// 直接改 records 的宿主侧(测试/桥接)负责手动触发;
    /// 工具侧路径(start_job/stop/watcher)已内建
    pub fn changed(&self) {
        let hook = self.inner.on_change.lock().ok().and_then(|s| s.clone());
        if let Some(hook) = hook {
            hook();
        }
    }

    /// 弱引用(宿主广播源持有)
    pub fn downgrade(&self) -> WeakJobsRegistry {
        WeakJobsRegistry(Arc::downgrade(&self.inner))
    }

    /// 注册活等待者,返回(唤醒源,守卫)。调用方必须「先注册后复查
    /// 状态」:注册与结算之间的窗口由 Notify 补齐——结算方
    /// notify_waiters 唤醒注册在其之前的全部等待者,复查兜住更早的落定
    pub fn register_waiter(&self, id: u64) -> (Arc<tokio::sync::Notify>, JobWaitGuard) {
        let notify = {
            let mut w = self.inner.waiters.lock().unwrap_or_else(|p| p.into_inner());
            let st = w.entry(id).or_insert_with(|| WaitState {
                notify: Arc::new(tokio::sync::Notify::new()),
                live: 0,
            });
            st.live += 1;
            st.notify.clone()
        };
        (
            notify,
            JobWaitGuard {
                registry: self.clone(),
                id,
            },
        )
    }

    /// 结算唤醒:唤醒该 job 的全部活等待者,返回「结算瞬间有活等待者」
    /// (true = 终态已随 wait 结果交付,端口通知应抑制)。表项由守卫
    /// 注销清理,此处不摘除——唤醒后等待者还要读 records 终态
    pub fn wake_waiters(&self, id: u64) -> bool {
        let notify = self
            .inner
            .waiters
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id)
            .filter(|st| st.live > 0)
            .map(|st| st.notify.clone());
        match notify {
            Some(n) => {
                n.notify_waiters();
                true
            }
            None => false,
        }
    }
}

/// 活等待者守卫:drop 即注销(超时/提前返回路径的自动退场——注销后
/// 结算瞬间无活等待者,通知照常投递)
pub struct JobWaitGuard {
    registry: JobsRegistry,
    id: u64,
}

impl Drop for JobWaitGuard {
    fn drop(&mut self) {
        let mut w = self
            .registry
            .inner
            .waiters
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(st) = w.get_mut(&self.id) {
            st.live = st.live.saturating_sub(1);
            if st.live == 0 {
                w.remove(&self.id);
            }
        }
    }
}

impl Default for JobsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// records 直访(既有 `.lock()` 调用点零改)
impl std::ops::Deref for JobsRegistry {
    type Target = Mutex<Vec<JobRecord>>;

    fn deref(&self) -> &Self::Target {
        &self.inner.records
    }
}

/// 下一个任务 id:内存峰值与 `floor`(磁盘现存日志的最大 id)取大者
/// 再 +1。日志跨重启驻留于工作区而注册表纯内存,仅按内存计数会让
/// 重启后的新任务复用旧 id——`File::create` 截断写,旧日志随之被静默
/// 销毁;磁盘即事实源,用户手删旧日志则自然回落
pub fn next_job_id(registry: &JobsRegistry, floor: u64) -> u64 {
    registry
        .lock()
        .map(|r| r.iter().map(|j| j.id).max().unwrap_or(0).max(floor) + 1)
        .unwrap_or(floor + 1)
}

/// 目录里现存日志的最大 id(`N.log` 命名;目录缺席/无文件/非数字名
/// 一律跳过)= 0
pub fn max_disk_job_id(jobs_dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(jobs_dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            name.strip_suffix(".log")?.parse::<u64>().ok()
        })
        .max()
        .unwrap_or(0)
}

/// shell job 结算通知拼装(结算摘要 + 输出尾部 → 模型可见文本;
/// source 染色载荷)。形态对齐 subagent 侧 settlement_notice
pub fn job_settlement_notice(
    id: u64,
    command: &str,
    status_label: &str,
    exit_code: Option<i32>,
    signal: Option<&str>,
    tail: &str,
) -> (String, Value) {
    let subject = format!("Background job {id} ({command})");
    let summary = match status_label {
        "done" => format!("{subject} finished successfully."),
        "stopped" => format!("{subject} was stopped."),
        "failed" => match (exit_code, signal) {
            (Some(code), _) => format!("{subject} exited with code {code}."),
            (None, Some(sig)) => format!("{subject} was terminated by {sig}."),
            _ => format!("{subject} failed before it finished."),
        },
        other => format!("{subject} ended abnormally ({other})."),
    };
    let mut text = summary.clone();
    if !tail.trim().is_empty() {
        text.push_str("\n\nTail of its output:\n\n");
        text.push_str(tail.trim());
    }
    let source = json!({
        "kind": "shell-job-settled",
        "form": "notice",
        "summary": summary,
        "jobId": id,
        "command": command,
    });
    (text, source)
}

/// jobs 工具:list / stop / read
pub struct JobTool {
    /// 注册表(与 BashTool 后台路径共享)
    pub registry: JobsRegistry,
}

impl JobTool {
    /// 以共享注册表构建
    pub fn new(registry: JobsRegistry) -> Self {
        Self { registry }
    }

    /// 尾读日志(最后 8KB → 解码 → 截 4096 字符)。流式落盘后文件
    /// 大小无界,不整读进内存;文件缺席(启动失败)= 如实报读不了
    async fn read_log(&self, path: &PathBuf) -> String {
        const TAIL_BYTES: u64 = 8192;
        let read = match tokio::fs::File::open(path).await {
            Ok(file) => file,
            Err(e) => return format!("(log unreadable: {e})"),
        };
        let mut file = read;
        let len = file.metadata().await.map(|m| m.len()).unwrap_or(0);
        let mut bytes = Vec::new();
        {
            use tokio::io::{AsyncReadExt, AsyncSeekExt};
            // seek 到尾窗起点(失败从 0 读,窗口偏大而已)
            let _ = file
                .seek(std::io::SeekFrom::Start(len.saturating_sub(TAIL_BYTES)))
                .await;
            let _ = file.read_to_end(&mut bytes).await;
        }
        // 按字节读再解码:日志原样落的是子进程输出,平台 shell 未必写 UTF-8
        // (见 liuma_sandbox::text),read_to_string 会因非 UTF-8 整条读失败
        let text = liuma_sandbox::text::decode_output(&bytes);
        let chars: Vec<char> = text.chars().collect();
        if chars.len() > 4096 {
            chars[chars.len() - 4096..].iter().collect()
        } else {
            text
        }
    }
}

impl ToolPort for JobTool {
    fn specs(&self) -> Vec<Value> {
        vec![json!({
            "type": "function",
            "function": {
                "name": "jobs",
                "description": "Manage background jobs started with bash run_in_background. list: show all jobs with status; read: show a job's output (tail; streamed live while the job runs); wait: block until a job settles or the timeout elapses (prefer this over sleeping in a shell command); stop: terminate a running job.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["list", "read", "wait", "stop"], "description": "Operation to perform" },
                        "id": { "type": "integer", "description": "Job id (action=read/wait/stop)" },
                        "timeout_ms": { "type": "integer", "description": "Max wait in milliseconds for action=wait (default 60000, max 600000). A timed-out wait returns the current output and leaves the job running." }
                    },
                    "required": ["action"],
                },
            },
        })]
    }

    async fn execute(&mut self, call: &ToolCallRequest) -> ToolOutput {
        if call.name != "jobs" {
            return ToolOutput {
                output: format!("unknown tool: {}", call.name),
                success: false,
                ..Default::default()
            };
        }
        let fail = |msg: String| ToolOutput {
            output: msg,
            success: false,
            ..Default::default()
        };
        let arguments: Value = if let Some(s) = call.arguments.as_str() {
            serde_json::from_str(s).unwrap_or(json!({}))
        } else {
            call.arguments.clone()
        };
        let Some(action) = arguments["action"].as_str() else {
            return fail("jobs requires arguments.action (list/read/wait/stop)".into());
        };
        let id = arguments["id"].as_u64();
        // stop 先行处理(锁内取句柄,锁外 await——guard 不可跨 await)
        if action == "stop" {
            let Some(id) = id else {
                return fail("jobs stop requires arguments.id (integer)".into());
            };
            // 先行置位 stopped,再发信号:watcher 只改 running 状态,
            // 置位在前即不会被 watcher 的退出收尾覆盖为 failed
            let (killer, grace) = {
                let Ok(mut registry) = self.registry.lock() else {
                    return fail("jobs registry unavailable".into());
                };
                let Some(job) = registry.iter_mut().find(|j| j.id == id) else {
                    return fail(format!("job {id} not found"));
                };
                if job.status != "running" {
                    return fail(format!("job {id} is not running ({})", job.status));
                }
                let Some(killer) = job.killer.clone() else {
                    return fail(format!("job {id} handle unavailable"));
                };
                job.status = "stopped".into();
                (killer, job.grace)
            };
            self.registry.changed();
            // 分离终止:不持 Child(其正被 watcher 的 stdout 读占用),
            // 直接组信号;回收与状态收尾由 watcher 负责
            let killed = killer.kill_detached(grace).await;
            return ToolOutput {
                output: format!("stopping job {id} (signal sent: {killed})"),
                success: true,
                ..Default::default()
            };
        }
        // wait:阻塞等待结算(先注册后复查防丢唤醒;Notify 等待在
        // 锁外,超时守卫注销后结算照常走端口通知)
        if action == "wait" {
            let Some(id) = id else {
                return fail("jobs wait requires arguments.id (integer)".into());
            };
            let timeout_ms = arguments["timeout_ms"]
                .as_u64()
                .unwrap_or(JOB_WAIT_DEFAULT_MS)
                .clamp(1, JOB_WAIT_MAX_MS);
            enum WaitLane {
                /// 已结算(调用前或注册复查窗口内落定)
                Settled,
                /// 运行中:挂唤醒源阻塞
                Blocked(Arc<tokio::sync::Notify>, JobWaitGuard),
            }
            let lane = {
                let Ok(registry) = self.registry.lock() else {
                    return fail("jobs registry unavailable".into());
                };
                match registry.iter().find(|j| j.id == id) {
                    None => return fail(format!("job {id} not found")),
                    Some(job) if job.status != "running" => WaitLane::Settled,
                    Some(_) => {
                        let (notify, guard) = self.registry.register_waiter(id);
                        // 先注册后复查:注册与结算之间的窗口在此兜住
                        if registry
                            .iter()
                            .find(|j| j.id == id)
                            .is_none_or(|j| j.status != "running")
                        {
                            WaitLane::Settled
                        } else {
                            WaitLane::Blocked(notify, guard)
                        }
                    }
                }
            };
            if let WaitLane::Blocked(notify, guard) = lane {
                if tokio::time::timeout(
                    std::time::Duration::from_millis(timeout_ms),
                    notify.notified(),
                )
                .await
                .is_err()
                {
                    // 超时:守卫就地注销(此后结算瞬间无活等待者 →
                    // 通知照常)。返回运行中快照,不是错误——是否再等
                    // 由模型决定
                    let (_status, command, log_path) = {
                        let Ok(registry) = self.registry.lock() else {
                            return fail("jobs registry unavailable".into());
                        };
                        match registry.iter().find(|j| j.id == id) {
                            Some(job) => (
                                job.status.clone(),
                                job.command.clone(),
                                job.log_path.clone(),
                            ),
                            None => return fail(format!("job {id} not found")),
                        }
                    };
                    let text = self.read_log(&log_path).await;
                    return ToolOutput {
                        output: format!(
                            "[status: running] {command}\n{text}\n[wait timed out after {timeout_ms}ms; the job keeps running — wait again or read it later]"
                        ),
                        success: true,
                        ..Default::default()
                    };
                }
                drop(guard); // 已唤醒 = 已结算:守卫使命完成
            }
            // 终态快照 + 输出(watcher 收尾已 flush;锁内取字段锁外读)
            let (status, command, log_path, exit_code, signal) = {
                let Ok(registry) = self.registry.lock() else {
                    return fail("jobs registry unavailable".into());
                };
                match registry.iter().find(|j| j.id == id) {
                    Some(job) => (
                        job.status.clone(),
                        job.command.clone(),
                        job.log_path.clone(),
                        job.exit_code,
                        job.signal,
                    ),
                    None => return fail(format!("job {id} not found")),
                }
            };
            let text = self.read_log(&log_path).await;
            let mut output = format!("[{status}] {command}\n{text}");
            if let Some(code) = exit_code {
                output.push_str(&format!("\n[exit code: {code}]"));
            } else if let Some(sig) = signal {
                output.push_str(&format!("\n[terminated by signal {sig}]"));
            }
            return ToolOutput {
                output,
                success: true,
                ..Default::default()
            };
        }
        // read 的文件 IO 在锁外(await 不持 records guard)
        if action == "read" {
            let Some(id) = id else {
                return fail("jobs read requires arguments.id (integer)".into());
            };
            let job = {
                let Ok(registry) = self.registry.lock() else {
                    return fail("jobs registry unavailable".into());
                };
                let Some(job) = registry.iter().find(|j| j.id == id) else {
                    return fail(format!("job {id} not found"));
                };
                (
                    job.status.clone(),
                    job.command.clone(),
                    job.log_path.clone(),
                )
            };
            let (status, command, log_path) = job;
            let text = self.read_log(&log_path).await;
            return ToolOutput {
                output: format!("[{}] {}\n{}", status, command, text),
                success: true,
                ..Default::default()
            };
        }
        let Ok(registry) = self.registry.lock() else {
            return fail("jobs registry unavailable".into());
        };
        match action {
            "list" => {
                if registry.is_empty() {
                    return ToolOutput {
                        output: "(no jobs)".into(),
                        success: true,
                        ..Default::default()
                    };
                }
                let lines: Vec<String> = registry
                    .iter()
                    .map(|j| format!("{}. [{}] {}", j.id, j.status, j.command))
                    .collect();
                ToolOutput {
                    output: lines.join("\n"),
                    success: true,
                    ..Default::default()
                }
            }
            other => fail(format!("unknown action: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{JobNotify, SettlementNotificationPort};
    use liuma_sandbox::process::ExitStatus;
    use std::future::Future;
    use std::pin::Pin;

    #[test]
    fn next_id_is_monotonic() {
        let registry = JobsRegistry::new();
        assert_eq!(next_job_id(&registry, 0), 1);
        registry.lock().unwrap().push(JobRecord {
            id: 1,
            command: "x".into(),
            status: "done".into(),
            log_path: PathBuf::from("/tmp/x.log"),
            killer: None,
            grace: std::time::Duration::from_secs(1),
            started_at: 0,
            ended_at: None,
            exit_code: None,
            signal: None,
            notice_suppressed: false,
        });
        assert_eq!(next_job_id(&registry, 0), 2);
    }

    /// 重启回归锁:注册表清零后新 id 不得复用磁盘上的旧日志号——
    /// 日志文件以 `File::create` 截断写,复用即静默销毁旧输出
    #[test]
    fn next_id_seeds_from_disk_floor_after_restart() {
        let dir = std::env::temp_dir().join(format!("liuma-jobs-floor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("3.log"), b"old").unwrap();
        std::fs::write(dir.join("7.log"), b"old").unwrap();
        std::fs::write(dir.join("not-a-number.log"), b"skip").unwrap();

        let fresh = JobsRegistry::new();
        assert_eq!(max_disk_job_id(&dir), 7);
        assert_eq!(
            next_job_id(&fresh, max_disk_job_id(&dir)),
            8,
            "越过磁盘峰值"
        );

        // 内存峰值与磁盘取大者
        fresh.lock().unwrap().push(JobRecord {
            id: 9,
            command: "x".into(),
            status: "running".into(),
            log_path: PathBuf::from("/tmp/x.log"),
            killer: None,
            grace: std::time::Duration::from_secs(1),
            started_at: 0,
            ended_at: None,
            exit_code: None,
            signal: None,
            notice_suppressed: false,
        });
        assert_eq!(next_job_id(&fresh, max_disk_job_id(&dir)), 10);

        // 目录缺席 = 0 下限,行为与旧实现的空注册表一致
        assert_eq!(
            next_job_id(
                &JobsRegistry::new(),
                max_disk_job_id(&std::env::temp_dir().join("liuma-jobs-absent-seed"))
            ),
            1
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 变更回调:登记与状态收尾后触发;回调内再锁 records 不死锁
    #[test]
    fn change_hook_fires_after_registry_writes() {
        let registry = JobsRegistry::new();
        let fired = Arc::new(Mutex::new(0u32));
        let counter = Arc::clone(&fired);
        registry.set_on_change(Some(Arc::new(move || {
            if let Ok(mut records) = counter.lock() {
                *records += 1;
            }
        })));
        registry.lock().unwrap().push(JobRecord {
            id: 1,
            command: "x".into(),
            status: "running".into(),
            log_path: PathBuf::from("/tmp/x.log"),
            killer: None,
            grace: std::time::Duration::from_secs(1),
            started_at: 0,
            ended_at: None,
            exit_code: None,
            signal: None,
            notice_suppressed: false,
        });
        registry.changed();
        {
            let mut records = registry.lock().unwrap();
            records[0].status = "done".into();
        }
        registry.changed();
        registry.set_on_change(None);
        registry.changed();
        assert_eq!(
            *fired.lock().unwrap(),
            2,
            "登记 + 收尾各一次,摘除后不再触发"
        );
    }

    /// 弱引用生命周期:强引用全释即升级失败
    #[test]
    fn weak_registry_upgrade_follows_lifecycle() {
        let weak = {
            let registry = JobsRegistry::new();
            registry.downgrade()
        };
        assert!(weak.upgrade().is_none());
    }

    /// 通知变体:done/stopped/failed(码/信号/不可知);空尾省略
    /// Output tail;source 染色字段
    #[test]
    fn job_settlement_notice_variants() {
        let (text, source) =
            job_settlement_notice(7, "cargo build", "done", Some(0), None, "tail text");
        assert!(text.contains("Background job 7 (cargo build) finished successfully."));
        assert!(text.contains("Tail of its output:\n\ntail text"));
        assert_eq!(source["kind"], "shell-job-settled");
        assert_eq!(source["jobId"], 7);
        assert_eq!(source["command"], "cargo build");

        let (text, _) = job_settlement_notice(8, "x", "stopped", None, None, "");
        assert!(text.contains("was stopped"));
        assert!(!text.contains("Tail of its output"), "空尾省略输出段");

        let (text, _) = job_settlement_notice(9, "x", "failed", Some(2), None, "");
        assert!(text.contains("exited with code 2"));

        let (text, _) = job_settlement_notice(10, "x", "failed", None, Some("SIGKILL"), "");
        assert!(text.contains("terminated by SIGKILL"));

        let (text, _) = job_settlement_notice(11, "x", "weird", None, None, "");
        assert!(text.contains("ended abnormally (weird)"));
    }

    /// 结算通知录制口(wait 去重断言用)
    #[derive(Default)]
    struct RecordingPort {
        calls: Mutex<Vec<String>>,
    }

    impl SettlementNotificationPort for RecordingPort {
        fn notify(
            &self,
            _parent_session: &str,
            text: String,
            _source: Value,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            Box::pin(async move {
                if let Ok(mut c) = self.calls.lock() {
                    c.push(text);
                }
            })
        }
    }

    fn push_record(registry: &JobsRegistry, id: u64, status: &str, log_path: PathBuf) {
        registry.lock().unwrap().push(JobRecord {
            id,
            command: "cargo build".into(),
            status: status.into(),
            log_path,
            killer: None,
            grace: std::time::Duration::from_secs(1),
            started_at: 0,
            ended_at: None,
            exit_code: None,
            signal: None,
            notice_suppressed: false,
        });
    }

    fn wait_call(id: u64, timeout_ms: Option<u64>) -> ToolCallRequest {
        let mut args = json!({ "action": "wait", "id": id });
        if let Some(t) = timeout_ms {
            args["timeout_ms"] = json!(t);
        }
        ToolCallRequest {
            name: "jobs".into(),
            arguments: args,
            id: String::new(),
        }
    }

    /// wait 快路径:已结算立即返回终态(退出码尾标 / 信号尾标)
    #[tokio::test]
    async fn wait_returns_settled_job_immediately() {
        let registry = JobsRegistry::new();
        let dir = std::env::temp_dir().join(format!("liuma-jobs-wait-done-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("1.log"), b"built ok\n").unwrap();
        std::fs::write(dir.join("2.log"), b"killed\n").unwrap();
        push_record(&registry, 1, "done", dir.join("1.log"));
        push_record(&registry, 2, "failed", dir.join("2.log"));
        {
            let mut r = registry.lock().unwrap();
            r[0].exit_code = Some(0);
            r[1].signal = Some(9);
        }
        let mut tool = JobTool::new(registry);
        let out = tool.execute(&wait_call(1, None)).await;
        assert!(out.success, "{}", out.output);
        assert!(out.output.contains("[done] cargo build"), "{}", out.output);
        assert!(out.output.contains("built ok"));
        assert!(out.output.contains("[exit code: 0]"), "{}", out.output);
        let out = tool.execute(&wait_call(2, None)).await;
        assert!(
            out.output.contains("[failed] cargo build"),
            "{}",
            out.output
        );
        assert!(
            out.output.contains("[terminated by signal 9]"),
            "{}",
            out.output
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 去重:结算瞬间有活等待者 → 等待者被唤醒、终态经 wait 交付、
    /// 端口通知抑制(判定在收尾同步段一次性完成,无 await 竞态)
    #[tokio::test]
    async fn settle_with_live_waiter_suppresses_notice() {
        let registry = JobsRegistry::new();
        let dir =
            std::env::temp_dir().join(format!("liuma-jobs-wait-dedup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("1.log"), b"partial\n").unwrap();
        push_record(&registry, 1, "running", dir.join("1.log"));
        let port = Arc::new(RecordingPort::default());
        let notify: JobNotify = (port.clone(), "s-parent".into());
        // 模拟 wait 动作:注册等待者并持有守卫(活等待)
        let (woken, _guard) = registry.register_waiter(1);
        let reg = registry.clone();
        let status = ExitStatus {
            code: Some(0),
            signal: None,
        };
        let settle = tokio::spawn(async move {
            crate::finish_job(&reg, 1, Some(status));
            crate::notify_job_settled(
                &reg,
                1,
                "cargo build",
                Some(status),
                b"done text\n",
                Some(&notify),
            )
            .await;
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), woken.notified())
            .await
            .expect("活等待者未被结算唤醒");
        settle.await.unwrap();
        assert!(
            port.calls.lock().unwrap().is_empty(),
            "终态已随 wait 交付,端口通知不得重复"
        );
        assert!(registry.lock().unwrap()[0].notice_suppressed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// wait 超时:返回运行中快照(success=true 不是错误);守卫注销后
    /// 结算 → 端口通知照常(不丢)
    #[tokio::test]
    async fn wait_timeout_keeps_notice_path_alive() {
        let registry = JobsRegistry::new();
        let dir =
            std::env::temp_dir().join(format!("liuma-jobs-wait-timeout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("1.log"), b"still going\n").unwrap();
        push_record(&registry, 1, "running", dir.join("1.log"));
        let port = Arc::new(RecordingPort::default());
        let mut tool = JobTool::new(registry.clone());
        let out = tool.execute(&wait_call(1, Some(30))).await;
        assert!(out.success, "{}", out.output);
        assert!(out.output.contains("[status: running]"), "{}", out.output);
        assert!(
            out.output.contains("[wait timed out after 30ms"),
            "{}",
            out.output
        );
        // 守卫已随超时注销:结算走端口通知,不丢
        let notify: JobNotify = (port.clone(), "s-parent".into());
        let status = ExitStatus {
            code: Some(2),
            signal: None,
        };
        crate::finish_job(&registry, 1, Some(status));
        crate::notify_job_settled(
            &registry,
            1,
            "cargo build",
            Some(status),
            b"boom\n",
            Some(&notify),
        )
        .await;
        let calls = port.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "超时注销后结算必须照常通知");
        assert!(calls[0].contains("exited with code 2"), "{}", calls[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
