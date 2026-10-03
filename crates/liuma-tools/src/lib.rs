//! tools 组件:工具 schema 注册与执行编排(native 阶段)。
//!
//! `BashTool` 把沙箱链接进 agent loop:工具执行 = 宿主 spawn
//! (fail-closed 沙箱、SIGTERM→grace→SIGKILL、独立进程组),
//! 输出经 tool/result 事件记录后进入下一轮请求派生(记录 ⟺ 可见)。
//! 执行世界能力束 = BashTool 携带的 cwd + SandboxPolicy,
//! 整体传递、可窄化(「执行世界」思想)。

use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use liuma_agent_loop::{CancelToken, ToolCallRequest, ToolOutput, ToolPort, ToolView};
use liuma_sandbox::process::ExitStatus;
use liuma_sandbox::pty::spawn_pty;
use liuma_sandbox::shell;
use liuma_sandbox::{ExitClass, SandboxMode, SandboxPolicy};
use liuma_sandbox::{SpawnOptions, spawn as spawn_child};
use serde_json::{Value, json};

pub mod ask_question;
pub mod file;
pub mod goal;
pub mod jobs;
pub mod session_query;
pub mod subagent;
pub mod todo;
pub mod workflow;

/// 会话权限模式动态源:工具执行时解析——日志 fold,权限
/// 事件落档即对下一次执行生效,无需重装配。缺省 = 装配期静态策略
/// (CLI/测试装配)。
pub type ModeSource = std::sync::Arc<dyn Fn() -> SandboxMode + Send + Sync>;

/// 沙箱升级审批裁决
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    /// 批准一次(只盖发起该请求的本次调用,不落 sandbox/mode 事件)
    AllowedOnce,
    /// 用户拒绝(对该命令终局:停止解释,不绕行)
    Rejected,
    /// 用户取消/通道中止
    Cancelled,
    /// 无可用审批通道
    Unavailable,
}

/// 沙箱升级请求(闸门审计与问询载荷;目标模式必须严格加宽,工具侧已验)
#[derive(Debug, Clone)]
pub struct EscalationRequest {
    /// 发起工具名(bash / …)
    pub tool_name: String,
    /// 关联工具调用 id(缺 = 引擎未透传)
    pub call_id: Option<String>,
    /// 待执行命令原文(审批卡信任锚:用户看命令,不只是看理由)
    pub command: String,
    /// 目标模式
    pub target_mode: SandboxMode,
    /// 模型给出的一句话理由
    pub justification: String,
}

/// 宿主审批闸门(批准先于执行):实现方负责审计对落档(approval/asked·
/// decided)与用户问询;approval=never 时实现方在入口直接拒绝
/// (不问任何应答方,不可绕过)。
pub trait ApprovalPort: Send + Sync {
    fn request(
        &self,
        req: EscalationRequest,
    ) -> Pin<Box<dyn Future<Output = ApprovalOutcome> + Send>>;
}

/// 拒绝提示链(沙箱拒绝输出的下一行;教模型带参重试一次,审批问用户)
pub const ESCALATION_HINT: &str = "[sandbox: escalation available — retry this exact command once with sandbox_permissions (the narrowest wider mode that suffices) + justification; the approval prompt asks the user]";

/// 前台命令缺省墙钟预算(120s;到点转后台不杀)
pub const BASH_TIMEOUT_DEFAULT_MS: u64 = 120_000;
/// 前台预算上限(600s;更长的活儿用 run_in_background,超限拒绝而非钳制)
pub const BASH_TIMEOUT_MAX_MS: u64 = 600_000;

