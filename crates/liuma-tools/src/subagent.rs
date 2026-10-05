//! 子代理工具:
//! 默认后台委派 + 结算通知回投 + 续话控制 + 重启恢复。
//!
//! 结构:主会话的 tool/call 触发 → 独立子会话(自己的 EventLog 与
//! JSONL 文件,会话边界 = 日志边界)→ 嵌套 LoopEngine 跑任务。
//! 两种委派形态:
//! - **后台(默认)**:立即返回子代理 id,子代理在驻留任务里跑;结算时经
//!   [`SettlementNotificationPort`] 把一条 `subagent-settled` 通知投回父
//!   会话并唤醒(父空闲=下一 turn,忙碌=step 边界认领);子代理驻留
//!   idle,`send_message` 可续话(运行中=steer 中途插话,idle=开新 turn);
//!   每次结算都再通知。子会话首事件写
//!   `subagent/descriptor`、每次结算写 `subagent/settled`——重启后父会话
//!   attach 扫描无 settled 尾的子会话即可 cold-resume(持久驻留)。
//! - **前台(run_in_background=false)**:同步等待,最终报告作为 tool/result
//!   返回(workflow/ralph 编排器走同一前台入口)。
//!
//! 能力束窄化:子代理可写根 = 主 workspace 下的 `.liuma/subagents/<id>/`
//! (读全盘、写限子根)。子工具面 = 全量 minus 递归:bash(+jobs)/文件三件/
//! todo_write/goal/session_query(port 在场)/send_message(回发父,仅驻留);
//! 排除 subagent/workflow(递归)、ask_user_question(子代理不能向人提问)、
//! plan(计划模式是与人的审批契约)、wasm 组件(实例不可克隆)。
//! 取消传播:父令牌触发 → 子静默退出(不通知,避免唤醒已取消会话);
//! `interrupt_agent` 只停当前 turn(每 turn 新令牌),子代理保持可续话。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use liuma_agent_loop::{
    CancelToken, LlmEvent, LlmTransport, LoopEngine, LoopError, RequestHeader, SteerInput,
    Summarizer, ToolCallRequest, ToolOutput, ToolPort, ToolPortObj,
};
use liuma_host::JsonlBackend;
use liuma_llm::InvariantGate;
use liuma_session::{EventEnvelope, EventLog};
use serde_json::{Value, json};

/// 子代理会话句柄(会话化血缘的产出:槽位 id 供通知卡跳转/续话寻址,
/// 路径供独立重放)
#[derive(Debug, Clone)]
pub struct SubagentSessionHandle {
    /// 子会话槽位 id(非默认工作区为 "<ws>/<stem>" 复合形态)
    pub session_id: String,
    /// 子会话 session.jsonl 路径
    pub session_path: std::path::PathBuf,
}

/// 子会话工厂(会话化 + 句柄化 + 重启恢复):让 subagent
/// 子任务注册为带血缘(parentSessionId + origin:'subagent')的独立会话。
/// 实现方:liuma-core AppHost。None = 未注入,回落自建 `.liuma/subagents/<id>/`。
pub trait SessionFactory: Send + Sync {
    /// 创建带血缘的子会话,返回其槽位 id 与 session.jsonl 路径
    fn create_subagent(&self, parent: &str) -> SubagentSessionHandle;

    /// 扫描可重挂的驻留子代理(有 descriptor 标记者):返回
    /// (句柄, 是否中断[末事件非 settled], label, prompt)。实现方负责认领
    /// 防双挂(已认领的不返回);中断者由调用方先冷修夏(实现方完成)再重挂。
    fn resumable_children(
        &self,
        parent: &str,
    ) -> Vec<(SubagentSessionHandle, bool, String, String)> {
        let _ = parent;
        Vec::new()
    }

    /// 驻留退出释放认领(默认 no-op)
    fn release_child(&self, session_id: &str) {
        let _ = session_id;
    }
}

/// 通知 port(定义迁至 crate 根,泛化为「后台执行体 → 发起会话」:
/// 子代理结算 kind="subagent-settled"、回发消息 kind="subagent-message"、
/// shell job 结算 kind="shell-job-settled")。宿主实现:经 Notice 入
/// 目标会话队列——空闲=唤醒下一 turn,忙碌=引擎 step 边界认领。
/// 目标不存在时静默丢弃。旧路径 re-export 保持挂接面不变
pub use crate::SettlementNotificationPort;

/// 结算通知拼装(结算摘要 + 结算通知的消息形态):
/// 返回 (模型可见文本, source 染色载荷)。
pub fn settlement_notice(
    child_id: &str,
    stop_reason: &str,
    closing: Option<&str>,
) -> (String, Value) {
    let subject = format!("Background subagent {child_id}");
    let summary = match stop_reason {
        "completed" => {
            format!("{subject} finished and will do no further work unless you send it more.")
        }
        "aborted" => format!("{subject} was stopped before it finished."),
        "error" => format!("{subject} failed before it finished."),
        other => format!("{subject} ended abnormally ({other}) before it finished."),
    };
    let mut text = summary.clone();
    match closing {
        Some(closing) if !closing.trim().is_empty() => {
            text.push_str("\n\nIts closing message:\n\n");
            text.push_str(closing);
        }
        _ => text.push_str("\n\nIt left no closing message."),
    }
    let source = json!({
        "kind": "subagent-settled",
        "form": "notice",
        "summary": summary,
        "senderSessionId": child_id,
    });
    (text, source)
}

/// 对等代理消息拼装(kind=subagent-message,通知不进转录——7a81ea6 口径,
/// 可见性 = 任务面板行态摘要 + 目标会话日志染色行)。
/// sender 泛化:label 在场(子代理)→ `Message from agent <label> (<id>):`,
/// 缺席(主会话)→ `Message from agent <id>:`
pub(crate) fn agent_message_notice(
    sender_id: &str,
    label: Option<&str>,
    message: &str,
) -> (String, Value) {
    let subject = match label {
        Some(l) if !l.trim().is_empty() => format!("agent {l} ({sender_id})"),
        _ => format!("agent {sender_id}"),
    };
    let text = format!("Message from {subject}:\n\n{message}");
    let summary: String = message
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .chars()
        .take(80)
        .collect();
    let source = json!({
        "kind": "subagent-message",
        "form": "notice",
        "summary": summary,
        "senderSessionId": sender_id,
    });
    (text, source)
}

/// 子代理独立传输工厂:每个子代理构建自己的出网传输——后台并发
/// 不再被单一传输钉死。实现方:mount 面(每次 build_raw_transport)与
/// 测试(脚本组队列逐个弹出)。
pub type TransportFactory<T> = Arc<dyn Fn() -> Result<T, String> + Send + Sync>;

/// 驻留子代理的续话消息(send_message 投递;逐条一个 turn)
#[derive(Debug)]
pub enum ChildMsg {
    /// 续话 prompt(作为子代理的下一 turn 输入;source = 输入染色,
    /// 阶梯续话分支携带,与 steer 路径同构)
    Prompt { text: String, source: Option<Value> },
}

/// 对等代理寻址 port(宿主实现 = 投递阶梯,计划 peer-agent-messaging §1.1):
/// 路由、权限(归属链/terminated)、离线处置集中实现侧;工具层只做参数
/// 解析与失败呈递(失败工具结果 = 发送方审计)。
pub trait AddressingPort: Send + Sync {
    /// 投递一条消息(sender → target)。text/source 由调用方拼装(含染色),
    /// 实现侧透传不重组。阶梯:驻留活体(running = steer / idle = 开新
    /// turn)→ 装配主会话 Notice → 离线挂起收件箱。范围外/terminated =
    /// Err(结构化错误文本)。
    fn deliver(
        &self,
        sender: &str,
        target: &str,
        text: &str,
        source: Value,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<DeliveryKind, String>> + Send + '_>>;

    /// sender 的可发清单(含 relation/状态/label;list_agents 的数据背书)。
    fn roster(&self, sender: &str) -> Vec<RosterEntry>;
}

/// 投递结果阶梯(工具结果文案据此拼)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryKind {
    /// 目标运行中:消息入 steer 缓冲,step 边界认领(同一 turn)
    Steered,
    /// 目标驻留空闲:续话通道开新 turn
    TurnStarted,
    /// 目标为装配态主会话:入 Notice 队列
    Noticed,
    /// 目标离线:挂起收件箱,活体回归时投递
    Queued,
}

/// 对等代理清单项
#[derive(Debug, Clone)]
pub struct RosterEntry {
    /// 会话槽位 id
    pub session_id: String,
    /// 显示名(descriptor label / 委派 description)
    pub label: String,
    /// 与 sender 的 relation:parent / sibling / child
    pub relation: String,
    /// 存活态:running / idle / offline
    pub status: String,
}

/// 子代理注册项(进程内记录;子会话日志文件才是持久事实)。
/// 前台(一次性)子代理只用到前半;后台(驻留)子代理额外携带续话
/// 通道、中途插话缓冲与打断句柄。
#[derive(Clone)]
pub struct SubagentRecord {
    /// 子代理标识(单调递增;进程内簿记,模型面用 session_id 寻址)
    pub id: u64,
    /// 任务描述(subagent 的 description 参数;list_agents 的 label)
    pub task: String,
    /// 状态:running / idle / done / failed / cancelled / stopped
    /// (后台驻留:running↔idle 循环;前台一次性:终态)
    pub status: String,
    /// 子会话槽位 id(血缘跳转 / send_message 寻址 / 通知卡跳转)
    pub session_id: String,
    /// 子会话日志路径(独立重放入口)
    pub session_path: String,
    /// 是否后台驻留子代理(list_agents 只列后台;前台一次性不列——
    /// "one-shot children cannot be continued, so the model never selects them")
    pub background: bool,
    /// 续话通道(后台驻留;None = 前台一次性)
    pub tx: Option<tokio::sync::mpsc::UnboundedSender<ChildMsg>>,
    /// 中途插话缓冲(运行中 send_message 的落点,引擎 step 边界认领)
    pub steer: Option<Arc<Mutex<VecDeque<SteerInput>>>>,
    /// 当前 turn 打断句柄(interrupt_agent;只停正在跑的 turn)
    pub stop: Option<Arc<tokio::sync::Notify>>,
    /// 委派 prompt(副行展示;重挂取自 descriptor)
    pub prompt: String,
    /// 委派/重挂时刻(ms)
    pub started_at: i64,
    /// 终态时刻(ms;None = 仍在驻留)
    pub ended_at: Option<i64>,
    /// 最近入站消息摘要(任务面板行副行;阶梯驻留投递时更新,
    /// 非持久——重启后由日志染色行承担内容可见性)
    pub last_message: Option<String>,
}

/// 注册表状态变化观察回调(宿主据此广播 jobs 帧;回调内禁止再锁 records)
pub type RegistryChangeHook = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Default)]
pub struct SubagentRegistry {
    inner: Arc<RegistryInner>,
}

#[derive(Default)]
struct RegistryInner {
    records: Mutex<Vec<SubagentRecord>>,
    on_change: Mutex<Option<RegistryChangeHook>>,
}

impl SubagentRegistry {
    /// 挂状态变化观察回调(覆盖式;装配时由宿主接线)
    pub fn set_on_change(&self, hook: Option<RegistryChangeHook>) {
        *self
            .inner
            .on_change
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = hook;
    }

    /// 状态变化通知(guard 释放后调用,防回调重入死锁)
    fn changed(&self) {
        let hook = self
            .inner
            .on_change
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    /// 弱引用(宿主 jobs 源登记;不阻止注册表随工具释放)
    pub fn downgrade(&self) -> WeakRegistry {
        WeakRegistry(Arc::downgrade(&self.inner))
    }

    /// 快照(background 驻留子代理;前台一次性不进 jobs 呈现)
    pub fn background_records(&self) -> Vec<SubagentRecord> {
        self.inner
            .records
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|r| r.background)
            .cloned()
            .collect()
    }
}

impl std::ops::Deref for SubagentRegistry {
    type Target = Mutex<Vec<SubagentRecord>>;

    fn deref(&self) -> &Self::Target {
        &self.inner.records
    }
}

/// 子会话事件出口(宿主据此向客户端广播子会话实时流;参数 =
/// 子会话槽位 id、引擎事件)。None = 只落盘不广播(CLI/单测)。
pub type SubagentEventSink = Arc<dyn Fn(&str, &EventEnvelope) + Send + Sync>;

/// jobs 呈现接线(宿主登记注册表为父会话的 jobs 源)
pub type JobsBinding = Arc<dyn Fn(&str, SubagentRegistry) + Send + Sync>;

