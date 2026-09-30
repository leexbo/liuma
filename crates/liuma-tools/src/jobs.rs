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

/// 注册表内部:records 互斥 + 变更回调(分锁,回调内可再锁 records)
struct JobsInner {
    records: Mutex<Vec<JobRecord>>,
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

    /// 通知状态变更(records guard 释放后调用,回调内可再锁)
    pub(crate) fn changed(&self) {
        let hook = self.inner.on_change.lock().ok().and_then(|s| s.clone());
        if let Some(hook) = hook {
            hook();
        }
    }

    /// 弱引用(宿主广播源持有)
    pub fn downgrade(&self) -> WeakJobsRegistry {
        WeakJobsRegistry(Arc::downgrade(&self.inner))
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

/// 下一个任务 id
pub fn next_job_id(registry: &JobsRegistry) -> u64 {
    registry
        .lock()
        .map(|r| r.iter().map(|j| j.id).max().unwrap_or(0) + 1)
        .unwrap_or(1)
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
                "description": "Manage background jobs started with bash run_in_background. list: show all jobs with status; read: show a job's output (tail; streamed live while the job runs); stop: terminate a running job.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["list", "read", "stop"], "description": "Operation to perform" },
                        "id": { "type": "integer", "description": "Job id (action=read/stop)" }
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
            return fail("jobs requires arguments.action (list/read/stop)".into());
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

    #[test]
    fn next_id_is_monotonic() {
        let registry = JobsRegistry::new();
        assert_eq!(next_job_id(&registry), 1);
        registry.lock().unwrap().push(JobRecord {
            id: 1,
            command: "x".into(),
            status: "done".into(),
            log_path: PathBuf::from("/tmp/x.log"),
            killer: None,
            grace: std::time::Duration::from_secs(1),
            started_at: 0,
            ended_at: None,
        });
        assert_eq!(next_job_id(&registry), 2);
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
}