/// 结算通知 port:后台执行体(子代理 / shell job)→ 发起会话的单向
/// 通知通道。宿主实现:经 Notice 入会话队列——空闲 = 唤醒下一 turn,
/// 忙碌 = 引擎 step 边界认领(followup/steer 双语义)。目标会话不
/// 存在时静默丢弃(发起方不再存活不是错误,子会话/日志自身即记录)
pub trait SettlementNotificationPort: Send + Sync {
    /// 投递一条通知(text = 模型可见文本;source = 染色载荷)
    fn notify(
        &self,
        parent_session: &str,
        text: String,
        source: Value,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// job 结算通知口(通知通道 + 发起会话 id)
pub type JobNotify = (std::sync::Arc<dyn SettlementNotificationPort>, String);

pub use ask_question::{AskQuestionPort, AskQuestionTool, QuestionItem, QuestionOption};
pub use file::FileTools;
pub use goal::GoalTool;
pub use jobs::{
    JobKiller, JobRecord, JobTool, JobsRegistry, WeakJobsRegistry, job_settlement_notice,
    max_disk_job_id, next_job_id,
};
pub use subagent::{SubagentControlTool, SubagentRecord, SubagentRegistry, SubagentTool};
pub use todo::TodoWriteTool;
pub use workflow::{RALPH_DONE, RalphTool, WorkflowTool};

/// bash 工具:命令执行经沙箱链(pipes 或 PTY)
pub struct BashTool {
    /// 工作目录(同时是默认可写根)
    pub cwd: PathBuf,
    /// 沙箱策略(执行世界能力束;fail-closed)
    pub policy: SandboxPolicy,
    /// 终止宽限(SIGTERM→grace→SIGKILL 的 grace)
    pub grace: Duration,
    /// PTY 模式(需要终端语义的命令:isatty/彩色/行缓冲)
    pub pty: bool,
    /// 软取消令牌(执行中 select:取消即杀子进程并温和返回)
    pub cancel: CancelToken,
    /// 后台任务注册表(run_in_background 路径;None = 不支持后台)
    pub jobs: Option<JobsRegistry>,
    /// 会话权限模式动态源(execute 时解析;缺省 = 装配期静态 policy)
    pub mode_source: Option<ModeSource>,
    /// 宿主审批闸门(沙箱一次性升级;None = 无升级能力,hint 亦不提示)
    pub approval: Option<std::sync::Arc<dyn ApprovalPort>>,
    /// job 结算通知口(None = 此装配不投通知;schema 文案与承诺据此分支)
    pub job_notify: Option<JobNotify>,
}

impl BashTool {
    /// 以工作目录构建:workspace-write 策略(cwd + 平台临时区可写;
    /// 编译类工具在 /tmp 落中间产物
    /// 不会被拦,与 write/file 工具能力同源)
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        let cwd = cwd.into();
        Self {
            policy: SandboxPolicy::workspace_write(cwd.clone()),
            cwd,
            grace: Duration::from_secs(5),
            pty: false,
            cancel: CancelToken::new(),
            jobs: None,
            mode_source: None,
            approval: None,
            job_notify: None,
        }
    }

    /// 覆写沙箱策略(访问模式:read-only = 无可写根;full-access = 全盘可写)
    pub fn with_policy(mut self, policy: SandboxPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// 挂动态模式源(会话日志 fold;权限切换落档即生效,空闲与运行中一致)
    pub fn with_mode_source(mut self, source: ModeSource) -> Self {
        self.mode_source = Some(source);
        self
    }

    /// 注入宿主审批闸门(启用 sandbox_permissions 一次性升级)
    pub fn with_approval_port(mut self, port: std::sync::Arc<dyn ApprovalPort>) -> Self {
        self.approval = Some(port);
        self
    }

    /// 执行时解析策略:动态源优先,静态 policy 兜底。ReadOnly 仍走沙箱链
    /// (无可写根)——读命令可用,写被内核拦并带拒绝标记,优于装配期
    /// 「不装 bash」的粗粒度压制
    fn resolve_policy(&self) -> SandboxPolicy {
        match self.mode_source.as_ref().map(|f| f()) {
            Some(SandboxMode::FullAccess) => SandboxPolicy::full_access(),
            Some(SandboxMode::WorkspaceWrite) => SandboxPolicy::workspace_write(self.cwd.clone()),
            Some(SandboxMode::ReadOnly) => SandboxPolicy::read_only(),
            None => self.policy.clone(),
        }
    }

    /// 严格加宽表:一次性升级只允许到「严格更宽」的档位
    /// (read-only → [workspace-write, full-access];workspace-write →
    /// [full-access];full-access 不可再升)
    fn wider_modes(mode: SandboxMode) -> &'static [SandboxMode] {
        match mode {
            SandboxMode::ReadOnly => &[SandboxMode::WorkspaceWrite, SandboxMode::FullAccess],
            SandboxMode::WorkspaceWrite => &[SandboxMode::FullAccess],
            SandboxMode::FullAccess => &[],
        }
    }

    /// 升级参数解析:两参成对 + justification 非空(错误文案逐字固定;
    /// 档位词汇 = RS 三态去 danger 命名)
    fn parse_escalation(arguments: &Value) -> Result<Option<(SandboxMode, String)>, String> {
        let perms = arguments["sandbox_permissions"].as_str();
        let just = arguments["justification"].as_str();
        match (perms, just) {
            (None, None) => Ok(None),
            (Some(_), None) => {
                Err("invalid escalation: sandbox_permissions requires a justification".into())
            }
            (None, Some(_)) => Err(
                "invalid escalation: justification is only valid together with sandbox_permissions"
                    .into(),
            ),
            (Some(mode), Some(just)) => {
                if just.trim().is_empty() {
                    return Err("invalid justification: expected a non-empty sentence".into());
                }
                // 全部三态词汇在此接受;非严格加宽(同级/降级)由加宽表
                // 检查统一拒绝(unknown 仅指真正未知的字符串)
                let target = match mode {
                    "read-only" => SandboxMode::ReadOnly,
                    "workspace-write" => SandboxMode::WorkspaceWrite,
                    "full-access" => SandboxMode::FullAccess,
                    other => {
                        return Err(format!(
                            "invalid escalation: unknown sandbox_permissions \"{other}\""
                        ));
                    }
                };
                Ok(Some((target, just.trim().to_string())))
            }
        }
    }

    /// 启用 PTY 模式(沙箱经 argv 包装;landlock-only 系统拒绝执行)
    pub fn with_pty(mut self) -> Self {
        self.pty = true;
        self
    }

    /// 设置软取消令牌(与会话令牌共享)
    pub fn with_cancel(mut self, token: CancelToken) -> Self {
        self.cancel = token;
        self
    }

    /// 注入后台任务注册表(启用 run_in_background;与 jobs 工具共享)
    pub fn with_jobs(mut self, registry: JobsRegistry) -> Self {
        self.jobs = Some(registry);
        self
    }

    /// 注入 job 结算通知口(发起会话 id;job 落定时投递通知)
    pub fn with_job_notify(
        mut self,
        port: std::sync::Arc<dyn SettlementNotificationPort>,
        session_id: impl Into<String>,
    ) -> Self {
        self.job_notify = Some((port, session_id.into()));
        self
    }

    /// 工具 schema(注册面;OpenAI wire 形状 `tools[]` 元素)。
    /// 工具名与描述随平台 shell 走(`bash` / `pwsh`)——模型看到的工具名
    /// 要与它实际要写的语法一致,否则会按 bash 语义写出 Windows 上跑不通
    /// 的命令。「结算会通知」承诺只在通知口在场时写进描述
    /// (行为承诺与接口一致)
    pub fn spec(&self) -> Value {
        let notices = self.job_notify.is_some();
        let bg_desc = if notices {
            "Run detached; returns a job id immediately. You are notified when the job settles (finishes, fails, or is stopped) — do not poll; use the jobs tool only to read more output or to stop it."
        } else {
            "Run detached; returns a job id immediately (manage via the jobs tool)"
        };
        let timeout_desc = if notices {
            "Wall-clock budget for this foreground command in milliseconds (default 120000, max 600000). When it elapses the command is moved to a background job — nothing is killed, the output keeps streaming to the job log, and you will be notified when it settles."
        } else {
            "Wall-clock budget for this foreground command in milliseconds (default 120000, max 600000). When it elapses the command is moved to a background job — nothing is killed, the output keeps streaming to the job log; manage it with the jobs tool."
        };
        json!({
            "type": "function",
            "function": {
                "name": shell::tool_name(),
                "description": shell::tool_description(),
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": shell::command_param_description() },
                        "description": {
                            "type": "string",
                            "description": "Clear, concise description of what this command does in active voice, 5-10 words (shown in the UI). Examples: \"ls\" → \"List files in current directory\"; \"git status\" → \"Show working tree status\"; \"npm install\" → \"Install package dependencies\"."
                        },
                        "run_in_background": { "type": "boolean", "description": bg_desc },
                        "timeout_ms": {
                            "type": "integer",
                            "description": timeout_desc
                        },
                        "sandbox_permissions": {
                            "type": "string",
                            "enum": ["workspace-write", "full-access"],
                            "description": "The wider sandbox mode this command needs. Only valid as a one-shot retry of a command the sandbox just denied; requires justification and user approval. Foreground commands only."
                        },
                        "justification": {
                            "type": "string",
                            "description": "Required with sandbox_permissions: one sentence for the user explaining why this exact command needs the wider access."
                        }
                    },
                    "required": ["command", "description"],
                },
            },
        })
    }

    /// PTY 路径:沙箱经 argv 包装;通道化增量读 + 前台预算三相
    /// (EOF / 取消 / 预算耗尽——耗尽与 pipes 同一裁决:移交或降级杀)。
    /// 取消即 killer 组信号杀进程
    async fn execute_pty(&mut self, command: &str, timeout_ms: u64) -> ToolOutput {
        let policy = self.resolve_policy();
        let (program, args) = match shell::shell_argv(command) {
            Ok(v) => v,
            Err(e) => return fail_output(format!("shell unavailable: {e}")),
        };
        let mut session = match spawn_pty(&program, &args, Some(&self.cwd), Some(&policy)) {
            Ok(s) => s,
            Err(e) => {
                return ToolOutput {
                    output: format!("pty spawn failed: {e}"),
                    success: false,
                    ..Default::default()
                };
            }
        };
        let (mut rx, _read_task) = match session.start_read_chunks() {
            Ok(v) => v,
            Err(e) => return fail_output(format!("pty read start failed: {e}")),
        };
        let mut out: Vec<u8> = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        enum Phase {
            Eof,
            Cancelled,
            Deadline,
        }
        let phase = loop {
            tokio::select! {
                chunk = rx.recv() => match chunk {
                    // 通道关闭(EOF/读错)即读尽
                    None => break Phase::Eof,
                    Some(bytes) => out.extend_from_slice(&bytes),
                },
                _ = self.cancel.cancelled() => break Phase::Cancelled,
                _ = tokio::time::sleep_until(deadline) => break Phase::Deadline,
            }
        };
        let settle_pty = |status: ExitStatus, output: String| {
            let (success, exit_code, signal) = settle(status);
            ToolOutput {
                output,
                success,
                view: Some(terminal_view(exit_code, signal, Some(&self.cwd))),
                ..Default::default()
            }
        };
        match phase {
            Phase::Cancelled => {
                session.kill();
                ToolOutput {
                    output: "cancelled".into(),
                    success: false,
                    ..Default::default()
                }
            }
            Phase::Eof => {
                let output = liuma_sandbox::text::decode_output(&out).trim().to_string();
                let status = session.wait().await.unwrap_or(ExitStatus {
                    code: None,
                    signal: None,
                });
                settle_pty(status, output)
            }
            Phase::Deadline => {
                // 裁决序与 pipes 一致:取消优先 → 已退出排干落定 → 移交/降级
                if self.cancel.is_cancelled() {
                    session.kill();
                    return ToolOutput {
                        output: "cancelled".into(),
                        success: false,
                        ..Default::default()
                    };
                }
                match session.try_wait() {
                    Ok(Some(_)) => {
                        while let Some(bytes) = rx.recv().await {
                            out.extend_from_slice(&bytes);
                        }
                        let output = liuma_sandbox::text::decode_output(&out).trim().to_string();
                        let status = session.wait().await.unwrap_or(ExitStatus {
                            code: None,
                            signal: None,
                        });
                        settle_pty(status, output)
                    }
                    _ => match self.jobs.clone() {
                        Some(registry) => {
                            self.handover_pty_to_background(
                                registry, session, rx, out, command, timeout_ms,
                            )
                            .await
                        }
                        None => {
                            session.kill();
                            let mut text =
                                liuma_sandbox::text::decode_output(&out).trim().to_string();
                            text.push_str(&format!(
                                "\n[timed out after {timeout_ms}ms — killed; this assembly has no background jobs, so the command could not continue]"
                            ));
                            ToolOutput {
                                output: text,
                                success: false,
                                ..Default::default()
                            }
                        }
                    },
                }
            }
        }
    }

    /// 后台执行:沙箱 spawn + 注册表登记 + watcher 任务(流式落盘、
    /// 状态收尾);立即返回 job id。后台任务不随父取消终止(它是
    /// 「后台」的全部意义),由 jobs 工具显式 stop。
    async fn execute_background(&mut self, command: &str) -> ToolOutput {
        let Some(registry) = self.jobs.clone() else {
            return ToolOutput {
                output: "background jobs not enabled in this assembly".into(),
                success: false,
                ..Default::default()
            };
        };
        let opts = SpawnOptions {
            cwd: Some(self.cwd.clone()),
            env: Default::default(),
            sandbox: Some(self.resolve_policy()),
            stdin: None,
        };
        let (program, args) = match shell::shell_argv(command) {
            Ok(v) => v,
            Err(e) => return fail_output(format!("shell unavailable: {e}")),
        };
        let mut child = match spawn_child(&program, &args, &opts).await {
            Ok(child) => child,
            Err(e) => return fail_output(format!("spawn failed: {e}")),
        };
        let stdout = child.take_stdout();
        // 分离终止宽限:SIGTERM 后 1s 即 SIGKILL(不等前台 grace)
        let killer = JobKiller::Group(child.group_killer());
        let (id, log_path) = match start_job(
            &registry,
            &self.cwd,
            command,
            killer,
            Duration::from_secs(1),
        )
        .await
        {
            Ok(v) => v,
            // 登记失败即无收尾方:杀掉刚 spawn 的进程再报错,不留孤儿
            Err(msg) => {
                let _ = child.kill_with_grace(self.grace).await;
                return fail_output(msg);
            }
        };
        tokio::spawn(run_job_watcher(
            child,
            stdout,
            Vec::new(),
            JobWatch {
                registry,
                id,
                log_path: log_path.clone(),
                command: command.to_string(),
                notify: self.job_notify.clone(),
            },
        ));
        let settle_note = if self.job_notify.is_some() {
            "; you will be notified when it settles"
        } else {
            ""
        };
        ToolOutput {
            output: format!(
                "started job {id} (log: {}; jobs tool: list/read/stop{settle_note})",
                log_path.display()
            ),
            success: true,
            ..Default::default()
        }
    }

    /// timeout_ms 解析:缺省 DEFAULT;整数 1..=MAX(文案逐字固定)
    fn parse_timeout_ms(arguments: &Value) -> Result<u64, String> {
        const MSG: &str = "invalid timeout_ms: expected an integer between 1 and 600000; use run_in_background for longer commands";
        match &arguments["timeout_ms"] {
            Value::Null => Ok(BASH_TIMEOUT_DEFAULT_MS),
            v => match v.as_u64() {
                Some(ms) if (1..=BASH_TIMEOUT_MAX_MS).contains(&ms) => Ok(ms),
                _ => Err(MSG.into()),
            },
        }
    }

    /// 前台落定:沙箱分类(denialSignatures + runnerFailureRules 语义)+
    /// 渲染意图。runner 失败(命令从未执行)/ 拒绝(内核拦截)/ 常规退出
    /// (success=true,退出码是数据、不是失败)
    async fn settle_foreground(
        &self,
        mut child: liuma_sandbox::Child,
        output: String,
        policy: &SandboxPolicy,
    ) -> ToolOutput {
        let (success, exit_code, signal, rendered) = match child.wait_classified().await {
            ExitClass::Ran(status) => {
                let (s, c, sig) = settle(status);
                (s, c, sig, output)
            }
            ExitClass::RunnerFailed { code, detail } => (
                false,
                code,
                None,
                format!("sandbox runner 失败(命令未执行):\n{detail}"),
            ),
            ExitClass::Denied { status, .. } => {
                // 拒绝标记 + stderr 原文 + 升级提示链(审批口在场且存在
                // 更宽档位时才提示——无审批服务时不撒谎)
                let stderr = child.stderr_text().await;
                let mut text = format!("{}\n{}", denial_marker(policy.mode), stderr.trim());
                if self.approval.is_some() && !Self::wider_modes(policy.mode).is_empty() {
                    text.push('\n');
                    text.push_str(ESCALATION_HINT);
                }
                (false, status.code, None, text)
            }
        };
        ToolOutput {
            output: rendered,
            success,
            view: Some(terminal_view(exit_code, signal, Some(&self.cwd))),
            ..Default::default()
        }
    }

    /// 预算耗尽裁决:①取消优先(与 deadline 同刻到达不得误判成超时)
    /// ②进程已自发退出 → 排干余量走正常落定(完成的命令不报「转后台」)
    /// ③真超时 → 移交后台;jobs 未装配 → 降级杀 + 部分输出。
    /// stdout 已自发关闭但进程存活的 daemon 型命令落 ③:移交正确
    async fn adjudicate_deadline(
        &self,
        child: liuma_sandbox::Child,
        mut stdout: Option<tokio::process::ChildStdout>,
        out: Vec<u8>,
        policy: &SandboxPolicy,
        command: &str,
        timeout_ms: u64,
    ) -> ToolOutput {
        let mut child = child;
        if self.cancel.is_cancelled() {
            let _ = child.kill_with_grace(self.grace).await;
            return ToolOutput {
                output: "cancelled".into(),
                success: false,
                ..Default::default()
            };
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                // 已退出:排干 stdout 余量(EOF 即到),完整输出正常落定
                let mut out = out;
                if let Some(pipe) = stdout.as_mut() {
                    use tokio::io::AsyncReadExt;
                    let mut rest = Vec::new();
                    let _ = pipe.read_to_end(&mut rest).await;
                    out.extend_from_slice(&rest);
                }
                let output = liuma_sandbox::text::decode_output(&out).trim().to_string();
                self.settle_foreground(child, output, policy).await
            }
            // None = 还在跑(真超时);Err = 探测失败,按存活性保守处理
            _ => match self.jobs.clone() {
                Some(registry) => {
                    self.handover_to_background(registry, child, stdout, out, command, timeout_ms)
                        .await
                }
                None => {
                    // 降级:无后台装配无法转走,杀进程并交出已积累输出
                    let _ = child.kill_with_grace(self.grace).await;
                    let stderr = child.stderr_text().await;
                    let mut text = liuma_sandbox::text::decode_output(&out).trim().to_string();
                    if !stderr.trim().is_empty() {
                        text.push('\n');
                        text.push_str(stderr.trim());
                    }
                    text.push_str(&format!(
                        "\n[timed out after {timeout_ms}ms — killed; this assembly has no background jobs, so the command could not continue]"
                    ));
                    ToolOutput {
                        output: text,
                        success: false,
                        view: Some(terminal_view(None, None, Some(&self.cwd))),
                        ..Default::default()
                    }
                }
            },
        }
    }

    /// 前台超时移交:pipes 形态。登记 job + 流式 watcher(携带已读
    /// 前缀),进程原样继续(沙箱/审批上下文已在 spawn 时生效,无需迁移)
    async fn handover_to_background(
        &self,
        registry: JobsRegistry,
        child: liuma_sandbox::Child,
        stdout: Option<tokio::process::ChildStdout>,
        prefix: Vec<u8>,
        command: &str,
        timeout_ms: u64,
    ) -> ToolOutput {
        let mut child = child;
        let killer = JobKiller::Group(child.group_killer());
        // 分离终止宽限与 run_in_background 同参(SIGTERM 后 1s 即 SIGKILL)
        let started = start_job(
            &registry,
            &self.cwd,
            command,
            killer,
            Duration::from_secs(1),
        )
        .await;
        match started {
            Ok((id, log_path)) => {
                let notify = self.job_notify.clone();
                let settle_note = if notify.is_some() {
                    "the command keeps running and you will be notified when it settles (jobs tool: read/list/stop)"
                } else {
                    "the command keeps running; manage it with the jobs tool: read/list/stop"
                };
                tokio::spawn(run_job_watcher(
                    child,
                    stdout,
                    prefix,
                    JobWatch {
                        registry,
                        id,
                        log_path: log_path.clone(),
                        command: command.to_string(),
                        notify,
                    },
                ));
                ToolOutput {
                    output: format!(
                        "moved to background: job {id} after {timeout_ms}ms ({settle_note})\nlog: {}",
                        log_path.display()
                    ),
                    success: true,
                    ..Default::default()
                }
            }
            Err(msg) => {
                // 登记失败即无收尾方:杀掉进程再报错,不留孤儿
                let _ = child.kill_with_grace(self.grace).await;
                fail_output(msg)
            }
        }
    }

    /// 前台超时移交:PTY 形态。读通道与已读前缀随会话移交 watcher;
    /// 登记失败路径靠 master drop 的 SIGHUP 收口(PTY 语义)
    async fn handover_pty_to_background(
        &self,
        registry: JobsRegistry,
        session: liuma_sandbox::pty::PtySession,
        rx: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        prefix: Vec<u8>,
        command: &str,
        timeout_ms: u64,
    ) -> ToolOutput {
        let Some(killer) = session.killer() else {
            // 无分离句柄则无从 stop:降级杀(master drop 送 SIGHUP)
            drop(session);
            return ToolOutput {
                output: format!(
                    "[timed out after {timeout_ms}ms — killed; this pty session has no termination handle]"
                ),
                success: false,
                ..Default::default()
            };
        };
        let killer = JobKiller::Pty(killer);
        // PTY 无宽限语义,grace 仅占位(killer 即杀)
        let started = start_job(&registry, &self.cwd, command, killer, Duration::ZERO).await;
        match started {
            Ok((id, log_path)) => {
                let notify = self.job_notify.clone();
                let settle_note = if notify.is_some() {
                    "the command keeps running and you will be notified when it settles (jobs tool: read/list/stop)"
                } else {
                    "the command keeps running; manage it with the jobs tool: read/list/stop"
                };
                tokio::spawn(run_pty_job_watcher(
                    session,
                    rx,
                    prefix,
                    JobWatch {
                        registry,
                        id,
                        log_path: log_path.clone(),
                        command: command.to_string(),
                        notify,
                    },
                ));
                ToolOutput {
                    output: format!(
                        "moved to background: job {id} after {timeout_ms}ms ({settle_note})\nlog: {}",
                        log_path.display()
                    ),
                    success: true,
                    ..Default::default()
                }
            }
            Err(msg) => {
                drop(session);
                fail_output(msg)
            }
        }
    }
}