/// shell job 呈现接线(宿主登记 JobsRegistry 为父会话的 jobs 源;
/// 状态变化 → session/jobs 帧,与子代理行合并广播)
pub type ShellJobsBinding = Arc<dyn Fn(&str, crate::JobsRegistry) + Send + Sync>;

/// 子代理宿主桥:jobs 呈现接线 + 子会话事件实时流转发,
/// 装配时由宿主(liuma-core)构造、经 MountContext 注入
pub struct SubagentBridge {
    /// 登记注册表为某父会话的 jobs 源(状态变化 → session/jobs 帧)
    pub jobs: JobsBinding,
    /// 子会话事件出口(引擎事件 → session/event 实时流)
    pub events: SubagentEventSink,
    /// shell job 注册表登记(状态变化 → session/jobs 帧合并广播)
    pub shell_jobs: ShellJobsBinding,
}

/// 注册表弱引用(宿主持有;upgrade 失败 = 工具已随会话释放)
#[derive(Clone, Default)]
pub struct WeakRegistry(std::sync::Weak<RegistryInner>);

impl WeakRegistry {
    /// 升级为强引用(失败 = 已释放)
    pub fn upgrade(&self) -> Option<SubagentRegistry> {
        self.0.upgrade().map(|inner| SubagentRegistry { inner })
    }
}

/// 子代理共享装配产出(前台/驻留两条执行路径共用)
struct ChildParts {
    /// 子可写根(bash 能力束窄化 + FileTools 根)
    child_root: std::path::PathBuf,
    header: RequestHeader,
    log: Arc<Mutex<EventLog>>,
    /// 子代理自己的后台任务注册表(bash 后台 + jobs 工具共享)
    jobs: crate::JobsRegistry,
    /// 子代理 job 结算通知路由(驻留形态在场;前台一次性 = None,
    /// turn 结束后无会话可投,job 静默落定)
    job_notify: Option<Arc<dyn SettlementNotificationPort>>,
}

/// 子代理会话的 job 结算通知路由(发起会话 = 子代理自己):
/// 与 `send_message` 同一语义——运行中 → steer 最近 step(中途插话),
/// 空闲 → 续话通道开新 turn。注册表已终结 = 静默丢弃
struct ChildJobNotify {
    self_id: String,
    registry: SubagentRegistry,
}

impl SettlementNotificationPort for ChildJobNotify {
    fn notify(
        &self,
        _session: &str,
        text: String,
        source: Value,
    ) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let registry = self.registry.clone();
        let self_id = self.self_id.clone();
        Box::pin(async move {
            let Ok(r) = registry.lock() else {
                return;
            };
            let Some(rec) = r.iter().find(|s| s.session_id == self_id) else {
                return;
            };
            if rec.status == "running"
                && let Some(buf) = &rec.steer
            {
                let mut b = buf.lock().unwrap_or_else(|p| p.into_inner());
                b.push_back(SteerInput {
                    id: format!("steer-{}", uuid::Uuid::now_v7()),
                    text,
                    images: Vec::new(),
                    files: Vec::new(),
                    source: Some(source),
                });
                return;
            }
            if let Some(tx) = &rec.tx {
                let _ = tx.send(ChildMsg::Prompt {
                    text,
                    source: Some(source),
                });
            }
        })
    }
}

/// 子代理 system prompt(公共骨架;驻留 + 寻址在场时追加可发范围指引)
fn child_system_prompt(child_root: &std::path::Path, parent_id: Option<&str>) -> String {
    let mut system = format!(
        "You are a liuma subagent executing one task in an isolated workspace ({}). \
Finish the task and reply with the result only — you cannot ask questions.",
        child_root.display()
    );
    if let Some(parent) = parent_id {
        // 可续话子代理指引:告知父 id 与寻址范围(直接父/兄弟/直属子)
        system.push_str(&format!(
            "\n\nYour parent agent id is {parent}. With `send_message` you can reach any \
agent in your addressable range — your direct parent, siblings sharing it, and your own \
direct children; your final reply is also delivered to your parent automatically."
        ));
    }
    system
}

/// 子代理共享装配:子目录、子 header、日志与持久化后端、后台任务注册表。
/// 工具集按 turn 构建([`build_child_tools`])——取消令牌每 turn 一枚,
/// 运行中的 bash 进程靠它中断。
async fn prepare_child_parts(
    handle: &SubagentSessionHandle,
    model: &str,
    parent_id: Option<&str>,
) -> Result<ChildParts, String> {
    let child_root = handle
        .session_path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    tokio::fs::create_dir_all(&child_root)
        .await
        .map_err(|e| format!("subagent workspace create failed: {e}"))?;
    let header = RequestHeader {
        model: model.to_string(),
        system: child_system_prompt(&child_root, parent_id),
        temperature: 0.0,
        reasoning_effort: None,
        tools: Vec::new(),
    };
    // open(追加,缺则建)而非 create(截断):重挂形态必须保全既有历史
    let backend = JsonlBackend::open(&handle.session_path)
        .map_err(|e| format!("subagent session create failed: {e}"))?;
    // 日志从**现文件**重建高水位(fresh 子会话种子 seq 1/2 在档,续写从
    // 3 起——空日志从 1 起会与种子重复,整份日志非连续)。
    // 接手即治愈:文件若带历史损伤(seq 断裂/重复块,旧版本「空日志
    // 重复写」的遗留),以最长连续前缀重建并把文件重写为该前缀——接手
    // 点是单写者,重写安全;新追加自此保持连续
    let mut log_inner = EventLog::new();
    {
        let raw = backend
            .load()
            .map_err(|e| format!("subagent session reload failed: {e}"))?;
        let mut broken = false;
        for ev in raw {
            // append 拒绝非连续(含旧版本重复块损伤):断裂即停,内存
            // 日志恰好持有最长连续前缀
            if log_inner.append(ev).is_err() {
                broken = true;
                break;
            }
        }
        if broken {
            // 接手即治愈:文件重写为最长连续前缀——接手点是单写者,
            // 重写安全;损伤尾段(旧版本重复块遗留)自此收敛
            eprintln!(
                "[subagent] 日志含损伤,按最长连续前缀治愈: {}",
                handle.session_path.display()
            );
            let mut f = std::fs::File::create(&handle.session_path)
                .map_err(|e| format!("subagent session heal failed: {e}"))?;
            use std::io::Write as _;
            for ev in log_inner.iter() {
                serde_json::to_writer(&mut f, &ev)
                    .map_err(|e| format!("subagent session heal failed: {e}"))?;
                f.write_all(b"\n")
                    .map_err(|e| format!("subagent session heal failed: {e}"))?;
            }
            f.flush()
                .map_err(|e| format!("subagent session heal failed: {e}"))?;
        }
    }
    // 持久化汇 = 日志锁内定 seq 即写盘(单写权威;turn sink 不再手动落盘)
    let sink_backend = backend.clone();
    log_inner.set_durability_sink(Box::new(move |ev| {
        sink_backend.append(ev).map_err(|e| e.to_string())
    }));
    let log = Arc::new(Mutex::new(log_inner));
    Ok(ChildParts {
        child_root,
        header,
        log,
        jobs: crate::JobsRegistry::new(),
        job_notify: None,
    })
}

/// 子代理工具面构建上下文(每 turn 重建工具集时的全部权属)
struct ChildToolContext<'a> {
    turn_token: &'a CancelToken,
    parts: &'a ChildParts,
    query_port: Option<Arc<dyn crate::session_query::SessionQueryPort>>,
    session_id: &'a str,
    /// 对等寻址 port(驻留 + port 在场才装配 AgentMessageTool;
    /// CLI 无 port 形态 = 子面无寻址工具,与前台同步语义一致)
    addressing: Option<Arc<dyn AddressingPort>>,
    /// 本子代理的显示名(descriptor label;notice 文本模板用)
    self_label: Option<&'a str>,
}

/// 按 turn 构建子工具集(全量 minus 递归;bash 携带本 turn 取消令牌,
/// 运行中进程随令牌中断):
/// bash(+jobs 后台)/ 文件三件 / todo_write / goal / jobs /
/// session_query(port 在场)/ send_message 对等寻址(port 在场的驻留形态)。
/// 排除:subagent/workflow(递归)、ask_user_question(不能向人提问)、
/// plan(与人的审批契约)、wasm 组件(实例不可克隆)。
fn build_child_tools(ctx: ChildToolContext) -> Result<liuma_agent_loop::ToolSet, String> {
    let child_bash =
        crate::BashTool::new(&ctx.parts.child_root).with_cancel(ctx.turn_token.clone());
    let child_bash = child_bash.with_jobs(ctx.parts.jobs.clone());
    // job 结算通知投回子代理自己(驻留形态;发起会话 = 本子代理)
    let child_bash = match &ctx.parts.job_notify {
        Some(port) => child_bash.with_job_notify(Arc::clone(port), ctx.session_id),
        None => child_bash,
    };
    let mut tools: Vec<Box<dyn ToolPortObj>> = vec![
        Box::new(child_bash),
        Box::new(crate::FileTools::new(&ctx.parts.child_root)),
        Box::new(crate::TodoWriteTool::new(Arc::clone(&ctx.parts.log))),
        Box::new(crate::GoalTool::new(Arc::clone(&ctx.parts.log))),
        Box::new(crate::JobTool::new(ctx.parts.jobs.clone())),
    ];
    if let Some(port) = ctx.query_port {
        tools.push(Box::new(crate::session_query::SessionQueryTool::new(
            port,
            ctx.session_id,
        )));
    }
    if let Some(port) = ctx.addressing {
        tools.push(Box::new(AgentMessageTool::new(
            ctx.session_id,
            ctx.self_label.map(str::to_string),
            port,
        )));
    }
    liuma_agent_loop::ToolSet::new(tools).map_err(|e| format!("child toolset assembly failed: {e}"))
}

/// 对等代理消息工具(父面与子面同装,仅 sender 身份不同):
/// `send_message`(范围内任意 agent)/ `list_agents`(roster)。
/// 失败 = port 错误文本原样透传(isError;发送方日志即审计)。
pub struct AgentMessageTool {
    sender_id: String,
    sender_label: Option<String>,
    port: Arc<dyn AddressingPort>,
}

impl AgentMessageTool {
    /// 以 sender 身份与寻址 port 构建
    pub fn new(
        sender_id: &str,
        sender_label: Option<String>,
        port: Arc<dyn AddressingPort>,
    ) -> Self {
        Self {
            sender_id: sender_id.to_string(),
            sender_label,
            port,
        }
    }
}

impl ToolPort for AgentMessageTool {
    fn specs(&self) -> Vec<Value> {
        vec![
            json!({
                "type": "function",
                "function": {
                    "name": "send_message",
                    "description": "Send a message to any agent in your addressable range: your direct parent, a sibling sharing your parent, or one of your direct children. If the target is still working, the message steers its nearest step; if it is idle, the message starts a turn; if it is offline, the message is queued in its inbox and delivered when the agent is back. This call returns no answer from the target — only confirmation that the message was delivered. A failure means the message was NOT delivered.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "agent_id": { "type": "string", "description": "The agent id of the target (see list_agents for your addressable agents)." },
                            "message": { "type": "string", "description": "The message to deliver to the agent." }
                        },
                        "required": ["agent_id", "message"]
                    },
                },
            }),
            json!({
                "type": "function",
                "function": {
                    "name": "list_agents",
                    "description": "List the agents you can message, by durable id, relation (parent, sibling, child) and live status (running, idle, or offline). Use it to recall who exists, not to poll for completion — you are told when a child finishes. The snapshot is not a delivery promise — `send_message` performs the authoritative check and may still fail.",
                    "parameters": { "type": "object", "properties": {} },
                },
            }),
        ]
    }

    async fn execute(&mut self, call: &ToolCallRequest) -> ToolOutput {
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
        match call.name.as_str() {
            "send_message" => {
                let (Some(agent_id), Some(message)) = (
                    arguments["agent_id"].as_str(),
                    arguments["message"].as_str(),
                ) else {
                    return fail(
                        "send_message requires arguments.agent_id and arguments.message (strings)"
                            .into(),
                    );
                };
                let (text, source) =
                    agent_message_notice(&self.sender_id, self.sender_label.as_deref(), message);
                match self
                    .port
                    .deliver(&self.sender_id, agent_id, &text, source)
                    .await
                {
                    Ok(kind) => ToolOutput {
                        output: match kind {
                            DeliveryKind::Steered => format!(
                                "message delivered to agent {agent_id} (steered into its current turn)"
                            ),
                            DeliveryKind::TurnStarted => format!(
                                "message delivered to agent {agent_id} (started a new turn)"
                            ),
                            DeliveryKind::Noticed => {
                                format!("message delivered to agent {agent_id}")
                            }
                            DeliveryKind::Queued => format!(
                                "message queued for agent {agent_id}; it will be delivered when the agent is back"
                            ),
                        },
                        success: true,
                        ..Default::default()
                    },
                    Err(e) => fail(e),
                }
            }
            "list_agents" => {
                let entries = self.port.roster(&self.sender_id);
                if entries.is_empty() {
                    return ToolOutput {
                        output: "(no addressable agents)".into(),
                        success: true,
                        ..Default::default()
                    };
                }
                let rows: Vec<String> = entries
                    .iter()
                    .map(|e| format!("{} [{}/{}] {}", e.session_id, e.relation, e.status, e.label))
                    .collect();
                ToolOutput {
                    output: rows.join("\n"),
                    success: true,
                    ..Default::default()
                }
            }
            _ => fail(format!("unknown tool: {}", call.name)),
        }
    }
}