/// 后台 job 登记:目录就绪 + 注册表 push + 变更通知。
/// `run_in_background` 启动与前台超时移交共用
async fn start_job(
    registry: &JobsRegistry,
    cwd: &std::path::Path,
    command: &str,
    killer: JobKiller,
    grace: Duration,
) -> Result<(u64, PathBuf), String> {
    // 目录先就绪:id 分配要播种磁盘现存日志(跨重启不复用旧号,
    // 否则截断写会销毁旧日志)
    let jobs_dir = cwd.join(".liuma/jobs");
    tokio::fs::create_dir_all(&jobs_dir)
        .await
        .map_err(|e| format!("jobs dir create failed: {e}"))?;
    let floor = max_disk_job_id(&jobs_dir);
    let id = next_job_id(registry, floor);
    let log_path = jobs_dir.join(format!("{id}.log"));
    registry
        .lock()
        .map(|mut r| {
            r.push(JobRecord {
                id,
                command: command.to_string(),
                status: "running".into(),
                log_path: log_path.clone(),
                killer: Some(killer),
                grace,
                started_at: jobs::now_ms(),
                ended_at: None,
            })
        })
        .map_err(|_| "jobs registry unavailable".to_string())?;
    registry.changed();
    Ok((id, log_path))
}

/// 尾窗推进(有界;结算通知 tail 的来源,不随日志大小增长)
fn advance_tail(tail: &mut Vec<u8>, chunk: &[u8]) {
    const TAIL_LIMIT: usize = 2048;
    tail.extend_from_slice(chunk);
    let len = tail.len();
    if len > TAIL_LIMIT {
        tail.drain(..len - TAIL_LIMIT);
    }
}

/// watcher 物料(两形态共用):登记簿寻址 + 结算通知口
struct JobWatch {
    registry: JobsRegistry,
    id: u64,
    log_path: PathBuf,
    command: String,
    notify: Option<JobNotify>,
}

/// 后台 watcher(pipes 形态):stdout 逐块流式落盘(任务运行中
/// `jobs read` 即有内容)→ 退出 → stderr 一并落盘(沙箱拒绝/脚本
/// 错误文本不进 log 则排查无据)→ 状态收尾 + 结算通知。
/// stop 先置位 stopped、watcher 只改 running 的竞争机制保持(kill 后
/// 也经此路径)。落盘失败不阻断收尾(创建失败 = 全程无文件,read
/// 如实报读不了)
async fn run_job_watcher(
    mut child: liuma_sandbox::Child,
    mut stdout: Option<tokio::process::ChildStdout>,
    prefix: Vec<u8>,
    watch: JobWatch,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut log = tokio::fs::File::create(&watch.log_path).await.ok();
    let mut tail: Vec<u8> = Vec::new();
    if !prefix.is_empty() {
        if let Some(f) = log.as_mut() {
            let _ = f.write_all(&prefix).await;
        }
        advance_tail(&mut tail, &prefix);
    }
    if let Some(stdout) = stdout.as_mut() {
        let mut chunk = [0u8; 8192];
        loop {
            match stdout.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    if let Some(f) = log.as_mut() {
                        let _ = f.write_all(&chunk[..n]).await;
                    }
                    advance_tail(&mut tail, &chunk[..n]);
                }
                Err(_) => break,
            }
        }
    }
    let status = child.wait().await.ok();
    let stderr = child.stderr_text().await;
    if let Some(f) = log.as_mut() {
        if !stderr.is_empty() {
            let _ = f.write_all(stderr.as_bytes()).await;
            advance_tail(&mut tail, stderr.as_bytes());
        }
        let _ = f.flush().await;
    }
    finish_job(&watch.registry, watch.id, status.map(|s| s.success()));
    notify_job_settled(
        &watch.registry,
        watch.id,
        &watch.command,
        status,
        &tail,
        watch.notify.as_ref(),
    )
    .await;
}