/// 跑一个子 turn:select 软取消(父令牌级联或 interrupt future 触发 →
/// 令牌取消 → 子引擎安全点收尾)。事件逐条落子日志(backend 持久化);
/// 时钟取系统毫秒。
#[allow(clippy::too_many_arguments)]
async fn run_child_turn<G>(
    engine: &mut LoopEngine,
    gate: &mut G,
    tools: &mut liuma_agent_loop::ToolSet,
    prompt: &str,
    turn_token: &CancelToken,
    parent_cancel: &CancelToken,
    interrupt: impl Future<Output = ()> + Send,
    session_id: &str,
    event_sink: Option<&SubagentEventSink>,
) -> Result<liuma_agent_loop::TurnOutcome, LoopError>
where
    G: LlmTransport + Summarizer + Send,
{
    // 双取消源:父令牌级联 / interrupt 句柄,任一触发即取消本 turn 令牌
    let canceller = {
        let turn_token = turn_token.clone();
        let parent_cancel = parent_cancel.clone();
        async move {
            tokio::select! {
                _ = parent_cancel.cancelled() => {}
                _ = interrupt => {}
            }
            turn_token.cancel();
        }
    };
    let mut sink = |ev: &EventEnvelope| {
        // 实时流出口(桌面广播);落盘经日志持久化汇
        if let Some(relay) = event_sink {
            relay(session_id, ev);
        }
    };
    let run = engine.run_turn(
        prompt,
        None,
        &[],
        &[],
        &[],
        gate,
        tools,
        &|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)
        },
        &mut sink,
    );
    tokio::pin!(run);
    tokio::pin!(canceller);
    tokio::select! {
        out = &mut run => out,
        // 取消在子执行中触发:等待子引擎在安全点收尾(取消后 run 分支
        // 很快以 Cancelled 返回;此臂只负责不再等取消源)
        _ = &mut canceller => (&mut run).await,
    }
}

/// 注册表状态更新(找不到 = 会话已终结,忽略)。终态记 ended_at;
/// 回到运行/空闲态清空(驻留循环 running↔idle 反复)。变化后通知观察方。
fn set_status(registry: &SubagentRegistry, session_id: &str, status: &str) {
    let changed = if let Ok(mut r) = registry.lock() {
        match r.iter_mut().find(|s| s.session_id == session_id) {
            Some(rec) => {
                rec.status = status.into();
                // idle = 驻留最近一轮结算时刻(续话再跑时被 running 清空)
                rec.ended_at =
                    matches!(status, "idle" | "done" | "failed" | "cancelled" | "stopped")
                        .then(now_ms);
                true
            }
            None => false,
        }
    } else {
        false
    };
    if changed {
        registry.changed();
    }
}

/// 记录已被 terminate_agent 置 terminated(驻留循环的收口判据)
fn is_terminated(registry: &SubagentRegistry, session_id: &str) -> bool {
    registry.lock().is_ok_and(|r| {
        r.iter()
            .any(|s| s.session_id == session_id && s.status == "terminated")
    })
}

/// 子会话标记事件落档(descriptor/settled;重启恢复的判据;落盘经
/// 日志持久化汇,锁内原子)
fn commit_child_marker(log: &Arc<Mutex<EventLog>>, ty: &str, data: Value) {
    if let Ok(mut l) = log.lock() {
        // 归因事件照守卫约定标 ignorable:未登记类型不拒绝旧读取方重建日志
        let ev = EventEnvelope::new_ignorable(ty, now_ms(), data);
        let _ = l.append(ev);
    }
}

/// 离线收件箱冲账提取:日志中未消费的 pending(判据 = 有
/// `agent/inbox/pending` 无对应 `agent/inbox/consumed`,按落档序)。
/// 消费方把每条作为 turn 输入,**输入落档后**紧跟 consumed——顺序
/// 不可反(见调用点注释)。返回 (id, text, source 染色)。
pub fn extract_inbox_pending(log: &EventLog) -> Vec<(String, String, Option<Value>)> {
    let mut consumed = std::collections::HashSet::new();
    let mut pending: Vec<(String, String, Option<Value>)> = Vec::new();
    for ev in log.iter() {
        match ev.r#type.as_str() {
            "agent/inbox/pending" => {
                let Some(id) = ev.data["id"].as_str() else {
                    continue;
                };
                if id.is_empty() {
                    continue;
                }
                pending.push((
                    id.to_string(),
                    ev.data["text"].as_str().unwrap_or_default().to_string(),
                    ev.data.get("source").cloned(),
                ));
            }
            "agent/inbox/consumed" => {
                if let Some(id) = ev.data["id"].as_str() {
                    consumed.insert(id.to_string());
                }
            }
            _ => {}
        }
    }
    pending
        .into_iter()
        .filter(|(id, _, _)| !consumed.contains(id))
        .collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// subagent 共享核:委派执行的全部权属。收拢为 Arc 共享体——工具面
/// (会话内)与宿主 spawn 入口(桌面创建,经 SubagentBridge 注册的
/// 闭包)共享同一核,注册表/血缘/通知单点真身。
pub struct SubagentCore<T> {
    /// 父 workspace(子根 = `.liuma/subagents/<id>/`,能力束窄化)
    pub root: std::path::PathBuf,
    /// 子代理独立传输工厂(每个子代理一份传输;后台并发的前提)
    pub transport_factory: TransportFactory<T>,
    /// 模型标识(子 header)
    pub model: String,
    /// 父会话软取消令牌(级联:父取消 → 子静默退出)
    pub cancel: CancelToken,
    /// 注册表
    pub registry: SubagentRegistry,
    /// 子会话工厂(Some = 子任务注册为带血缘会话;None = 回落后建)
    pub factory: Option<Arc<dyn SessionFactory>>,
    /// 父会话 id(血缘 parent;factory 注入时传给 create_subagent)
    pub parent_id: String,
    /// 通知 port(None = 同步语义:模型面无后台参数、执行强制前台)
    pub notify: Option<Arc<dyn SettlementNotificationPort>>,
    /// 子会话事件出口(None = 只落盘)
    pub event_sink: Option<SubagentEventSink>,
    /// 会话检索 port(子代理工具面扩装,缺 = 子代理无检索)
    pub query_port: Option<Arc<dyn crate::session_query::SessionQueryPort>>,
    /// 对等寻址 port(驻留子面 AgentMessageTool 的背端;缺 = 子面无寻址)
    pub addressing: Option<Arc<dyn AddressingPort>>,
}

impl<T> Clone for SubagentCore<T> {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            transport_factory: Arc::clone(&self.transport_factory),
            model: self.model.clone(),
            cancel: self.cancel.clone(),
            registry: self.registry.clone(),
            factory: self.factory.clone(),
            parent_id: self.parent_id.clone(),
            notify: self.notify.clone(),
            event_sink: self.event_sink.clone(),
            query_port: self.query_port.clone(),
            addressing: self.addressing.clone(),
        }
    }
}

/// subagent 工具:委派任务 → 嵌套引擎执行(前台等结果 / 后台即返回)
pub struct SubagentTool<T> {
    core: Arc<SubagentCore<T>>,
}