/// 后台 watcher(PTY 形态):通道化增量读逐块落盘 → 会话退出 →
/// 收尾 + 结算通知。stderr 与 stdout 合流在 master(PTY 语义),
/// 无单独并入步
async fn run_pty_job_watcher(
    mut session: liuma_sandbox::pty::PtySession,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    prefix: Vec<u8>,
    watch: JobWatch,
) {
    use tokio::io::AsyncWriteExt;
    let mut log = tokio::fs::File::create(&watch.log_path).await.ok();
    let mut tail: Vec<u8> = Vec::new();
    if !prefix.is_empty() {
        if let Some(f) = log.as_mut() {
            let _ = f.write_all(&prefix).await;
        }
        advance_tail(&mut tail, &prefix);
    }
    while let Some(chunk) = rx.recv().await {
        if let Some(f) = log.as_mut() {
            let _ = f.write_all(&chunk).await;
        }
        advance_tail(&mut tail, &chunk);
    }
    if let Some(f) = log.as_mut() {
        let _ = f.flush().await;
    }
    let status = session.wait().await.ok();
    finish_job(&watch.registry, watch.id, status.map(|s| s.success()));
    notify_job_settled(
        &watch.registry,
        watch.id,
        &watch.command,
        status,
        &tail,
        watch.notify.as_ref(),
    )
    .await;
}

/// job 收尾(两形态 watcher 共用):状态落定 + 变更通知。
/// stop 抢先置位 stopped、收尾只改 running 的竞争机制保持;
/// ok = None 表示退出状态不可知(按失败落定)
fn finish_job(registry: &JobsRegistry, id: u64, ok: Option<bool>) {
    let status_label = if ok == Some(true) { "done" } else { "failed" };
    if let Ok(mut r) = registry.lock()
        && let Some(job) = r.iter_mut().find(|j| j.id == id)
    {
        if job.status == "running" {
            job.status = status_label.into();
        }
        job.killer = None;
        job.ended_at = Some(jobs::now_ms());
    }
    registry.changed();
}

/// 结算通知:按注册表终态措辞(stop 先置位的 stopped 与自然收尾的
/// done/failed 都投),携带退出码/信号与输出尾部,投给发起会话。
/// 自然退出与 stop 置位的竞争按终态标签措辞(用户确实请求了 stop)。
/// 通知口缺席 = 此装配不投(静默,不是错误)
async fn notify_job_settled(
    registry: &JobsRegistry,
    id: u64,
    command: &str,
    status: Option<ExitStatus>,
    tail: &[u8],
    notify: Option<&JobNotify>,
) {
    let Some((port, session)) = notify else {
        return;
    };
    let label = registry
        .lock()
        .ok()
        .and_then(|r| r.iter().find(|j| j.id == id).map(|j| j.status.clone()))
        .unwrap_or_else(|| "failed".into());
    let (exit_code, signal) = match status {
        Some(s) => (s.code, s.signal.map(signal_name)),
        None => (None, None),
    };
    let tail_text = liuma_sandbox::text::decode_output(tail).trim().to_string();
    let (text, source) = job_settlement_notice(
        id,
        command,
        &label,
        exit_code,
        signal.as_deref(),
        &tail_text,
    );
    port.notify(session, text, source).await;
}

impl ToolPort for BashTool {
    /// 工具声明(engine 注入请求 header 供模型选择)
    fn specs(&self) -> Vec<Value> {
        vec![self.spec()]
    }

    async fn execute(&mut self, call: &ToolCallRequest) -> ToolOutput {
        // 名字守卫同样随平台方言走:模型面工具名是 `bash` / `pwsh`,
        // 写死任一个都会在另一平台上把合法调用判成「未知工具」
        if call.name != shell::tool_name() {
            return ToolOutput {
                output: format!("unknown tool: {}", call.name),
                success: false,
                ..Default::default()
            };
        }
        // OpenAI 兼容 wire 上 arguments 是 JSON 编码字符串;本地夹具是对象。
        // 两种形态都接受(字符串解析失败按空对象处理,由下方必填检查拒绝)
        let arguments: Value = if let Some(s) = call.arguments.as_str() {
            serde_json::from_str(s).unwrap_or(json!({}))
        } else {
            call.arguments.clone()
        };
        let Some(command) = arguments["command"].as_str() else {
            return ToolOutput {
                output: "bash tool requires arguments.command (string)".into(),
                success: false,
                ..Default::default()
            };
        };
        // description 必填(给用户看的一句意图说明,进 UI
        // 摘要;非空校验错误消息固定文案)
        let description_empty = arguments["description"]
            .as_str()
            .map(str::trim)
            .map(str::is_empty)
            .unwrap_or(true);
        if description_empty {
            return ToolOutput {
                output: "invalid description: expected a non-empty string".into(),
                success: false,
                ..Default::default()
            };
        }
        // 升级参数解析(sandbox_permissions/justification 成对校验);
        // 仅前台——pty/后台带参显式拒绝(拒绝分类与提示链只在前台存在)
        let escalation = match Self::parse_escalation(&arguments) {
            Ok(e) => e,
            Err(msg) => {
                return ToolOutput {
                    output: msg,
                    success: false,
                    ..Default::default()
                };
            }
        };
        if let Some((_, _)) = &escalation
            && (arguments["run_in_background"].as_bool().unwrap_or(false) || self.pty)
        {
            return ToolOutput {
                output: "sandbox_permissions is only available for foreground commands".into(),
                success: false,
                ..Default::default()
            };
        }
        // 前台预算:timeout_ms(缺省 DEFAULT;文案逐字固定,越界拒绝
        // 而非钳制——静默改值会让模型误判自己的预算)
        let timeout_ms = match Self::parse_timeout_ms(&arguments) {
            Ok(ms) => ms,
            Err(msg) => {
                return ToolOutput {
                    output: msg,
                    success: false,
                    ..Default::default()
                };
            }
        };
        // 后台路径:spawn + 注册 + 立即返回 job id;
        // watcher 任务流式落盘并收尾状态(.liuma/jobs/<id>.log)。
        // 预算只约束前台等待,后台无此概念
        if arguments["run_in_background"].as_bool().unwrap_or(false) {
            if !arguments["timeout_ms"].is_null() {
                return ToolOutput {
                    output: "timeout_ms is only valid for foreground commands".into(),
                    success: false,
                    ..Default::default()
                };
            }
            return self.execute_background(command).await;
        }
        if self.pty {
            return self.execute_pty(command, timeout_ms).await;
        }
        let mut policy = self.resolve_policy();
        // 一次性升级闸门:严格加宽检查 → 审批口在场 → 问用户(批准先于
        // 执行,零执行失败即错误;固定次序与逐字文案)
        if let Some((target, justification)) = escalation {
            let current = policy.mode;
            if !Self::wider_modes(current).contains(&target) {
                return ToolOutput {
                    output: format!(
                        "sandbox escalation to \"{}\" is not strictly wider than this call's current \"{}\" mode",
                        mode_name(target),
                        mode_name(current)
                    ),
                    success: false,
                    ..Default::default()
                };
            }
            let Some(port) = &self.approval else {
                return ToolOutput {
                    output:
                        "sandbox escalation requires approval, but no approval service is composed"
                            .into(),
                    success: false,
                    ..Default::default()
                };
            };
            let req = EscalationRequest {
                tool_name: shell::tool_name().into(),
                call_id: None,
                command: command.to_string(),
                target_mode: target,
                justification,
            };
            let outcome = tokio::select! {
                out = port.request(req) => out,
                // 取消与审批竞态:取消即不再等待(宿主侧 drop 守卫清
                // pending 并落 decided(cancelled))
                _ = self.cancel.cancelled() => ApprovalOutcome::Cancelled,
            };
            match outcome {
                ApprovalOutcome::AllowedOnce => policy.mode = target,
                ApprovalOutcome::Rejected => {
                    return ToolOutput {
                        output: format!(
                            "the user rejected escalating this command to \"{}\"; \
                             treat the denial as final — do not retry the same command, \
                             adjust the approach instead",
                            mode_name(target)
                        ),
                        success: false,
                        ..Default::default()
                    };
                }
                ApprovalOutcome::Cancelled => {
                    return ToolOutput {
                        output: format!(
                            "approval for escalating to \"{}\" was cancelled",
                            mode_name(target)
                        ),
                        success: false,
                        ..Default::default()
                    };
                }
                ApprovalOutcome::Unavailable => {
                    return ToolOutput {
                        output: "sandbox escalation requires approval, but no approval channel is available".into(),
                        success: false,
                        ..Default::default()
                    };
                }
            }
        }
        let opts = SpawnOptions {
            cwd: Some(self.cwd.clone()),
            env: Default::default(),
            sandbox: Some(policy.clone()),
            stdin: None,
        };
        let (program, args) = match shell::shell_argv(command) {
            Ok(v) => v,
            Err(e) => return fail_output(format!("shell unavailable: {e}")),
        };
        // spawn 失败(含 fail-closed 沙箱拒绝)即工具失败,不中断 loop
        let mut child = match spawn_child(&program, &args, &opts).await {
            Ok(child) => child,
            Err(e) => return fail_output(format!("spawn failed: {e}")),
        };
        // 增量读循环:out 与管道属主外置——超时/取消时天然持有部分输出。
        // 逐块单次 read(管道 read 取消安全,读了多少算多少)
        let mut stdout = child.take_stdout();
        let mut out: Vec<u8> = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        /// 读循环收束相(EOF / 取消 / 预算耗尽)
        enum Phase {
            Eof,
            Cancelled,
            Deadline,
        }
        let phase = loop {
            tokio::select! {
                read = async {
                    use tokio::io::AsyncReadExt;
                    let stdout = stdout.as_mut().expect("select 前置条件保证在场");
                    let mut buf = vec![0u8; 8192];
                    let n = stdout.read(&mut buf).await;
                    (buf, n)
                }, if stdout.is_some() => {
                    let (buf, n) = read;
                    match n {
                        Ok(0) | Err(_) => break Phase::Eof,
                        Ok(n) => out.extend_from_slice(&buf[..n]),
                    }
                }
                // 软取消:杀子进程(SIGTERM→grace→SIGKILL)后温和返回
                _ = self.cancel.cancelled() => break Phase::Cancelled,
                _ = tokio::time::sleep_until(deadline) => break Phase::Deadline,
            }
        };
        match phase {
            Phase::Cancelled => {
                // 已取消,杀失败无补救手段(进程可能已退出)
                let _ = child.kill_with_grace(self.grace).await;
                ToolOutput {
                    output: "cancelled".into(),
                    success: false,
                    ..Default::default()
                }
            }
            Phase::Eof => {
                // stdout 读尽。进程可能仍挂着(关闭 stdout ≠ 退出),
                // 落定等待受同一 deadline 守护,到点走同一裁决
                let exited = tokio::select! {
                    status = child.wait() => Some(status.ok()),
                    _ = tokio::time::sleep_until(deadline) => None,
                };
                match exited {
                    Some(_) => {
                        let output = liuma_sandbox::text::decode_output(&out).trim().to_string();
                        self.settle_foreground(child, output, &policy).await
                    }
                    None => {
                        self.adjudicate_deadline(child, stdout, out, &policy, command, timeout_ms)
                            .await
                    }
                }
            }
            Phase::Deadline => {
                self.adjudicate_deadline(child, stdout, out, &policy, command, timeout_ms)
                    .await
            }
        }
    }
}

/// 落定退出状态 → Terminal 渲染意图(前台/PTY 共用;后台启动与
/// 执行错误无退出状态,不产视图走通用卡)
fn terminal_view(
    exit_code: Option<i32>,
    signal: Option<String>,
    cwd: Option<&std::path::Path>,
) -> ToolView {
    ToolView::Terminal {
        exit_code,
        signal,
        cwd: cwd.map(|c| c.display().to_string()),
    }
}

/// 工具失败输出(前台/后台/PTY 共用的失败形状:文本 + success=false)
fn fail_output(msg: String) -> ToolOutput {
    ToolOutput {
        output: msg,
        success: false,
        ..Default::default()
    }
}

/// 模式名(denial marker / 升级错误文案;与 liuma-core `permission.rs`
/// 字符串一致)
fn mode_name(mode: SandboxMode) -> &'static str {
    match mode {
        SandboxMode::ReadOnly => "read-only",
        SandboxMode::WorkspaceWrite => "workspace-write",
        SandboxMode::FullAccess => "full-access",
    }
}

/// 拒绝标记(`sandboxDenialMarker` 模式插值),
/// 模型据此识别「沙箱拦截而非命令逻辑错误」
fn denial_marker(mode: SandboxMode) -> String {
    format!(
        "[sandbox: file access denied under {} mode]",
        mode_name(mode)
    )
}

/// 退出状态 → (success, exit_code, signal):
/// - 有退出码且**非负**:落定成功,码作为数据透出;
/// - 有退出码但为负:失败,码照实透出(见下);
/// - 信号终止:失败 + 信号名;
/// - 状态不可知:失败,无线索。
///
/// 负码是 Windows 的异常终止:进程以 NTSTATUS 收场时 `ExitStatus::code()`
/// 把它读回成负 i32(`0xC0000005` → `-1073741819`),与 Unix 的被信号杀死
/// 同类——命令没跑完,不是「有码即成功」(PowerShell 侧 `$LASTEXITCODE`
/// 显示的也是这个有符号视图)。Unix 退出码恒为 0..=255,该分支不可达,
/// 行为不变。
fn settle(status: ExitStatus) -> (bool, Option<i32>, Option<String>) {
    match (status.code, status.signal) {
        (Some(code), _) => (code >= 0, Some(code), None),
        (None, Some(sig)) => (false, None, Some(signal_name(sig))),
        (None, None) => (false, None, None),
    }
}