impl<T> std::ops::Deref for SubagentTool<T> {
    type Target = SubagentCore<T>;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl<T> SubagentTool<T>
where
    T: LlmTransport + Summarizer + Send + 'static,
{
    /// 以父根、传输工厂与模型构建(与父 BashTool 同一 workspace;
    /// 子可写根在每次执行时按 id 窄化)
    pub fn new(
        root: impl Into<std::path::PathBuf>,
        transport_factory: TransportFactory<T>,
        model: String,
    ) -> Self {
        Self {
            core: Arc::new(SubagentCore {
                root: root.into(),
                transport_factory,
                model,
                cancel: CancelToken::new(),
                registry: SubagentRegistry::default(),
                factory: None,
                parent_id: String::new(),
                notify: None,
                event_sink: None,
                query_port: None,
                addressing: None,
            }),
        }
    }

    /// 核字段替换式 builder(装配期使用;克隆核重建 Arc)
    fn with_core(mut self, f: impl FnOnce(&mut SubagentCore<T>)) -> Self {
        let mut core = (*self.core).clone();
        f(&mut core);
        self.core = Arc::new(core);
        self
    }

    /// 注入子会话事件出口(桌面广播实时流)
    pub fn with_event_sink(self, sink: SubagentEventSink) -> Self {
        self.with_core(|c| c.event_sink = Some(sink))
    }

    /// 注入子会话工厂(None/未注入 = 回落后建)
    pub fn with_session_factory(self, factory: Arc<dyn SessionFactory>) -> Self {
        self.with_core(|c| c.factory = Some(factory))
    }

    /// 设置父会话 id(血缘 parent;factory 注入时传给 create_subagent)
    pub fn with_parent_id(self, parent: &str) -> Self {
        self.with_core(|c| c.parent_id = parent.to_string())
    }

    /// 设置父取消令牌(与 engine/REPL Ctrl-C 同源,级联到子)
    pub fn with_cancel(self, token: CancelToken) -> Self {
        self.with_core(|c| c.cancel = token)
    }

    /// 共享注册表(workflow/ralph 编排器与顶层 subagent 工具共用清单)
    pub fn with_registry(self, registry: SubagentRegistry) -> Self {
        self.with_core(|c| c.registry = registry)
    }

    /// 注入通知 port(注入后模型面切后台默认形态)
    pub fn with_notify(self, notify: Arc<dyn SettlementNotificationPort>) -> Self {
        self.with_core(|c| c.notify = Some(notify))
    }

    /// 注入会话检索 port(子代理工具面扩装)
    pub fn with_query_port(self, port: Arc<dyn crate::session_query::SessionQueryPort>) -> Self {
        self.with_core(|c| c.query_port = Some(port))
    }

    /// 注入对等寻址 port(子面 AgentMessageTool 装配前提)
    pub fn with_addressing(self, port: Arc<dyn AddressingPort>) -> Self {
        self.with_core(|c| c.addressing = Some(port))
    }
}

impl<T> SubagentCore<T>
where
    T: LlmTransport + Summarizer + Send + 'static,
{
    /// 建子会话:factory 注入 → 带血缘会话(返回槽位 id);未注入 → 回落
    /// 自建 `.liuma/subagents/<id>/`(id 为本地簿记形态)
    fn make_session(&self) -> (u64, SubagentSessionHandle) {
        let id = self.next_id();
        match &self.factory {
            Some(f) => (id, f.create_subagent(&self.parent_id)),
            None => {
                let root = self.root.join(format!(".liuma/subagents/{id}"));
                let path = root.join("session.jsonl");
                (
                    id,
                    SubagentSessionHandle {
                        session_id: format!("subagent-{id}"),
                        session_path: path,
                    },
                )
            }
        }
    }

    fn next_id(&self) -> u64 {
        self.registry
            .lock()
            .map(|r| r.iter().map(|s| s.id).max().unwrap_or(0) + 1)
            .unwrap_or(1)
    }

    /// 前台执行一次子代理任务:嵌套引擎 + 独立日志 + 窄化工具集,等结果。
    /// workflow/ralph 编排器与 run_in_background=false 同走此入口。
    pub(crate) async fn run_foreground(&self, prompt: &str) -> ToolOutput {
        let (id, handle) = self.make_session();
        let path_str = handle.session_path.display().to_string();
        let parts = match prepare_child_parts(&handle, &self.model, None).await {
            Ok(parts) => parts,
            Err(msg) => {
                return ToolOutput {
                    output: msg,
                    success: false,
                    ..Default::default()
                };
            }
        };
        let started_at = now_ms();
        self.registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(SubagentRecord {
                id,
                task: prompt.to_string(),
                status: "running".into(),
                session_id: handle.session_id.clone(),
                session_path: path_str.clone(),
                background: false,
                tx: None,
                steer: None,
                stop: None,
                prompt: prompt.to_string(),
                started_at,
                ended_at: None,
                last_message: None,
            });
        self.registry.changed();

        let transport = match (self.transport_factory)() {
            Ok(t) => t,
            Err(e) => {
                return ToolOutput {
                    output: format!("subagent transport unavailable: {e}"),
                    success: false,
                    ..Default::default()
                };
            }
        };
        let mut gate = InvariantGate::new(transport, Arc::clone(&parts.log));
        let mut engine = LoopEngine::new(parts.header.clone(), Arc::clone(&parts.log));
        // 每 turn 一枚令牌:前台只此一 turn,父取消传播直达;
        // 工具集随令牌构建(运行中 bash 可中断)
        let turn_token = CancelToken::new();
        engine.set_cancel(turn_token.clone());
        let mut tools = match build_child_tools(ChildToolContext {
            turn_token: &turn_token,
            parts: &parts,
            query_port: self.query_port.clone(),
            session_id: &handle.session_id,
            addressing: None,
            self_label: None,
        }) {
            Ok(t) => t,
            Err(msg) => {
                return ToolOutput {
                    output: msg,
                    success: false,
                    ..Default::default()
                };
            }
        };

        let parent_cancel = self.cancel.clone();
        let outcome = run_child_turn(
            &mut engine,
            &mut gate,
            &mut tools,
            prompt,
            &turn_token,
            &parent_cancel,
            // 前台无 interrupt 句柄:永不就绪的 future 占位
            std::future::pending::<()>(),
            &handle.session_id,
            self.event_sink.as_ref(),
        )
        .await;

        let status = match &outcome {
            Ok(_) => "done",
            Err(LoopError::Cancelled) => "cancelled",
            Err(_) => "failed",
        };
        if let Ok(mut r) = self.registry.lock()
            && let Some(rec) = r.iter_mut().find(|s| s.id == id)
        {
            rec.status = status.into();
        }

        match outcome {
            Ok(o) => ToolOutput {
                output: format!(
                    "{}\n\n(subagent {} · session {} · events {}..{})",
                    o.assistant_message, handle.session_id, path_str, o.seq_range.0, o.seq_range.1
                ),
                success: true,
                ..Default::default()
            },
            Err(e) => ToolOutput {
                output: format!(
                    "subagent {} failed: {e} (session {path_str})",
                    handle.session_id
                ),
                success: false,
                ..Default::default()
            },
        }
    }

    /// 后台委派:注册驻留记录 → spawn 驻留任务 → 立即返回子代理 id。
    /// 驻留任务跑初始 turn → 结算通知 → 转 idle 等 send_message。
    /// (共享核方法:模型面工具与桌面 spawn 入口同一路)
    pub fn start_background(&self, label: &str, prompt: &str) -> ToolOutput {
        let Some(notify) = self.notify.clone() else {
            // 无通知 port = 接口无后台语义(不可达:specs 不声明后台参数;
            // 兜底拒绝,行为承诺与接口一致)
            return ToolOutput {
                output: "subagent background requires the notification port".into(),
                success: false,
                ..Default::default()
            };
        };
        let (id, handle) = self.make_session();
        let session_id = handle.session_id.clone();
        let path_str = handle.session_path.display().to_string();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<ChildMsg>();
        let stop = Arc::new(tokio::sync::Notify::new());
        let steer = Arc::new(Mutex::new(VecDeque::new()));
        self.registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(SubagentRecord {
                id,
                task: label.to_string(),
                status: "running".into(),
                session_id: session_id.clone(),
                session_path: path_str,
                background: true,
                tx: Some(tx),
                steer: Some(Arc::clone(&steer)),
                stop: Some(Arc::clone(&stop)),
                prompt: prompt.to_string(),
                started_at: now_ms(),
                ended_at: None,
                last_message: None,
            });
        self.registry.changed();
        tokio::spawn(run_resident_child(ResidentChild {
            handle,
            label: label.to_string(),
            event_sink: self.event_sink.clone(),
            initial_prompt: prompt.to_string(),
            model: self.model.clone(),
            transport_factory: Arc::clone(&self.transport_factory),
            registry: self.registry.clone(),
            parent_id: self.parent_id.clone(),
            parent_cancel: self.cancel.clone(),
            notify,
            stop,
            steer,
            rx,
            query_port: self.query_port.clone(),
            addressing: self.addressing.clone(),
            release: Arc::new(|| ()),
            first: Some(ChildMsg::Prompt {
                text: prompt.to_string(),
                source: None,
            }),
            resumed_interrupted: false,
        }));
        ToolOutput {
            output: format!("started subagent {}", session_id),
            success: true,
            ..Default::default()
        }
    }

    /// 重启恢复:扫描父会话下有 descriptor 标记的子会话并重挂为
    /// 驻留(实现方已认领防双挂;中断者先冷修复并落「已恢复」settled
    /// 标记,**不投父通知**——通知即 turn 输入,重启不自动续跑)。
    /// 无 factory/notify 或无 tokio runtime(纯装配单测)→ no-op。
    pub fn resume_children(&self) {
        let Some(factory) = self.factory.clone() else {
            return;
        };
        let Some(notify) = self.notify.clone() else {
            return;
        };
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        for (handle, interrupted, label, prompt) in factory.resumable_children(&self.parent_id) {
            let session_id = handle.session_id.clone();
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<ChildMsg>();
            let stop = Arc::new(tokio::sync::Notify::new());
            let steer = Arc::new(Mutex::new(VecDeque::new()));
            let id = self.next_id();
            self.registry
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(SubagentRecord {
                    id,
                    task: label.clone(),
                    status: "idle".into(),
                    session_id: session_id.clone(),
                    session_path: handle.session_path.display().to_string(),
                    background: true,
                    tx: Some(tx),
                    steer: Some(Arc::clone(&steer)),
                    stop: Some(Arc::clone(&stop)),
                    prompt: prompt.clone(),
                    started_at: now_ms(),
                    ended_at: None,
                    last_message: None,
                });
            self.registry.changed();
            let release_factory = Arc::clone(&factory);
            let release_id = session_id.clone();
            tokio::spawn(run_resident_child(ResidentChild {
                handle,
                label,
                event_sink: self.event_sink.clone(),
                initial_prompt: String::new(),
                model: self.model.clone(),
                transport_factory: Arc::clone(&self.transport_factory),
                registry: self.registry.clone(),
                parent_id: self.parent_id.clone(),
                parent_cancel: self.cancel.clone(),
                notify: Arc::clone(&notify),
                stop,
                steer,
                rx,
                query_port: self.query_port.clone(),
                addressing: self.addressing.clone(),
                release: Arc::new(move || release_factory.release_child(&release_id)),
                first: None,
                resumed_interrupted: interrupted,
            }));
        }
    }
}

/// 驻留子代理任务的全部权属(spawn 一次性移交)
struct ResidentChild<T> {
    handle: SubagentSessionHandle,
    label: String,
    /// 子会话事件出口(新建与重挂都从工具注入)
    event_sink: Option<SubagentEventSink>,
    /// 初始 prompt(descriptor 落档用;重挂为空)
    initial_prompt: String,
    model: String,
    transport_factory: TransportFactory<T>,
    registry: SubagentRegistry,
    parent_id: String,
    parent_cancel: CancelToken,
    notify: Arc<dyn SettlementNotificationPort>,
    stop: Arc<tokio::sync::Notify>,
    steer: Arc<Mutex<VecDeque<SteerInput>>>,
    rx: tokio::sync::mpsc::UnboundedReceiver<ChildMsg>,
    query_port: Option<Arc<dyn crate::session_query::SessionQueryPort>>,
    /// 对等寻址 port(子面 AgentMessageTool 背端;None = 子面无寻址)
    addressing: Option<Arc<dyn AddressingPort>>,
    /// 驻留退出释放认领(重挂形态必持;新建为 no-op)
    release: Arc<dyn Fn() + Send + Sync>,
    /// 初始 prompt(委派即带;与续话统一走消息循环)
    first: Option<ChildMsg>,
    /// 重挂且上次被中断(落「已恢复」settled 标记,下次扫描视为已结)
    resumed_interrupted: bool,
}

/// 驻留子代理主循环入口:退出统一释放认领(防双挂)
async fn run_resident_child<T>(child: ResidentChild<T>)
where
    T: LlmTransport + Summarizer + Send + 'static,
{
    let release = Arc::clone(&child.release);
    run_resident_child_inner(child).await;
    release();
}

/// 驻留子代理主循环:逐条消息一个 turn;每次结算通知父会话并落 settled
/// 标记;运行中 send_message 经 steer_buf 中途插话,turn 间排干续跑;
/// 父取消静默退出;interrupt 只停当前 turn(子代理保持可续话)。
async fn run_resident_child_inner<T>(child: ResidentChild<T>)
where
    T: LlmTransport + Summarizer + Send + 'static,
{
    // 先解构为局部变量:消息循环的 select 双臂各自持有独立权属,避免
    // 借权冲突(next 借 &mut rx / 取消臂借 token 副本)
    let ResidentChild {
        handle,
        label,
        event_sink,
        initial_prompt,
        model,
        transport_factory,
        registry,
        parent_id,
        parent_cancel,
        notify,
        stop,
        steer,
        mut rx,
        query_port,
        addressing,
        release: _,
        mut first,
        resumed_interrupted,
    } = child;

    let mut parts = match prepare_child_parts(&handle, &model, Some(&parent_id)).await {
        Ok(parts) => parts,
        Err(_) => {
            // 装配失败:记录 failed 并以失败通知收口(父会话必须知道委派没成)
            set_status(&registry, &handle.session_id, "failed");
            let (text, source) = settlement_notice(&handle.session_id, "error", None);
            notify.notify(&parent_id, text, source).await;
            return;
        }
    };
    // 子代理自己发起的 shell job 结算通知投回自己(运行中 steer /
    // 空闲续话;steer/tx 已随驻留记录登记,路由按注册表现场判定)
    parts.job_notify = Some(Arc::new(ChildJobNotify {
        self_id: handle.session_id.clone(),
        registry: registry.clone(),
    }));
    if !initial_prompt.is_empty() {
        // descriptor 标记(重启恢复的判据;重挂形态不重写)
        commit_child_marker(
            &parts.log,
            "subagent/descriptor",
            json!({
                "parentSessionId": parent_id,
                "label": label,
                "prompt": initial_prompt,
                "mode": "continuable",
            }),
        );
    }
    // 重挂形态的引擎历史已由 prepare 从现文件重建(空日志重复写缺陷
    // 的遗留 re-load 已删:对已装载历史的日志再 append 恒被守卫拒绝)
    let transport = match (transport_factory)() {
        Ok(t) => t,
        Err(_) => {
            set_status(&registry, &handle.session_id, "failed");
            let (text, source) = settlement_notice(&handle.session_id, "error", None);
            notify.notify(&parent_id, text, source).await;
            return;
        }
    };
    let mut gate = InvariantGate::new(transport, Arc::clone(&parts.log));
    let mut engine = LoopEngine::new(parts.header.clone(), Arc::clone(&parts.log));
    // 中途插话:引擎 step 边界认领(steer 语义)
    engine.set_steer_buf(Arc::clone(&steer));

    // 离线收件箱冲账:重挂会话的日志里可能有未消费 pending(本子代理
    // 离线期间经阶梯落档的消息)。按序提取为前置输入(优先于续话通道
    // 新消息);每条 turn 的 user/message 落档后紧跟 consumed——顺序
    // 不可反:先落 consumed 再输入的崩溃窗口会丢消息,现顺序的崩溃
    // 窗口(输入已落档、consumed 未落)= 下次重挂重投一次 =
    // at-least-once,窄且代理侧幂等消化
    let mut inbox: VecDeque<(String, Option<Value>, String)> = {
        let log = parts.log.lock().unwrap_or_else(|p| p.into_inner());
        extract_inbox_pending(&log)
            .into_iter()
            .map(|(id, text, source)| (text, source, id))
            .collect()
    };

    // 重挂且中断:只落 settled 标记(下次扫描视为已结),**不投父通知**
    // ——通知经 Notice 泵即变父会话 turn 输入,空闲驱动认领 = 无用户
    // 确认就续跑/重跑上次中断的工作(退出时在跑的子代理每次重开必被
    // 重放)。中断痕迹留在子日志:冷修复的合成失败结果 + 本标记,父
    // 模型经 list_agents 可见 idle 态,用户让继续再 send_message
    if resumed_interrupted {
        commit_child_marker(
            &parts.log,
            "subagent/settled",
            json!({ "stopReason": "resumed" }),
        );
    }

    'resident: loop {
        // 空闲等待:取下一条消息(初始 prompt → 收件箱冲账 → 续话通道)。
        // 父取消 → 静默退出(不通知——唤醒已取消的父会话只会凭空起
        // turn;子会话日志即持久记录)
        let next_msg = {
            let get_next = async {
                if let Some(msg) = first.take() {
                    return Some((msg, None));
                }
                if let Some((text, source, id)) = inbox.pop_front() {
                    return Some((ChildMsg::Prompt { text, source }, Some(id)));
                }
                rx.recv().await.map(|m| (m, None))
            };
            tokio::select! {
                msg = get_next => match msg {
                    Some(next) => next,
                    None => {
                        // 唯一续话 sender 已 drop:注册表摘记录(会话终结)
                        // 或 terminate_agent(terminated)——后者落
                        // terminated settled,重启不复活(resumable 判据)
                        if is_terminated(&registry, &handle.session_id) {
                            set_status(&registry, &handle.session_id, "terminated");
                            commit_child_marker(
                                &parts.log,
                                "subagent/settled",
                                json!({ "stopReason": "terminated" }),
                            );
                        }
                        break 'resident;
                    }
                },
                _ = parent_cancel.cancelled() => {
                    set_status(&registry, &handle.session_id, "cancelled");
                    commit_child_marker(
                        &parts.log,

                        "subagent/settled",
                        json!({ "stopReason": "aborted" }),
                    );
                    break 'resident;
                }
            }
        };
        // 内层:连续执行队列(本条 + 期间经 steer/续话到达的)
        let mut queue: VecDeque<(ChildMsg, Option<String>)> = VecDeque::new();
        queue.push_back(next_msg);
        while let Some((msg, inbox_id)) = queue.pop_front() {
            let ChildMsg::Prompt { text, source } = msg;
            if parent_cancel.is_cancelled() {
                set_status(&registry, &handle.session_id, "cancelled");
                commit_child_marker(
                    &parts.log,
                    "subagent/settled",
                    json!({ "stopReason": "aborted" }),
                );
                break 'resident;
            }
            set_status(&registry, &handle.session_id, "running");
            // 续话通道的输入染色(与 steer 路径同构:引擎在首步真实用户
            // 消息落档时消费 pending_input_source)
            engine.set_input_source(source);

            // 每 turn 一枚令牌 + 随令牌重建工具集:interrupt 只停当前 turn,
            // 运行中 bash 随令牌中断,子代理保持驻留
            let turn_token = CancelToken::new();
            engine.set_cancel(turn_token.clone());
            let mut tools = match build_child_tools(ChildToolContext {
                turn_token: &turn_token,
                parts: &parts,
                query_port: query_port.clone(),
                session_id: &handle.session_id,
                addressing: addressing.clone(),
                self_label: Some(&label),
            }) {
                Ok(t) => t,
                Err(_) => {
                    set_status(&registry, &handle.session_id, "failed");
                    let (text, source) = settlement_notice(&handle.session_id, "error", None);
                    notify.notify(&parent_id, text, source).await;
                    return;
                }
            };
            let interrupt = {
                let stop = Arc::clone(&stop);
                async move {
                    stop.notified().await;
                }
            };
            let outcome = run_child_turn(
                &mut engine,
                &mut gate,
                &mut tools,
                &text,
                &turn_token,
                &parent_cancel,
                interrupt,
                &handle.session_id,
                event_sink.as_ref(),
            )
            .await;

            // 父取消级联 → 静默退出(不投通知)
            if parent_cancel.is_cancelled() {
                set_status(&registry, &handle.session_id, "cancelled");
                commit_child_marker(
                    &parts.log,
                    "subagent/settled",
                    json!({ "stopReason": "aborted" }),
                );
                break 'resident;
            }

            // 终止(terminate_agent):当前 turn 收尾后不再回 idle——落
            // terminated settled(重启不复活),不投结算通知
            if is_terminated(&registry, &handle.session_id) {
                set_status(&registry, &handle.session_id, "terminated");
                commit_child_marker(
                    &parts.log,
                    "subagent/settled",
                    json!({ "stopReason": "terminated" }),
                );
                break 'resident;
            }

            // 结算:状态 + 通知(成功/idle 可续话;cancelled=aborted 亦可续话;
            // 失败=error。全部通知变体,父会话决定下一步)
            let (status, stop_reason, closing) = match &outcome {
                Ok(o) => ("idle", "completed", Some(o.assistant_message.clone())),
                Err(LoopError::Cancelled) => ("idle", "aborted", None),
                Err(_) => ("failed", "error", None),
            };
            set_status(&registry, &handle.session_id, status);
            let (text, source) =
                settlement_notice(&handle.session_id, stop_reason, closing.as_deref());
            notify.notify(&parent_id, text, source).await;
            commit_child_marker(
                &parts.log,
                "subagent/settled",
                json!({ "stopReason": stop_reason }),
            );
            // 收件箱条目收尾:turn 的 user/message 已落档,紧跟 consumed
            // 标记(顺序语义见 extract_inbox_pending 文档)
            if let Some(id) = inbox_id {
                commit_child_marker(&parts.log, "agent/inbox/consumed", json!({ "id": id }));
            }

            // 排干运行中未及消费的插话(parked messages 依次续跑),
            // 再收续话通道;两者皆空 → 回 idle 等待
            {
                let mut b = steer.lock().unwrap_or_else(|p| p.into_inner());
                while let Some(s) = b.pop_front() {
                    queue.push_back((
                        ChildMsg::Prompt {
                            text: s.text,
                            source: s.source,
                        },
                        None,
                    ));
                }
            }
            if let Ok(msg) = rx.try_recv() {
                queue.push_back((msg, None));
            }
        }
    }
}

impl<T> ToolPort for SubagentTool<T>
where
    T: LlmTransport + Summarizer + Send + 'static,
{
    /// 模型面按接口分形态(行为承诺与接口一致):
    /// - 通知 port 在场 → 后台默认形态(description/prompt/
    ///   run_in_background;send_message 语义句已真实成立)
    /// - 缺席(CLI/单测)→ 同步形态(维持既有 task 参数与描述)
    fn specs(&self) -> Vec<Value> {
        if self.notify.is_some() {
            vec![json!({
                "type": "function",
                "function": {
                    "name": "subagent",
                    "description": "Delegate a self-contained task to a subagent (a separate agent that works in its own context) to offload focused, independent work — research, a scoped implementation, an analysis — so it does not consume this conversation's context. The subagent returns its result, not its intermediate steps. Give it a complete, standalone prompt: it does not see this conversation. This tool runs in the background by default, immediately returns a durable subagent id, and keeps the child conversation available for later turns. When that run settles, the runtime sends the parent a notice containing its outcome and any final assistant message; `send_message` steers the child's nearest step while it is running and starts a turn while it is idle. Set `run_in_background: false` only when your next action depends on receiving the result.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "description": { "type": "string", "description": "A short (3-5 word) description of the delegated task, for display." },
                            "prompt": { "type": "string", "description": "The complete, self-contained task for the subagent. It does not share this conversation's context, so include everything it needs." },
                            "run_in_background": { "type": "boolean", "description": "Whether to run in the background and return a durable subagent id immediately. Defaults to true. Set false to wait for the result when your next action depends on it." }
                        },
                        "required": ["description", "prompt"]
                    },
                },
            })]
        } else {
            vec![json!({
                "type": "function",
                "function": {
                    "name": "subagent",
                    "description": "Run a task in an isolated subagent: its own turn loop, its own sandboxed workspace (a subdirectory of the current workspace), its own session log. Returns the subagent's final report. Use for self-contained subtasks.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "task": { "type": "string", "description": "Complete task description for the subagent" }
                        },
                        "required": ["task"],
                    },
                },
            })]
        }
    }

    async fn execute(&mut self, call: &ToolCallRequest) -> ToolOutput {
        if call.name != "subagent" {
            return ToolOutput {
                output: format!("unknown tool: {}", call.name),
                success: false,
                ..Default::default()
            };
        }
        let arguments: Value = if let Some(s) = call.arguments.as_str() {
            serde_json::from_str(s).unwrap_or(json!({}))
        } else {
            call.arguments.clone()
        };
        // 后台形态(通知 port 在场):description/prompt/run_in_background,
        // 默认后台
        if self.notify.is_some() {
            let Some(prompt) = arguments["prompt"].as_str() else {
                return ToolOutput {
                    output: "subagent requires arguments.prompt (string)".into(),
                    success: false,
                    ..Default::default()
                };
            };
            let label = arguments["description"].as_str().unwrap_or(prompt);
            let background = arguments["run_in_background"].as_bool().unwrap_or(true);
            if background {
                return self.start_background(label, prompt);
            }
            return self.run_foreground(prompt).await;
        }
        // 同步形态(无通知 port):task 参数,前台等结果
        let Some(task) = arguments["task"].as_str() else {
            return ToolOutput {
                output: "subagent requires arguments.task (string)".into(),
                success: false,
                ..Default::default()
            };
        };
        self.run_foreground(task).await
    }
}

/// 驻留活体投递(投递阶梯第 2 步的单注册表形态;宿主扫全库
/// jobs_sources 后对命中注册表调用,测试 port 直用):
/// 目标 `running` → steer 缓冲(引擎 step 边界认领,同一 turn);
/// `idle` → 续话通道开新 turn(source 随 ChildMsg 染色)。
/// turn 刚结束的竞态窗口消息落 steer 缓冲,由驻留循环在回 idle 前
/// 排干,不丢。找不到记录 / 无驻留通道 = Err(调用方继续阶梯后续步)。
pub fn deliver_to_registry_record(
    registry: &SubagentRegistry,
    target: &str,
    text: &str,
    source: Value,
) -> Result<DeliveryKind, String> {
    let kind = deliver_to_registry_record_inner(registry, target, text, source.clone());
    if kind.is_ok() {
        // 最近入站消息摘要(任务面板行副行;非持久)。对等消息的
        // source 已带 summary(消息内容首行,拼通知时算好)——直接取,
        // 避免吃进「Message from …:」头行把 80 字预算耗光;无 summary
        // 的通知(结算)回落文本首行
        let summary = source
            .get("summary")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                text.lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .chars()
                    .take(80)
                    .collect()
            });
        if !summary.is_empty()
            && let Ok(mut r) = registry.lock()
            && let Some(rec) = r.iter_mut().find(|s| s.session_id == target)
        {
            rec.last_message = Some(summary);
        }
        registry.changed();
    }
    kind
}

fn deliver_to_registry_record_inner(
    registry: &SubagentRegistry,
    target: &str,
    text: &str,
    source: Value,
) -> Result<DeliveryKind, String> {
    let Ok(r) = registry.lock() else {
        return Err("registry unavailable".into());
    };
    let Some(rec) = r.iter().find(|s| s.session_id == target) else {
        return Err(format!("agent {target} not found"));
    };
    if rec.status == "running"
        && let Some(buf) = &rec.steer
    {
        let mut b = buf.lock().unwrap_or_else(|p| p.into_inner());
        b.push_back(SteerInput {
            id: format!("steer-{}", uuid::Uuid::now_v7()),
            text: text.to_string(),
            images: Vec::new(),
            files: Vec::new(),
            source: Some(source),
        });
        return Ok(DeliveryKind::Steered);
    }
    match &rec.tx {
        Some(tx)
            if tx
                .send(ChildMsg::Prompt {
                    text: text.to_string(),
                    source: Some(source),
                })
                .is_ok() =>
        {
            Ok(DeliveryKind::TurnStarted)
        }
        _ => Err(format!("agent {target} is no longer accepting messages")),
    }
}

/// 子代理控制工具:`interrupt_agent`(打断当前 turn)/
/// `terminate_agent`(永久终止,不可逆)。send_message/list_agents 已并入
/// 对等寻址面(AgentMessageTool,经 AddressingPort 走宿主投递阶梯)。
pub struct SubagentControlTool {
    /// 注册表(与执行工具共享)
    pub registry: SubagentRegistry,
}

impl SubagentControlTool {
    /// 以共享注册表构建
    pub fn new(registry: SubagentRegistry) -> Self {
        Self { registry }
    }
}