/// 信号编号 → 名(1–15 在 macOS/Linux 一致;其余回数值名,无损)
fn signal_name(sig: i32) -> String {
    let name = match sig {
        1 => "SIGHUP",
        2 => "SIGINT",
        3 => "SIGQUIT",
        4 => "SIGILL",
        5 => "SIGTRAP",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        9 => "SIGKILL",
        10 => "SIGUSR1",
        11 => "SIGSEGV",
        12 => "SIGUSR2",
        13 => "SIGPIPE",
        14 => "SIGALRM",
        15 => "SIGTERM",
        _ => return format!("SIG{sig}"),
    };
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 拒绝标记快照(模式名与
    /// liuma-core `permission.rs` 字符串一致,模型据此识别沙箱拦截)
    #[test]
    fn denial_marker_matches_mode_strings() {
        assert_eq!(
            denial_marker(SandboxMode::ReadOnly),
            "[sandbox: file access denied under read-only mode]"
        );
        assert_eq!(
            denial_marker(SandboxMode::WorkspaceWrite),
            "[sandbox: file access denied under workspace-write mode]"
        );
        assert_eq!(
            denial_marker(SandboxMode::FullAccess),
            "[sandbox: file access denied under full-access mode]"
        );
    }

    /// 落定快照:非负码即成功、负码(Windows 异常终止)失败、
    /// 信号终止失败、状态不可知失败
    #[test]
    fn settle_snapshot() {
        let s = |code, signal| ExitStatus { code, signal };
        assert_eq!(settle(s(Some(0), None)), (true, Some(0), None));
        assert_eq!(settle(s(Some(2), None)), (true, Some(2), None));
        // 回归锚:0xC0000005(访问违例)经 Windows 读回是负码,不得报成功
        assert_eq!(
            settle(s(Some(-1073741819), None)),
            (false, Some(-1073741819), None)
        );
        assert_eq!(
            settle(s(None, Some(15))),
            (false, None, Some("SIGTERM".into()))
        );
        assert_eq!(settle(s(None, None)), (false, None, None));
    }

    /// description 必填:缺参/空白拒绝,错误消息固定文案;
    /// 合法调用不受影响
    #[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见 tool_loop.rs 文件头)
    #[tokio::test]
    async fn bash_requires_non_empty_description() {
        let dir = std::env::temp_dir().join(format!("liuma-bash-desc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut tool = BashTool::new(&dir);
        for args in [
            json!({ "command": "echo hi" }),
            json!({ "command": "echo hi", "description": "   " }),
        ] {
            let out = ToolPort::execute(
                &mut tool,
                &ToolCallRequest {
                    name: shell::tool_name().into(),
                    arguments: args,
                },
            )
            .await;
            assert!(!out.success);
            assert_eq!(
                out.output,
                "invalid description: expected a non-empty string"
            );
        }
        let ok = ToolPort::execute(
            &mut tool,
            &ToolCallRequest {
                name: shell::tool_name().into(),
                arguments: json!({ "command": "echo hi", "description": "Echo greeting" }),
            },
        )
        .await;
        assert!(ok.success, "{}", ok.output);
    }

    /// 动态模式源:execute 时实时解析——read-only 下写 workspace 根被
    /// 内核拦(拒绝标记带模式名),翻转 workspace-write 后同一命令放行。
    /// 权限切换落档即生效的执行面基础
    #[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见 tool_loop.rs 文件头)
    #[tokio::test]
    async fn bash_mode_source_resolved_per_execute() {
        let dir = std::env::temp_dir().join(format!("liuma-bash-mode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mode = std::sync::Arc::new(std::sync::Mutex::new(SandboxMode::ReadOnly));
        let mode_for_tool = std::sync::Arc::clone(&mode);
        let mut tool = BashTool::new(&dir)
            .with_mode_source(std::sync::Arc::new(move || *mode_for_tool.lock().unwrap()));
        let call = |cmd: String| ToolCallRequest {
            name: shell::tool_name().into(),
            arguments: json!({ "command": cmd, "description": "Probe write" }),
        };
        let denied =
            ToolPort::execute(&mut tool, &call(format!("touch {}/f", dir.display()))).await;
        assert!(!denied.success, "read-only 写应被拦:{:?}", denied.output);
        assert!(
            denied.output.contains("read-only mode"),
            "拒绝标记应带模式名:{:?}",
            denied.output
        );
        *mode.lock().unwrap() = SandboxMode::WorkspaceWrite;
        let ok = ToolPort::execute(&mut tool, &call(format!("touch {}/f", dir.display()))).await;
        assert!(ok.success, "workspace-write 写应放行:{:?}", ok.output);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 新构建路径:workspace-write 根推导(cwd + 平台临时区),而非裸 cwd
    #[test]
    fn bash_new_derives_workspace_write_policy() {
        let tool = BashTool::new("/tmp/example-ws");
        assert_eq!(tool.policy.mode, SandboxMode::WorkspaceWrite);
        assert!(
            tool.policy
                .writable_roots()
                .iter()
                .any(|r| r.ends_with("example-ws"))
        );
        // /tmp 与 temp_dir 在 macOS 上都是符号链接(→ /private/*),
        // 按 roots 推导同款 canonicalize 后比较
        let tmp =
            std::fs::canonicalize(std::env::temp_dir()).unwrap_or_else(|_| std::env::temp_dir());
        assert!(tool.policy.writable_roots().iter().any(|r| r == &tmp));
        assert_eq!(tool.cwd, PathBuf::from("/tmp/example-ws"));
    }

    /// 一次性升级闸门:port 拒绝 → 逐字拒绝文本且零执行;port 批准 →
    /// 本次以宽策略执行(allow-once);下一次无参执行回到会话模式
    /// (被拒/批准都不落会话态)
    #[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见 tool_loop.rs 文件头)
    #[tokio::test]
    async fn bash_escalation_gate_and_one_shot() {
        use std::sync::atomic::{AtomicBool, Ordering};

        struct MockApproval {
            outcome: std::sync::Arc<std::sync::Mutex<ApprovalOutcome>>,
            consulted: std::sync::Arc<AtomicBool>,
        }
        impl ApprovalPort for MockApproval {
            fn request(
                &self,
                _req: EscalationRequest,
            ) -> Pin<Box<dyn Future<Output = ApprovalOutcome> + Send>> {
                let outcome = *self.outcome.lock().unwrap();
                let consulted = std::sync::Arc::clone(&self.consulted);
                Box::pin(async move {
                    consulted.store(true, Ordering::Relaxed);
                    outcome
                })
            }
        }

        let dir = std::env::temp_dir().join(format!("liuma-bash-esc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mode = std::sync::Arc::new(std::sync::Mutex::new(SandboxMode::WorkspaceWrite));
        let mode_for_tool = std::sync::Arc::clone(&mode);
        let outcome = std::sync::Arc::new(std::sync::Mutex::new(ApprovalOutcome::Rejected));
        let consulted = std::sync::Arc::new(AtomicBool::new(false));
        let mut tool = BashTool::new(&dir)
            .with_mode_source(std::sync::Arc::new(move || *mode_for_tool.lock().unwrap()))
            .with_approval_port(std::sync::Arc::new(MockApproval {
                outcome: std::sync::Arc::clone(&outcome),
                consulted: std::sync::Arc::clone(&consulted),
            }));
        let esc_call = |cmd: String| ToolCallRequest {
            name: shell::tool_name().into(),
            arguments: json!({
                "command": cmd,
                "description": "Escalate probe",
                "sandbox_permissions": "full-access",
                "justification": "命令需要写工作区外的用户目录",
            }),
        };
        let probe = |n: &str| format!("touch ~/liuma-esc-probe-{n}-{}", std::process::id());

        // ① 拒绝:逐字文本 + 零执行(文件不存在)
        let rejected = ToolPort::execute(&mut tool, &esc_call(probe("rejected"))).await;
        assert!(!rejected.success);
        assert!(
            rejected
                .output
                .contains("the user rejected escalating this command to \"full-access\""),
            "{:?}",
            rejected.output
        );
        // 拒绝文本带行为教学(拒绝即终局,勿原样重试)
        assert!(
            rejected
                .output
                .contains("do not retry the same command, adjust the approach instead"),
            "{:?}",
            rejected.output
        );
        let home = std::env::var("HOME").unwrap();
        assert!(
            !std::path::Path::new(&format!(
                "{home}/liuma-esc-probe-rejected-{}",
                std::process::id()
            ))
            .exists(),
            "被拒命令不得落盘"
        );

        // ② 批准(allow-once):本次以 full-access 执行,命令生效
        *outcome.lock().unwrap() = ApprovalOutcome::AllowedOnce;
        let ok = ToolPort::execute(&mut tool, &esc_call(probe("allowed"))).await;
        assert!(ok.success, "批准后应以宽策略执行:{:?}", ok.output);
        assert!(
            std::path::Path::new(&format!(
                "{home}/liuma-esc-probe-allowed-{}",
                std::process::id()
            ))
            .exists(),
            "宽策略写应落盘"
        );

        // ③ 下一次无参执行回到会话模式:home 写被拦(只盖本次的语义)
        consulted.store(false, Ordering::Relaxed);
        let plain_call = ToolCallRequest {
            name: shell::tool_name().into(),
            arguments: json!({ "command": probe("plain"), "description": "Plain probe" }),
        };
        let back = ToolPort::execute(&mut tool, &plain_call).await;
        assert!(!back.success, "无参执行应回到会话模式(被拦)");
        assert!(
            back.output.contains("workspace-write mode"),
            "{:?}",
            back.output
        );
        assert!(!consulted.load(Ordering::Relaxed), "无参执行不得咨询审批口");

        // ④ 非加宽请求(同级):从不问人,逐字拒绝
        *mode.lock().unwrap() = SandboxMode::ReadOnly;
        let narrow = ToolCallRequest {
            name: shell::tool_name().into(),
            arguments: json!({
                "command": probe("narrow"),
                "description": "Narrow probe",
                "sandbox_permissions": "read-only",
                "justification": "试图原地重复",
            }),
        };
        let out = ToolPort::execute(&mut tool, &narrow).await;
        assert!(
            out.output
                .contains("is not strictly wider than this call's current \"read-only\" mode"),
            "{:?}",
            out.output
        );
        assert!(!consulted.load(Ordering::Relaxed), "非加宽请求从不问人");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(format!(
            "{home}/liuma-esc-probe-allowed-{}",
            std::process::id()
        ));
    }

    /// 校验逐字:两参不成对 / justification 空 / 未知档位——错误文案固定,
    /// 且零执行、不问审批口
    #[tokio::test]
    async fn bash_escalation_validation_verbatim() {
        let dir = std::env::temp_dir().join(format!("liuma-bash-escv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut tool = BashTool::new(&dir);
        let cases: Vec<(Value, &str)> = vec![
            (
                json!({ "command": "echo hi", "description": "D", "sandbox_permissions": "full-access" }),
                "invalid escalation: sandbox_permissions requires a justification",
            ),
            (
                json!({ "command": "echo hi", "description": "D", "justification": "想让命令更自由" }),
                "invalid escalation: justification is only valid together with sandbox_permissions",
            ),
            (
                json!({ "command": "echo hi", "description": "D", "sandbox_permissions": "full-access", "justification": "   " }),
                "invalid justification: expected a non-empty sentence",
            ),
            (
                json!({ "command": "echo hi", "description": "D", "sandbox_permissions": "danger-full-access", "justification": "旧词" }),
                "invalid escalation: unknown sandbox_permissions \"danger-full-access\"",
            ),
        ];
        for (args, expect) in cases {
            let out = ToolPort::execute(
                &mut tool,
                &ToolCallRequest {
                    name: shell::tool_name().into(),
                    arguments: args,
                },
            )
            .await;
            assert!(!out.success);
            assert_eq!(out.output, expect);
        }
        // 无审批口 + 带参(校验通过):逐字「无审批服务」
        let out = ToolPort::execute(
            &mut tool,
            &ToolCallRequest {
                name: shell::tool_name().into(),
                arguments: json!({
                    "command": "echo hi",
                    "description": "D",
                    "sandbox_permissions": "full-access",
                    "justification": "需要全盘写",
                }),
            },
        )
        .await;
        assert_eq!(
            out.output,
            "sandbox escalation requires approval, but no approval service is composed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// timeout_ms 校验:越界/类型错/与后台成对,逐字拒绝且零执行
    /// (spawn 前返回,无平台沙箱依赖)
    #[tokio::test]
    async fn bash_timeout_validation_verbatim() {
        let dir = std::env::temp_dir().join(format!("liuma-bash-tmv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut tool = BashTool::new(&dir);
        const MSG: &str = "invalid timeout_ms: expected an integer between 1 and 600000; use run_in_background for longer commands";
        let cases: Vec<(Value, &str)> = vec![
            (
                json!({ "command": "echo hi", "description": "D", "timeout_ms": 0 }),
                MSG,
            ),
            (
                json!({ "command": "echo hi", "description": "D", "timeout_ms": 600_001 }),
                MSG,
            ),
            (
                json!({ "command": "echo hi", "description": "D", "timeout_ms": "120000" }),
                MSG,
            ),
            (
                json!({ "command": "echo hi", "description": "D", "timeout_ms": 1.5 }),
                MSG,
            ),
            (
                json!({ "command": "echo hi", "description": "D", "timeout_ms": 60_000, "run_in_background": true }),
                "timeout_ms is only valid for foreground commands",
            ),
        ];
        for (args, expect) in cases {
            let out = ToolPort::execute(
                &mut tool,
                &ToolCallRequest {
                    name: shell::tool_name().into(),
                    arguments: args,
                },
            )
            .await;
            assert!(!out.success, "应拒绝: {expect}");
            assert_eq!(out.output, expect);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// spec 文案随通知口分支:「结算会通知」承诺只在口在场时写进
    /// 描述(行为承诺与接口一致)
    #[test]
    fn bash_spec_adapts_to_job_notify() {
        struct NoopNotify;
        impl SettlementNotificationPort for NoopNotify {
            fn notify(
                &self,
                _parent_session: &str,
                _text: String,
                _source: Value,
            ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
                Box::pin(std::future::ready(()))
            }
        }
        let dir = std::env::temp_dir().join(format!("liuma-bash-spec-{}", std::process::id()));
        let plain = BashTool::new(&dir);
        let plain_spec = serde_json::to_string(&plain.spec()).unwrap();
        assert!(
            !plain_spec.contains("notified"),
            "无通知口不得承诺通知:{plain_spec}"
        );
        let notified =
            BashTool::new(&dir).with_job_notify(std::sync::Arc::new(NoopNotify), "session-x");
        let notified_spec = serde_json::to_string(&notified.spec()).unwrap();
        assert!(
            notified_spec.contains("You are notified when the job settles"),
            "有通知口应承诺通知:{notified_spec}"
        );
    }
}