impl ToolPort for SubagentControlTool {
    fn specs(&self) -> Vec<Value> {
        vec![
            json!({
                "type": "function",
                "function": {
                    "name": "interrupt_agent",
                    "description": "Request cancellation of a background agent's current turn by its agent id. The target is one of your direct children. Only the current turn stops: messages already queued for the agent stay parked until a later send_message, and the agent itself stays available for follow-ups. This call returns as soon as the stop request is accepted, so the target may keep running briefly; interrupting an agent that already finished is an accepted no-op.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "agent_id": { "type": "string", "description": "The agent id of the running agent to interrupt." }
                        },
                        "required": ["agent_id"]
                    },
                },
            }),
            json!({
                "type": "function",
                "function": {
                    "name": "terminate_agent",
                    "description": "Permanently terminate one of your direct background agents by its agent id. This is irreversible: the agent's pending inbox is dropped, it will not resume after a restart, and the only way to rerun its task is to delegate a new subagent. Use interrupt_agent instead when you only want to stop the current turn. Terminating an agent that already ended is an accepted no-op.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "agent_id": { "type": "string", "description": "The agent id of the background agent to terminate." }
                        },
                        "required": ["agent_id"]
                    },
                },
            }),
        ]
    }

    async fn execute(&mut self, call: &ToolCallRequest) -> ToolOutput {
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
        match call.name.as_str() {
            "interrupt_agent" => {
                let Some(agent_id) = arguments["agent_id"].as_str() else {
                    return fail("interrupt_agent requires arguments.agent_id (string)".into());
                };
                let Ok(r) = self.registry.lock() else {
                    return fail("registry unavailable".into());
                };
                // 打断已结束的子代理 = accepted no-op;只对正在跑的
                // turn 打断(Notify 仅唤醒在途等待者,idle 时本就无事可停)
                if let Some(rec) = r.iter().find(|s| s.session_id == agent_id)
                    && rec.status == "running"
                    && let Some(stop) = &rec.stop
                {
                    // notify_one 带许可语义:信号先于 interrupt future
                    // 注册的竞态窗口也不丢(notify_waiters 会丢)
                    stop.notify_one();
                }
                ToolOutput {
                    output: format!("interrupt requested for agent {agent_id}"),
                    success: true,
                    ..Default::default()
                }
            }
            "terminate_agent" => {
                let Some(agent_id) = arguments["agent_id"].as_str() else {
                    return fail("terminate_agent requires arguments.agent_id (string)".into());
                };
                // 置 terminated + drop 唯一续话 sender:驻留循环在 idle
                // 等待(recv None)或当前 turn 收尾即落 terminated settled
                // 退出(重启不复活);不可逆,重派 = 重新委派
                let stop = {
                    let Ok(mut r) = self.registry.lock() else {
                        return fail("registry unavailable".into());
                    };
                    match r.iter_mut().find(|s| s.session_id == agent_id) {
                        Some(rec) => {
                            if rec.status != "terminated" {
                                rec.status = "terminated".into();
                                rec.tx = None;
                                rec.ended_at = Some(now_ms());
                            }
                            rec.stop.clone()
                        }
                        None => return fail(format!("agent {agent_id} not found")),
                    }
                };
                // 运行中:打断当前 turn(收尾路径落 terminated settled;
                // notify_one 许可语义同 interrupt)
                if let Some(stop) = stop {
                    stop.notify_one();
                }
                ToolOutput {
                    output: format!("agent {agent_id} terminated (irreversible)"),
                    success: true,
                    ..Default::default()
                }
            }
            _ => fail(format!("unknown tool: {}", call.name)),
        }
    }
}

/// 仅供测试:子代理脚本事件的便捷构造(FakeProvider 脚本)
pub fn echo_events(text: &str) -> Vec<LlmEvent> {
    vec![LlmEvent::AssistantMessage(json!({ "content": text }))]
}

#[cfg(test)]
mod tests {
    use super::*;
    use liuma_llm::FakeProvider;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 录制型通知 port:记录 (parent, text, source) 三元组
    #[derive(Default, Clone)]
    struct RecordingNotify {
        calls: Arc<Mutex<Vec<(String, String, Value)>>>,
    }

    impl RecordingNotify {
        fn texts(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|(_, t, _)| t.clone())
                .collect()
        }
    }

    impl SettlementNotificationPort for RecordingNotify {
        fn notify(
            &self,
            parent_session: &str,
            text: String,
            source: Value,
        ) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.calls
                .lock()
                .unwrap()
                .push((parent_session.to_string(), text, source));
            Box::pin(std::future::ready(()))
        }
    }

    /// 录制型寻址 port:记录 (sender, target, text, source) 四元组
    /// (子面 send_message 的行为断言用)
    #[derive(Default, Clone)]
    struct RecordingAddressing {
        #[allow(clippy::type_complexity)]
        calls: Arc<Mutex<Vec<(String, String, String, Value)>>>,
    }

    impl AddressingPort for RecordingAddressing {
        fn deliver(
            &self,
            sender: &str,
            target: &str,
            text: &str,
            source: Value,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<DeliveryKind, String>> + Send + '_>>
        {
            self.calls.lock().unwrap().push((
                sender.to_string(),
                target.to_string(),
                text.to_string(),
                source,
            ));
            Box::pin(std::future::ready(Ok(DeliveryKind::Noticed)))
        }

        fn roster(&self, _sender: &str) -> Vec<RosterEntry> {
            Vec::new()
        }
    }

    /// 注册表路由型寻址 port:deliver = 驻留活体投递(与宿主阶梯第 2 步
    /// 同一实现 deliver_to_registry_record),roster = 注册表行。
    /// 父面 AgentMessageTool 的行为测试用。
    #[derive(Clone)]
    struct RegistryAddressing {
        registry: SubagentRegistry,
    }

    impl AddressingPort for RegistryAddressing {
        fn deliver(
            &self,
            _sender: &str,
            target: &str,
            text: &str,
            source: Value,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<DeliveryKind, String>> + Send + '_>>
        {
            let out = deliver_to_registry_record(&self.registry, target, text, source);
            Box::pin(std::future::ready(out))
        }

        fn roster(&self, _sender: &str) -> Vec<RosterEntry> {
            self.registry
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.background)
                .map(|r| RosterEntry {
                    session_id: r.session_id.clone(),
                    label: r.task.clone(),
                    relation: "child".into(),
                    status: r.status.clone(),
                })
                .collect()
        }
    }

    /// 脚本组工厂:第 n 次调用产一个携第 n 组脚本的 provider(一个子代理
    /// 一次调用;组内多条脚本 = 该子代理的多个 turn 依次消费)
    fn scripted_factory(groups: &[&[&str]]) -> (TransportFactory<FakeProvider>, Arc<AtomicUsize>) {
        let queue: Vec<Vec<Vec<LlmEvent>>> = groups
            .iter()
            .map(|group| {
                group
                    .iter()
                    .map(|text| vec![LlmEvent::AssistantMessage(json!({ "content": text }))])
                    .collect()
            })
            .collect();
        let built = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&built);
        let queue = Arc::new(Mutex::new(queue));
        (
            Arc::new(move || {
                let mut q = queue.lock().unwrap();
                let group = if q.is_empty() {
                    Vec::new()
                } else {
                    q.remove(0)
                };
                drop(q);
                counter.fetch_add(1, Ordering::SeqCst);
                let mut p = FakeProvider::new();
                for response in group {
                    p.then(response);
                }
                Ok(p)
            }),
            built,
        )
    }

    /// 工具调用起手脚本(子代理首个回复即发工具调用)
    fn tool_call_group(name: &str, arguments: Value) -> Vec<Vec<LlmEvent>> {
        vec![vec![LlmEvent::AssistantMessage(json!({
            "content": "", "tool_calls": [
                { "name": name, "arguments": arguments } ],
        }))]]
    }

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("liuma-sub47-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 轮询直到谓词命中(驻留任务异步结算;5s 预算——并行测试竞争下不 flaky)
    async fn wait_for(mut pred: impl FnMut() -> bool) {
        for _ in 0..1000 {
            if pred() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("condition not met within budget");
    }

    fn subagent_call(description: &str, prompt: &str) -> ToolCallRequest {
        ToolCallRequest {
            name: "subagent".into(),
            arguments: json!({ "description": description, "prompt": prompt }),
        }
    }

    fn child_events(session_path: &str) -> Vec<EventEnvelope> {
        liuma_host::persistence::jsonl::load_jsonl(std::path::Path::new(session_path))
            .expect("子会话日志必须可整份解码")
    }

    #[tokio::test]
    async fn background_delegation_returns_id_and_settles_with_notice() {
        let notify = RecordingNotify::default();
        let (factory, _built) = scripted_factory(&[&["hello from child"]]);
        let mut tool =
            SubagentTool::new(dir("bg"), factory, "m".into()).with_notify(Arc::new(notify.clone()));
        let out = ToolPort::execute(&mut tool, &subagent_call("Echo task", "say hi")).await;
        assert!(out.success, "{}", out.output);
        let session_id = out
            .output
            .strip_prefix("started subagent ")
            .expect("后台委派立即返回子代理 id")
            .to_string();
        assert!(!session_id.is_empty());

        // 结算通知:父会话、完成文案、closing message、染色载荷
        wait_for(|| notify.calls.lock().unwrap().len() == 1).await;
        let (parent, text, source) = notify.calls.lock().unwrap()[0].clone();
        assert_eq!(parent, "", "父会话 id 原样透传(未注入 parent_id 时为空)");
        assert!(
            text.contains("finished and will do no further work"),
            "{text}"
        );
        assert!(text.contains("Its closing message:"), "{text}");
        assert!(text.contains("hello from child"), "{text}");
        assert_eq!(source["kind"], "subagent-settled");
        assert_eq!(source["form"], "notice");
        assert_eq!(source["senderSessionId"], session_id);

        // 驻留:idle 可续话
        wait_for(|| {
            tool.registry
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.session_id == session_id && r.status == "idle")
        })
        .await;
    }

    #[tokio::test]
    async fn send_message_drives_continuation_turn() {
        let notify = RecordingNotify::default();
        // 同一子代理两个 turn:初始报告 + 续话回复(工厂一次调用,组内两条)
        let (factory, _built) = scripted_factory(&[&["first report", "second report after nudge"]]);
        let mut tool = SubagentTool::new(dir("cont"), factory, "m".into())
            .with_notify(Arc::new(notify.clone()));
        let out = ToolPort::execute(&mut tool, &subagent_call("Two turns", "start")).await;
        let session_id = out
            .output
            .strip_prefix("started subagent ")
            .unwrap()
            .to_string();
        wait_for(|| notify.calls.lock().unwrap().len() == 1).await;

        // send_message 立即投递(不阻塞等子代理回复)
        let mut msg_tool = AgentMessageTool::new(
            "",
            None,
            Arc::new(RegistryAddressing {
                registry: tool.registry.clone(),
            }),
        );
        let sent = ToolPort::execute(
            &mut msg_tool,
            &ToolCallRequest {
                name: "send_message".into(),
                arguments: json!({ "agent_id": session_id, "message": "continue please" }),
            },
        )
        .await;
        assert!(sent.success, "{}", sent.output);
        assert_eq!(
            sent.output,
            format!("message delivered to agent {session_id} (started a new turn)")
        );

        // 续话 turn 结算 → 第二次通知
        wait_for(|| notify.calls.lock().unwrap().len() == 2).await;
        let (_, text, _) = notify.calls.lock().unwrap()[1].clone();
        assert!(text.contains("second report after nudge"), "{text}");

        // 子会话日志确实跑了两个 turn
        let path = tool.registry.lock().unwrap()[0].session_path.clone();
        assert_eq!(
            child_events(&path)
                .iter()
                .filter(|e| e.r#type == "turn/start")
                .count(),
            2,
            "续话 = 子会话里的第二个 turn"
        );
    }

    #[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见 tool_loop.rs 文件头)
    #[tokio::test]
    async fn send_message_steers_running_child_mid_turn() {
        let notify = RecordingNotify::default();
        // 子代理卡在门控 bash(测试发完插话再放行,消费顺序确定);
        // 插话在其后的 step 边界被消费(同一 turn 内)
        let go_file = dir("steer-go").join("go");
        let group: Vec<Vec<LlmEvent>> = vec![
            tool_call_group(
                "bash",
                json!({
                    "command": format!(
                        "until [ -f \"{}\" ]; do sleep 0.05; done; echo released",
                        go_file.display()
                    ),
                    "description": "Gate on test signal",
                }),
            )
            .remove(0),
            vec![LlmEvent::AssistantMessage(
                json!({ "content": "final after steer" }),
            )],
        ];
        let factory: TransportFactory<FakeProvider> = {
            let slot = Arc::new(Mutex::new(Some(group)));
            Arc::new(move || {
                let g = slot.lock().unwrap().take().unwrap_or_default();
                let mut p = FakeProvider::new();
                for response in g {
                    p.then(response);
                }
                Ok(p)
            })
        };
        let mut tool = SubagentTool::new(dir("steer"), factory, "m".into())
            .with_notify(Arc::new(notify.clone()));
        let out = ToolPort::execute(
            &mut tool,
            &ToolCallRequest {
                name: "subagent".into(),
                arguments: json!({ "description": "Steer me", "prompt": "run long" }),
            },
        )
        .await;
        let session_id = out
            .output
            .strip_prefix("started subagent ")
            .unwrap()
            .to_string();
        wait_for(|| {
            tool.registry
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.session_id == session_id && r.status == "running")
        })
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        // 运行中插话 → steer 路由(立即投递确认)
        let mut msg_tool = AgentMessageTool::new(
            "",
            None,
            Arc::new(RegistryAddressing {
                registry: tool.registry.clone(),
            }),
        );
        let sent = ToolPort::execute(
            &mut msg_tool,
            &ToolCallRequest {
                name: "send_message".into(),
                arguments: json!({ "agent_id": session_id, "message": "change course now" }),
            },
        )
        .await;
        assert!(sent.success, "{}", sent.output);
        assert_eq!(
            sent.output,
            format!("message delivered to agent {session_id} (steered into its current turn)")
        );
        std::fs::write(&go_file, b"").unwrap();

        // 同一 turn 内消费:bash 放行后 step2 认领插话,最终回复出自脚本第 2 条
        wait_for(|| notify.calls.lock().unwrap().len() == 1).await;
        let (_, text, _) = notify.calls.lock().unwrap()[0].clone();
        assert!(text.contains("final after steer"), "{text}");
        let path = tool.registry.lock().unwrap()[0].session_path.clone();
        let events = child_events(&path);
        assert_eq!(
            events.iter().filter(|e| e.r#type == "turn/start").count(),
            1,
            "插话必须被同一 turn 消费(不另起 turn)"
        );
        // steer 的 user/message 落在 tool/result 之后(step 边界认领)
        let tool_result_seq = events
            .iter()
            .find(|e| e.r#type == "tool/result")
            .map(|e| e.seq)
            .unwrap_or_else(|| {
                panic!(
                    "bash 必有 tool/result,实际事件:{:?}",
                    events.iter().map(|e| e.r#type.as_str()).collect::<Vec<_>>()
                )
            });
        let steer_seq = events
            .iter()
            .find(|e| {
                e.r#type == "user/message"
                    && e.data["content"]
                        .as_str()
                        .map(|c| c.contains("change course"))
                        .unwrap_or(false)
            })
            .map(|e| e.seq)
            .expect("插话必须落为 user/message");
        assert!(steer_seq > tool_result_seq, "插话在下一 step 边界认领");
    }

    #[tokio::test]
    async fn child_sends_message_to_parent_mid_task() {
        let notify = RecordingNotify::default();
        let addressing = RecordingAddressing::default();
        // 子代理在任务中途主动回发父(脚本:先 send_message 再收尾;
        // 寻址走 AddressingPort,结算仍走 notify port)
        let mut group = tool_call_group(
            "send_message",
            json!({ "agent_id": "parent-1", "message": "interim finding" }),
        );
        group.push(vec![LlmEvent::AssistantMessage(
            json!({ "content": "final report" }),
        )]);
        let factory: TransportFactory<FakeProvider> = {
            let slot = Arc::new(Mutex::new(Some(group)));
            Arc::new(move || {
                let g = slot.lock().unwrap().take().unwrap_or_default();
                let mut p = FakeProvider::new();
                for response in g {
                    p.then(response);
                }
                Ok(p)
            })
        };
        let mut tool = SubagentTool::new(dir("childmsg"), factory, "m".into())
            .with_notify(Arc::new(notify.clone()))
            .with_addressing(Arc::new(addressing.clone()))
            .with_parent_id("parent-1");
        let out = ToolPort::execute(&mut tool, &subagent_call("Report home", "work")).await;
        let session_id = out
            .output
            .strip_prefix("started subagent ")
            .unwrap()
            .to_string();

        // 中途消息经寻址 port:sender/target/文本模板/染色
        wait_for(|| addressing.calls.lock().unwrap().len() == 1).await;
        let (sender, target, text, source) = addressing.calls.lock().unwrap()[0].clone();
        assert_eq!(sender, session_id);
        assert_eq!(target, "parent-1");
        assert!(text.contains("Message from agent"), "{text}");
        assert!(text.contains("interim finding"), "{text}");
        assert_eq!(source["kind"], "subagent-message");
        assert_eq!(source["senderSessionId"], session_id);
        // 结算通知仍走 notify port
        wait_for(|| notify.calls.lock().unwrap().len() == 1).await;
        let (_, text, _) = notify.calls.lock().unwrap()[0].clone();
        assert!(text.contains("final report"), "{text}");
    }

    #[tokio::test]
    async fn child_toolset_includes_todo_write() {
        // 工具面扩装:子代理可调 todo_write(全量 minus 递归的代表项)
        let notify = RecordingNotify::default();
        let mut group = tool_call_group(
            "todo_write",
            json!({ "todos": [ { "content": "step", "status": "completed" } ] }),
        );
        group.push(vec![LlmEvent::AssistantMessage(
            json!({ "content": "todo done" }),
        )]);
        let factory: TransportFactory<FakeProvider> = {
            let slot = Arc::new(Mutex::new(Some(group)));
            Arc::new(move || {
                let g = slot.lock().unwrap().take().unwrap_or_default();
                let mut p = FakeProvider::new();
                for response in g {
                    p.then(response);
                }
                Ok(p)
            })
        };
        let mut tool = SubagentTool::new(dir("tools"), factory, "m".into())
            .with_notify(Arc::new(notify.clone()))
            .with_parent_id("parent-1");
        let out = ToolPort::execute(&mut tool, &subagent_call("With tools", "work")).await;
        assert!(out.success, "{}", out.output);
        wait_for(|| notify.calls.lock().unwrap().len() == 1).await;
        let path = tool.registry.lock().unwrap()[0].session_path.clone();
        let events = child_events(&path);
        assert!(
            events
                .iter()
                .any(|e| e.r#type == "tool/call" && e.data["name"] == "todo_write"),
            "子代理必须能调 todo_write"
        );
        assert!(
            events
                .iter()
                .any(|e| e.r#type == "tool/result" && e.data["success"] == true),
            "todo_write 必须真实执行成功"
        );
    }

    #[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见 tool_loop.rs 文件头)
    #[tokio::test]
    async fn interrupt_agent_stops_current_turn_child_stays_contidable() {
        let notify = RecordingNotify::default();
        // 初始 turn 卡在长 bash;打断后子代理仍可续话(只停当前 turn)
        let mut group = tool_call_group(
            "bash",
            json!({ "command": "sleep 30 && echo never", "description": "Long bash" }),
        );
        group.push(vec![LlmEvent::AssistantMessage(
            json!({ "content": "after interrupt" }),
        )]);
        let factory: TransportFactory<FakeProvider> = {
            let slot = Arc::new(Mutex::new(Some(group)));
            Arc::new(move || {
                let g = slot.lock().unwrap().take().unwrap_or_default();
                let mut p = FakeProvider::new();
                for response in g {
                    p.then(response);
                }
                Ok(p)
            })
        };
        let mut tool = SubagentTool::new(dir("intr"), factory, "m".into())
            .with_notify(Arc::new(notify.clone()));
        let out = ToolPort::execute(
            &mut tool,
            &ToolCallRequest {
                name: "subagent".into(),
                arguments: json!({ "description": "Long task", "prompt": "run long bash" }),
            },
        )
        .await;
        let session_id = out
            .output
            .strip_prefix("started subagent ")
            .unwrap()
            .to_string();

        // 等 turn 进入长 bash 执行中
        wait_for(|| {
            tool.registry
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.session_id == session_id && r.status == "running")
        })
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        let mut control = SubagentControlTool::new(tool.registry.clone());
        let ack = ToolPort::execute(
            &mut control,
            &ToolCallRequest {
                name: "interrupt_agent".into(),
                arguments: json!({ "agent_id": session_id }),
            },
        )
        .await;
        assert!(ack.success, "{}", ack.output);
        assert_eq!(
            ack.output,
            format!("interrupt requested for agent {session_id}")
        );

        // 打断结算:aborted 变体 + 子代理转 idle(保持可续话)
        wait_for(|| notify.calls.lock().unwrap().len() == 1).await;
        let (_, text, _) = notify.calls.lock().unwrap()[0].clone();
        assert!(text.contains("was stopped before it finished"), "{text}");
        wait_for(|| {
            tool.registry
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.session_id == session_id && r.status == "idle")
        })
        .await;

        // 打断后续话仍可用(新 turn,新令牌)
        let mut msg_tool = AgentMessageTool::new(
            "",
            None,
            Arc::new(RegistryAddressing {
                registry: tool.registry.clone(),
            }),
        );
        let sent = ToolPort::execute(
            &mut msg_tool,
            &ToolCallRequest {
                name: "send_message".into(),
                arguments: json!({ "agent_id": session_id, "message": "wrap up" }),
            },
        )
        .await;
        assert!(sent.success, "{}", sent.output);
        wait_for(|| notify.calls.lock().unwrap().len() == 2).await;
        let (_, text, _) = notify.calls.lock().unwrap()[1].clone();
        assert!(text.contains("after interrupt"), "{text}");
    }

    #[tokio::test]
    async fn parent_cancel_silently_stops_child_without_notice() {
        let notify = RecordingNotify::default();
        let group = tool_call_group(
            "bash",
            json!({ "command": "sleep 30 && echo never", "description": "Long bash" }),
        );
        let factory: TransportFactory<FakeProvider> = {
            let slot = Arc::new(Mutex::new(Some(group)));
            Arc::new(move || {
                let g = slot.lock().unwrap().take().unwrap_or_default();
                let mut p = FakeProvider::new();
                for response in g {
                    p.then(response);
                }
                Ok(p)
            })
        };
        let cancel = CancelToken::new();
        let mut tool = SubagentTool::new(dir("pcancel"), factory, "m".into())
            .with_notify(Arc::new(notify.clone()))
            .with_cancel(cancel.clone());
        let out = ToolPort::execute(&mut tool, &subagent_call("Long", "run long")).await;
        let session_id = out
            .output
            .strip_prefix("started subagent ")
            .unwrap()
            .to_string();
        wait_for(|| {
            tool.registry
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.session_id == session_id && r.status == "running")
        })
        .await;
        // 父令牌触发 → 子静默退出(cancelled),不发结算通知
        cancel.cancel();
        wait_for(|| {
            tool.registry
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.session_id == session_id && r.status == "cancelled")
        })
        .await;
        assert!(
            notify.texts().is_empty(),
            "父取消不投通知:{:?}",
            notify.texts()
        );
    }

    #[tokio::test]
    async fn list_agents_lists_background_only() {
        let notify = RecordingNotify::default();
        let (factory, built) = scripted_factory(&[&["bg result"], &["fg result"]]);
        let mut tool = SubagentTool::new(dir("list"), factory, "m".into())
            .with_notify(Arc::new(notify.clone()));
        let bg = ToolPort::execute(&mut tool, &subagent_call("Bg task", "run bg")).await;
        let bg_id = bg
            .output
            .strip_prefix("started subagent ")
            .unwrap()
            .to_string();
        // 等驻留任务取走第 1 组传输,前台再取第 2 组(脚本组与传输一一对应)
        wait_for(|| built.load(Ordering::SeqCst) >= 1).await;
        // 前台一次性(run_in_background=false)
        let fg = ToolPort::execute(
            &mut tool,
            &ToolCallRequest {
                name: "subagent".into(),
                arguments: json!({ "description": "Fg task", "prompt": "run fg", "run_in_background": false }),
            },
        )
        .await;
        assert!(fg.success, "{}", fg.output);
        assert!(fg.output.contains("fg result"), "前台等结果");

        wait_for(|| notify.calls.lock().unwrap().len() == 1).await;
        let mut msg_tool = AgentMessageTool::new(
            "",
            None,
            Arc::new(RegistryAddressing {
                registry: tool.registry.clone(),
            }),
        );
        let list = ToolPort::execute(
            &mut msg_tool,
            &ToolCallRequest {
                name: "list_agents".into(),
                arguments: json!({}),
            },
        )
        .await;
        assert!(list.output.contains(&bg_id), "{}", list.output);
        assert!(
            list.output.contains("[child/idle] Bg task"),
            "{}",
            list.output
        );
        assert!(
            !list.output.contains("Fg task"),
            "前台一次性不列入清单:{}",
            list.output
        );
    }

    #[test]
    fn specs_adapt_to_notification_port() {
        let (factory, _built) = scripted_factory(&[]);
        // 无 port:同步形态(task 参数;无 run_in_background)
        let sync_tool = SubagentTool::<FakeProvider>::new(dir("s1"), factory.clone(), "m".into());
        let specs = ToolPort::specs(&sync_tool);
        assert_eq!(specs.len(), 1);
        assert!(
            specs[0]["function"]["parameters"]["properties"]
                .get("task")
                .is_some()
        );
        assert!(
            specs[0]["function"]["parameters"]["properties"]
                .get("run_in_background")
                .is_none()
        );

        // 有 port:后台形态(description/prompt/run_in_background;默认后台)
        let notify = RecordingNotify::default();
        let bg_tool = SubagentTool::<FakeProvider>::new(dir("s2"), factory, "m".into())
            .with_notify(Arc::new(notify));
        let specs = ToolPort::specs(&bg_tool);
        assert_eq!(specs.len(), 1);
        let props = specs[0]["function"]["parameters"]["properties"].clone();
        assert!(props.get("description").is_some());
        assert!(props.get("prompt").is_some());
        assert!(props.get("task").is_none());
        assert!(props.get("run_in_background").is_some());
        assert!(
            specs[0]["function"]["description"]
                .as_str()
                .unwrap()
                .contains("runs in the background by default")
        );
        // 对等寻址面:send_message 描述须含中途 steer 与可发范围句
        // (父/兄弟/子),list_agents 描述含 relation 句
        let msg_tool = AgentMessageTool::new(
            "",
            None,
            Arc::new(RegistryAddressing {
                registry: SubagentRegistry::default(),
            }),
        );
        let specs = ToolPort::specs(&msg_tool);
        let send_spec = specs
            .iter()
            .find(|s| s["function"]["name"] == "send_message")
            .unwrap();
        let desc = send_spec["function"]["description"].as_str().unwrap();
        assert!(desc.contains("steers its nearest step"), "{desc}");
        assert!(desc.contains("your direct parent"), "{desc}");
        assert!(desc.contains("sibling"), "{desc}");
        assert!(desc.contains("queued in its inbox"), "{desc}");
        let list_spec = specs
            .iter()
            .find(|s| s["function"]["name"] == "list_agents")
            .unwrap();
        assert!(
            list_spec["function"]["description"]
                .as_str()
                .unwrap()
                .contains("relation (parent, sibling, child)")
        );
    }

    #[test]
    fn settlement_notice_variants() {
        let (text, source) = settlement_notice("s-1", "completed", Some("done report"));
        assert!(text.contains("Background subagent s-1 finished"));
        assert!(text.contains("Its closing message:\n\ndone report"));
        assert_eq!(source["kind"], "subagent-settled");
        assert_eq!(source["senderSessionId"], "s-1");

        let (text, _) = settlement_notice("s-2", "aborted", None);
        assert!(text.contains("was stopped before it finished"));
        assert!(text.contains("It left no closing message."));

        let (text, _) = settlement_notice("s-3", "error", Some("  "));
        assert!(text.contains("failed before it finished"));
        assert!(
            text.contains("It left no closing message."),
            "空白 closing 视同无"
        );
    }

    /// 离线收件箱冲账:重挂会话从日志提取未消费 pending(有 pending 无
    /// consumed)作为前置 turn 输入;输入落档后紧跟 consumed。
    /// 已消费条目(a)不重投,未消费条目(b)恰好投一次。
    #[tokio::test]
    async fn inbox_pending_replayed_on_resume_and_consumed() {
        let notify = RecordingNotify::default();
        // 手工 seed 子会话日志:descriptor + pending(a) + pending(b) +
        // consumed(a)——期望重挂后只重投 b
        let root = dir("inbox");
        let child_dir = root.join(".liuma/subagents/seed-1");
        std::fs::create_dir_all(&child_dir).unwrap();
        let child_path = child_dir.join("session.jsonl");
        let mut events: Vec<EventEnvelope> = Vec::new();
        let mut seed = |ty: &str, data: Value| {
            let mut ev = EventEnvelope::new_ignorable(ty, now_ms(), data);
            ev.seq = events.len() as u64 + 1;
            events.push(ev);
        };
        seed(
            "subagent/descriptor",
            json!({ "parentSessionId": "parent-1", "label": "Seeded", "prompt": "seed prompt", "mode": "continuable" }),
        );
        seed(
            "agent/inbox/pending",
            json!({ "sender": "peer-x", "text": "already eaten", "id": "a", "source": null }),
        );
        seed(
            "agent/inbox/pending",
            json!({ "sender": "peer-x", "text": "offline message", "id": "b", "source": null }),
        );
        seed("agent/inbox/consumed", json!({ "id": "a" }));
        {
            use std::io::Write as _;
            let mut f = std::fs::File::create(&child_path).unwrap();
            for ev in &events {
                serde_json::to_writer(&mut f, ev).unwrap();
                f.write_all(b"\n").unwrap();
            }
        }
        let handle = SubagentSessionHandle {
            session_id: "seed-1".into(),
            session_path: child_path.clone(),
        };
        struct SeededFactory {
            handle: SubagentSessionHandle,
        }
        impl SessionFactory for SeededFactory {
            fn create_subagent(&self, _parent: &str) -> SubagentSessionHandle {
                self.handle.clone()
            }
            fn resumable_children(
                &self,
                _parent: &str,
            ) -> Vec<(SubagentSessionHandle, bool, String, String)> {
                vec![(
                    self.handle.clone(),
                    false,
                    "Seeded".into(),
                    "seed prompt".into(),
                )]
            }
        }
        let (factory, _built) = scripted_factory(&[&["replied after offline"]]);
        let tool = SubagentTool::new(root.clone(), factory, "m".into())
            .with_notify(Arc::new(notify.clone()))
            .with_session_factory(Arc::new(SeededFactory { handle }))
            .with_parent_id("parent-1");
        tool.resume_children();
        // 驻留起 → 冲账消费 pending(b) → turn 收尾落 consumed(b)
        wait_for(|| {
            child_events(&child_path.display().to_string())
                .iter()
                .any(|e| e.r#type == "agent/inbox/consumed" && e.data["id"] == "b")
        })
        .await;
        let events = child_events(&child_path.display().to_string());
        // b 作为 turn 输入落档(user/message);a 不重投
        assert!(
            events.iter().any(|e| e.r#type == "user/message"
                && e.data["content"]
                    .as_str()
                    .unwrap_or("")
                    .contains("offline message")),
            "未消费 pending 必须作为 turn 输入落档:{:?}",
            events.iter().map(|e| e.r#type.as_str()).collect::<Vec<_>>()
        );
        assert!(
            !events.iter().any(|e| e.r#type == "user/message"
                && e.data["content"]
                    .as_str()
                    .unwrap_or("")
                    .contains("already eaten")),
            "已消费 pending 不得重投"
        );
        // consumed(b) 恰一条(幂等)
        assert_eq!(
            events
                .iter()
                .filter(|e| e.r#type == "agent/inbox/consumed" && e.data["id"] == "b")
                .count(),
            1
        );
        // 结算通知仍投父(冲账 turn 也是 turn)
        wait_for(|| notify.calls.lock().unwrap().len() == 1).await;
    }

    /// terminate_agent(idle 态):置 terminated + drop 续话通道 → 驻留
    /// 循环 recv None 退出,落 settled terminated(重启不复活判据),
    /// **不投结算通知**(idle 终止无回合可结算)。
    #[tokio::test]
    async fn terminate_agent_idle_persists_terminated_marker() {
        let notify = RecordingNotify::default();
        let (factory, _built) = scripted_factory(&[&["b ready"]]);
        let mut tool = SubagentTool::new(dir("term-idle"), factory, "m".into())
            .with_notify(Arc::new(notify.clone()));
        let out = ToolPort::execute(&mut tool, &subagent_call("Term me", "run")).await;
        let session_id = out
            .output
            .strip_prefix("started subagent ")
            .unwrap()
            .to_string();
        wait_for(|| {
            tool.registry
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.session_id == session_id && r.status == "idle")
        })
        .await;

        let mut control = SubagentControlTool::new(tool.registry.clone());
        let ack = ToolPort::execute(
            &mut control,
            &ToolCallRequest {
                name: "terminate_agent".into(),
                arguments: json!({ "agent_id": session_id }),
            },
        )
        .await;
        assert!(ack.success, "{}", ack.output);
        // marker 由驻留循环异步落盘:直接轮询日志(状态置位是同步的,
        // 等状态会早于落盘读日志)
        let path = tool
            .registry
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.session_id == session_id)
            .map(|r| r.session_path.clone())
            .unwrap();
        wait_for(|| {
            child_events(&path)
                .iter()
                .any(|e| e.r#type == "subagent/settled" && e.data["stopReason"] == "terminated")
        })
        .await;
        // 子日志落 terminated settled(重启不复活判据)
        let events = child_events(&path);
        assert!(
            events
                .iter()
                .any(|e| e.r#type == "subagent/settled" && e.data["stopReason"] == "terminated"),
            "终止必须落 settled terminated:{:?}",
            events.iter().map(|e| e.r#type.as_str()).collect::<Vec<_>>()
        );
        // idle 终止无回合可结算:不再追加通知
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        assert_eq!(
            notify.calls.lock().unwrap().len(),
            1,
            "idle 终止不投结算通知"
        );
        // 终止后续话通道已 drop:再投递 = 错误
        let mut msg_tool = AgentMessageTool::new(
            "",
            None,
            Arc::new(RegistryAddressing {
                registry: tool.registry.clone(),
            }),
        );
        let sent = ToolPort::execute(
            &mut msg_tool,
            &ToolCallRequest {
                name: "send_message".into(),
                arguments: json!({ "agent_id": session_id, "message": "anyone?" }),
            },
        )
        .await;
        assert!(!sent.success, "终止后不可再投递:{}", sent.output);
    }

    /// terminate_agent(running 态):打断当前 turn → 收尾落 terminated
    /// settled,不投结算通知(终止不是一次可续话的结算)。
    #[cfg(unix)]
    #[tokio::test]
    async fn terminate_agent_running_lands_terminated_marker() {
        let notify = RecordingNotify::default();
        let go_file = dir("term-run").join("go");
        let group: Vec<Vec<LlmEvent>> = vec![
            tool_call_group(
                "bash",
                json!({
                    "command": format!(
                        "until [ -f \"{}\" ]; do sleep 0.05; done; echo released",
                        go_file.display()
                    ),
                    "description": "Gate on test signal",
                }),
            )
            .remove(0),
            vec![LlmEvent::AssistantMessage(
                json!({ "content": "never reached" }),
            )],
        ];
        let factory: TransportFactory<FakeProvider> = {
            let slot = Arc::new(Mutex::new(Some(group)));
            Arc::new(move || {
                let g = slot.lock().unwrap().take().unwrap_or_default();
                let mut p = FakeProvider::new();
                for response in g {
                    p.then(response);
                }
                Ok(p)
            })
        };
        let mut tool = SubagentTool::new(dir("term-run2"), factory, "m".into())
            .with_notify(Arc::new(notify.clone()));
        let out = ToolPort::execute(
            &mut tool,
            &ToolCallRequest {
                name: "subagent".into(),
                arguments: json!({ "description": "Term running", "prompt": "run long" }),
            },
        )
        .await;
        let session_id = out
            .output
            .strip_prefix("started subagent ")
            .unwrap()
            .to_string();
        wait_for(|| {
            tool.registry
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.session_id == session_id && r.status == "running")
        })
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        let mut control = SubagentControlTool::new(tool.registry.clone());
        let ack = ToolPort::execute(
            &mut control,
            &ToolCallRequest {
                name: "terminate_agent".into(),
                arguments: json!({ "agent_id": session_id }),
            },
        )
        .await;
        assert!(ack.success, "{}", ack.output);
        std::fs::write(&go_file, b"").unwrap();
        let path = tool
            .registry
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.session_id == session_id)
            .map(|r| r.session_path.clone())
            .unwrap();
        // marker 由收尾路径异步落盘:轮询日志
        wait_for(|| {
            child_events(&path)
                .iter()
                .any(|e| e.r#type == "subagent/settled" && e.data["stopReason"] == "terminated")
        })
        .await;
        let events = child_events(&path);
        assert!(
            events
                .iter()
                .any(|e| e.r#type == "subagent/settled" && e.data["stopReason"] == "terminated"),
            "运行中终止收尾落 terminated settled"
        );
        assert!(
            !events.iter().any(|e| {
                e.r#type == "assistant/message"
                    && e.data["content"]
                        .as_str()
                        .is_some_and(|c| c.contains("never reached"))
            }),
            "终止后不得再跑后续 step"
        );
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        assert_eq!(
            notify.calls.lock().unwrap().len(),
            0,
            "运行中终止不投结算通知"
        );
    }
}
