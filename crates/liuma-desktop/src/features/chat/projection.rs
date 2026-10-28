//! 客方事件 → 节点流投影(纯函数;vendor `ui-conversation` ChatView 的
//! 简化版)。节点按稳定 key 增量更新(append/定稿/配对),顺序 = 事件序。
//!
//! 事件形状源 `liuma-core/src/translate.rs`(客方 camelCase);未列类型忽略。

use crate::kits::i18n::t;
use liuma_core::proto::SessionEvent;
use serde_json::Value;

/// 工具执行态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolState {
    /// 执行中
    Running,
    /// 成功
    Done,
    /// 失败
    Error,
    /// 被中断(回合中止时仍未落定;渲染层 amber 状态点,
    /// 不再残留运行扫光)
    Stopped,
}

/// todo 条目(todo/write 投影;渲染在 todo_dock)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoItem {
    /// 内容
    pub content: String,
    /// completed / in_progress / pending
    pub status: String,
}

/// 通告行类别(渲染期词典化;locale-owned:宿主 detail 逐字直显)
#[derive(Debug, Clone, PartialEq)]
pub enum NoticeKind {
    /// 回合错误(detail = 宿主错误原文;缺席时渲染层兜底「未知错误」)
    TurnError { detail: Option<String> },
    /// 压缩失败(text = 宿主 settlement 原文直显)
    Compaction { text: String },
    /// 本地通告(shell 直推的操作反馈;构建期定稿文案,不随档重渲)
    Local { text: String },
}

/// 本回合交付单元(present 宣告 ∪ diff 写入的并集投影)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deliverable {
    /// 文件路径(模型面;渲染时按工作区根展开)
    pub path: String,
    /// 模型宣告的一句描述(交付层语义「是什么」);None = 副标题
    /// 依次回退变更统计 / 大写扩展名
    pub description: Option<String>,
    /// 变更层统计(最后一次编辑的 +added/-removed;变更层语义
    /// 「改了多少」,diff 派生与后随编辑更新,宣告不携带)
    pub stat: Option<(u64, u64)>,
    /// 是否经模型 present 宣告(渲染分层:宣告 = 60px 交付卡,
    /// 未宣告变更 = 26px 紧凑行)
    pub announced: bool,
}

/// 消息流节点(稳定 key;同 key 增量替换)
#[derive(Debug, Clone, PartialEq)]
pub enum ChatNode {
    /// 用户消息(user/message 与 agent/inbox/spliced 插入)
    User {
        /// 稳定 key(user:<seq> / user:<id>)
        key: String,
        /// 文本
        text: String,
        /// 图片块(attachment 引用数组;空 = 纯文本)
        images: Vec<Value>,
        /// 文件块(attachment 引用数组;空 = 无文件)
        files: Vec<Value>,
        /// 消息时刻(信封毫秒;操作行时间戳,0 = 缺席)
        time: i64,
    },
    /// 注入上下文(user/message + source.kind ≠ "user";4a 完整溯源模型):
    /// 模型实际收到的非用户来源消息(AGENTS.md / @session 快照),折叠渲染
    /// 为「注入行」。
    Context {
        /// 稳定 key(ctx:<seq>)
        key: String,
        /// 模型可见注入文本(content)
        content: String,
        /// 来源染色(source;kind/form + producer 扩展字段)
        source: Value,
    },
    /// 助手消息(流式增量 → assistant/message 定稿)
    Assistant {
        /// 稳定 key(a:<turn>:<step>)
        key: String,
        /// 正文
        text: String,
        /// 正文版本号(text 每次突变 +1;流式 drive 帧首 O(1) 判「未变」
        /// ——稳态帧原实现逐节点全文 memcmp,大会话每帧 O(全部正文字节))
        text_ver: u32,
        /// 思考内容(Think 折叠行,点击展开)
        reasoning: String,
        /// 流式进行中
        streaming: bool,
        /// 定稿用量(durationMs/ttftMs/outputTokens)
        usage: Option<Value>,
        /// 持久消息 id(assistant/message 落档;反馈定位 key)
        message_id: String,
    },
    /// 工具调用(tool/call → tool/result 配对)
    Tool {
        /// 稳定 key(call:<callId>)
        key: String,
        /// 工具名
        name: String,
        /// 折叠摘要(工具名感知键序;见 [`summarize_call`])
        summary: String,
        /// 执行态
        state: ToolState,
        /// 原始参数(JSON 字符串)
        arguments: String,
        /// 输出(配对 tool/result)
        output: Option<String>,
        /// 渲染意图(call 侧 = 运行中意图,result 侧 = 已应用事实;
        /// 无视图 = 通用 IN/OUT 卡。窄化在 ui/toolcard)
        view: Option<Value>,
        /// 结果图片(MCP 图片桥;image 引用块数组,缺席 = 无图)
        images: Vec<Value>,
    },
    /// 回合收尾(turn/end)
    TurnTail {
        /// 稳定 key(turn-end:<seq>)
        key: String,
        /// 中断(true)/正常完成
        aborted: bool,
        /// 轮号(turn_usage 桶的查询键;与 Translator 计数同源)
        turn: u64,
        /// 收尾时刻(轮尾时钟文本;信封毫秒)
        ended_ms: i64,
        /// 轮墙钟用时(turn/end − turn/start;0 = 起始未知)
        run_ms: i64,
        /// 本回合交付宣告(present;去重保序,后到覆盖 description)
        deliverables: Vec<Deliverable>,
    },
    /// 通告行(turn/error 等异常落档的可视化)
    Notice {
        /// 稳定 key
        key: String,
        /// 类别(渲染期词典化;宿主 detail 逐字)
        kind: NoticeKind,
    },
    /// 压缩标记行(compaction/summary 落档;单行统计 + 点击定位台账
    /// 记录——摘要全文的归宿是检查器,聊天流只留事实)
    Compaction {
        /// 稳定 key(cpt:<seq>;seq 即台账 compacted 记录的定位锚)
        key: String,
        /// 折叠条数(None = 载荷无统计,标题退「上下文已压缩」)
        items: Option<u64>,
        /// 折叠前缀估算 token
        tokens: Option<u64>,
        /// 价值裁定**已生效**裁掉的条数(0/缺席 = 未裁)
        pruned: Option<u64>,
    },
    /// 压缩空反馈行(compaction/error kind=empty;渲染期词典化,
    /// 中性别红。宿主载荷是英文常量,事件原文不动)
    CompactStatus {
        /// 稳定 key(cpt-empty:<seq>)
        key: String,
    },
    /// 计划归档卡(plan/submitted 落档;批准/取消仅更新状态)
    Plan {
        /// 稳定 key(plan:<seq>)
        key: String,
        /// 计划正文(markdown)
        plan: String,
        status: PlanStatus,
    },
    /// 模型请求重试行(llm/retry 落档;
    /// 状态随 llm/retry-started / 取消收尾演化)
    Retry {
        /// 稳定 key(retry:<seq>)
        key: String,
        /// 第几次重试(1 起)
        retry: u32,
        /// 重试上限
        max_retries: u32,
        /// 本次退避时长(毫秒;倒计时源)
        delay_ms: u64,
        /// 失败分类码(TRANSPORT/TIMEOUT/SERVER/RATE_LIMIT/EMPTY_RESPONSE)
        code: String,
        /// 失败摘要(展开详情「失败原因」)
        message: String,
        state: RetryState,
    },
}

/// 重试行状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryState {
    /// 等待退避(直播中带倒计时;重放为静态)
    Waiting,
    /// 退避结束、重发开始(llm/retry-started)
    Started,
    /// 退避中被取消(取消收尾且未见 started)
    Cancelled,
}

impl ChatNode {
    /// 稳定 key
    pub fn key(&self) -> &str {
        match self {
            ChatNode::User { key, .. }
            | ChatNode::Assistant { key, .. }
            | ChatNode::Tool { key, .. }
            | ChatNode::TurnTail { key, .. }
            | ChatNode::Context { key, .. }
            | ChatNode::Notice { key, .. }
            | ChatNode::Compaction { key, .. }
            | ChatNode::CompactStatus { key, .. }
            | ChatNode::Plan { key, .. }
            | ChatNode::Retry { key, .. } => key,
        }
    }
}

/// 队列预览单行化截断上限(展示面防巨型排队文本逐帧全量整形)
const QUEUE_PREVIEW_MAX_CHARS: usize = 240;

/// session/queue 帧 items → 队列投影(preview = text 块拼接;
/// 全部块均为文本才可编辑)
pub fn parse_queue_items(items: Vec<Value>) -> Vec<QueueEntry> {
    items
        .iter()
        .filter_map(|item| {
            let id = item["id"].as_str()?.to_string();
            let placement = match item["placement"].as_str() {
                Some("steering") => QueuePlacement::Steering,
                _ => QueuePlacement::Queued,
            };
            let blocks = item["message"]["content"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut raw = String::new();
            let mut all_text = true;
            for b in &blocks {
                match b["type"].as_str() {
                    Some("text") => raw.push_str(b["text"].as_str().unwrap_or_default()),
                    _ => all_text = false,
                }
            }
            // 显示面单行化:queue dock 行是固定单行设计,truncate 对硬换行
            // 失效(逐 \n 出行、各行皆短则不截断),多行原文会溢出行框被相邻
            // 层盖住;编辑面 text 保留原文不动
            let preview = one_line(&raw, QUEUE_PREVIEW_MAX_CHARS);
            let text = all_text.then_some(raw);
            Some(QueueEntry {
                id,
                placement,
                preview,
                text,
            })
        })
        .collect()
}

/// 队列/插队条目(session/queue 权威快照的 UI 投影;非持久节点)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEntry {
    /// 条目 id(host 定位变更用)
    pub id: String,
    /// queued = 待运行(next-turn)/ steering = 中途插队(立即投递)
    pub placement: QueuePlacement,
    /// 文本预览(单行化:空白折叠 + 截断;展示面专用)
    pub preview: String,
    /// 纯文本(可编辑;含非文本块 = 不可编辑)
    pub text: Option<String>,
}

impl QueueEntry {
    /// 展示面完整文本:text(原始拼接)优先;含非文本块的条目不可编辑,
    /// 回落单行 preview(此类条目本就无完整原文可展)
    pub fn full_text(&self) -> &str {
        self.text.as_deref().unwrap_or(&self.preview)
    }
}

/// 乐观用户消息(prompt RPC 回执落地、`user/message` 事件未到)。
/// 直播态:重放不记,宿主重同步(subscribed)清空
#[derive(Debug, Clone)]
pub struct PendingUser {
    /// 宿主预分配消息 id(RPC 回执携带;user/message 载荷同 id → 原位替换)
    pub id: String,
    /// 发送文本(渲染与折叠用)
    pub text: String,
}

/// 队列条目落位
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueuePlacement {
    /// 待运行(next-turn)
    Queued,
    /// 中途插队(立即投递)
    Steering,
}

/// 计划归档卡状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStatus {
    /// 调用派生(tool/call 落卡)尚未见 plan/submitted 确认;渲染同
    /// Pending,失败收口后转 Failed
    Draft,
    /// 已提交待批
    Pending,
    /// 已批准
    Approved,
    /// 已拒绝(留在 plan 模式;反馈已回传模型修订重提)
    Declined,
    /// 已取消/驳回
    Cancelled,
    /// 提交失败(校验/模式拒绝;失败的提交计划正文仍可见)
    Failed,
}

/// 摘要长度估算:分母 = 折叠前缀 token 的 1/4,夹在 [800, 6000] 字符。
/// 摘要(八节 checkpoint)与折叠前缀同量级但远短于它,直接用前缀做
/// 分母会让条几乎不动;这是**估算**,故百分比封顶 99% 由 done 宣告
/// 完成,悬停明细另给真实字符数。
const SUMMARY_CHARS_PER_PREFIX_TOKENS: f64 = 0.25;
/// 分母下限(字符)
const SUMMARY_TARGET_MIN_CHARS: f64 = 800.;
/// 分母上限(字符)
const SUMMARY_TARGET_MAX_CHARS: f64 = 6000.;

/// 压缩进度快照(compaction/progress 载荷;UI 进度条的数据面)
#[derive(Debug, Clone, PartialEq)]
pub struct CompactProgress {
    /// 相位:judge | summarize | commit | done | failed
    pub phase: String,
    /// 真实已生成正文字符数(单调不减)
    pub generated_chars: u64,
    /// 已耗时(毫秒)
    pub elapsed_ms: u64,
    /// 折叠前缀 token 估算(进度分母的来源)
    pub estimated_tokens: u64,
    /// 本次折叠的终点 seq(载荷 throughSeq;一次折叠内恒定,跨次不同)
    /// ——填充的值过渡按它作键:新一次压缩从零起涨,不退着走
    pub through_seq: u64,
}

impl CompactProgress {
    /// 是否终局相位(UI 据它清 / 收尾)
    pub fn terminal(&self) -> bool {
        matches!(self.phase.as_str(), "done" | "failed")
    }

    /// 进度百分比(0..=99;分子 = 真实字符数,分母 = 估算目标长度)
    pub fn percent(&self) -> f32 {
        let target = (self.estimated_tokens as f64 * SUMMARY_CHARS_PER_PREFIX_TOKENS)
            .clamp(SUMMARY_TARGET_MIN_CHARS, SUMMARY_TARGET_MAX_CHARS);
        ((self.generated_chars as f64 / target) * 100.0).clamp(0.0, 99.0) as f32
    }
}

/// 单会话投影状态
#[derive(Debug, Default)]
pub struct ChatState {
    /// 顺序节点流
    pub nodes: Vec<ChatNode>,
    /// 节点内容版本(任一节点的文本/思考就地变更即递增)。节点数与
    /// 末键之外的第三类变更:流式定稿把已有节点从空文本改到有文本,
    /// 行槽派生(答复切组/可见性)依赖文本内容,
    /// 行槽缓存签名须随内容变化失效
    pub content_ver: u64,
    /// todo 列表(todo/write 全量替换)
    pub todos: Vec<TodoItem>,
    /// 本地 running(turn/start~turn/end)
    pub running: bool,
    /// 队列/插队条目(session/queue 权威快照整体替换)
    pub queue: Vec<QueueEntry>,
    /// 计划模式(plan/mode)
    pub plan_mode: bool,
    /// 手动压缩进行中(/compact RPC 受理置位;compaction/summary|error
    /// 终局事件清位。瞬态不入节点:重放不重现进行态,只现终局行)
    pub compact_running: bool,
    /// 手动压缩排队中(回合进行时受理;驱动仅在 turn 间隙取压缩任务,
    /// turn/end 事件晋升为进行态)
    pub compact_queued: bool,
    /// 压缩进度(compaction/progress;瞬态:重放不重现,终局清位)
    pub compact_progress: Option<CompactProgress>,
    /// 刚完成压缩的标记行 key 与时刻(完成闪:标记行入场时条走满 +
    /// SUCCESS 闪,年龄门控——虚拟化重挂不重放)。纯 UI 元数据。
    pub compact_settled: Option<(String, std::time::Instant)>,
    /// 当前 turn 已收到的交付宣告(去重保序;turn/end 挂 TurnTail 产物网格)
    pub turn_deliverables: Vec<Deliverable>,
    /// 当前 turn 起始时刻(信封毫秒;turn/end 求轮墙钟用时,瞬态不入相等性)
    turn_started_ms: Option<i64>,
    /// 节点出生时刻(入场动画年龄门控:超龄不包动画,gpui list 虚拟化
    /// 重挂不重放)。纯 UI 元数据,不参与相等性。
    pub node_born: std::collections::HashMap<String, std::time::Instant>,
    /// 重试行退避截止时刻(直播帧到达时刻 + delayMs;渲染期推算剩余秒,
    /// 1s tick 只触发重绘)。纯 UI 元数据,不参与相等性;重放不记。
    pub retry_deadlines: std::collections::HashMap<String, std::time::Instant>,
    /// 乐观用户消息(prompt RPC 回执即插;`user/message` 事件帧到达按
    /// 载荷 id 去重替换——发送到回显不再等「落盘→认领→全量扫→事件帧」
    /// 的完整往返)。直播态,不参与相等性;重放不记
    pub pending_user: Vec<PendingUser>,
    /// 本 turn 已认领 pending(认领先于 turn/start;hook 拒绝路径
    /// turn/start→turn/end 无 user/message,pending 由此回收)
    pub(crate) pending_user_claimed: bool,
    /// 节点 key → nodes 下标索引(定位 O(1))。历史合并的 chunk 追加
    /// 原为每次 `position` 线性扫,O(节点数 × 事件数) 二次复杂度——
    /// 3k 节点 × 3 万 chunk ≈ 10⁸ 次 key 比较,是打开长会话主线程冻结
    /// 的第二来源。节点只追加与就地改、无删除/重排(全文件变更面核查),
    /// 索引随 push 维护即可,无需失效路径。
    node_index: std::collections::HashMap<String, usize>,
    /// 历史合并中(born 不记录:载入的历史行不做入场动画;仅方法内瞬态)
    merging: bool,
    /// 已载历史的最早截断 seq(下一页 `before_seq`;0 = 已到会话头)
    pub history_cut: u64,
    /// 是否还有更早历史页(session.history 响应回填)
    pub history_has_more: bool,
    /// ask_user_question 行状态(行键 `call:{seq}` → 相位/题目/答案)。
    /// 直播与历史同 fold:工具行摘要与「回答」重开入口的事实源;迟到
    /// 应答/超时/取消都经 ask 族事件收口,重放一致
    pub asks: std::collections::HashMap<String, AskRowState>,
}

/// ask 工具行相位(事件驱动:requested→Open,timed-out→Continued,
/// answered→Answered,cancelled→Cancelled)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskPhase {
    /// 等待作答(卡在场或已收起,行可重开)
    Open,
    /// 超时转持续(模型已拿 pending 结果;仍可答)
    Continued,
    /// 已答(行摘要 n/total,展开只读回看)
    Answered,
    /// 已取消
    Cancelled,
}

/// 一行 ask 的折叠状态
#[derive(Debug, Clone, PartialEq)]
pub struct AskRowState {
    /// 发起 rpc(行「回答」重开卡时按此应答)
    pub rpc_id: String,
    /// 相位
    pub phase: AskPhase,
    /// 原问题集(事件载荷原样;重开卡与回看共用)
    pub questions: Value,
    /// 超时截止(信封时间 + timeoutSec;阻塞/continued 无)
    pub deadline_ms: Option<i64>,
    /// 已答批(ask/answered 的 answers;未答 None)
    pub answers: Option<Value>,
}

/// 相等性排除 UI 元数据:同事件序列的两个状态节点/业务字段全同,
/// 但出生时刻(Instant)不同;`merging` 仅 merge_history 执行期内为真。
impl PartialEq for ChatState {
    fn eq(&self, other: &Self) -> bool {
        self.nodes == other.nodes
            && self.todos == other.todos
            && self.running == other.running
            && self.queue == other.queue
            && self.plan_mode == other.plan_mode
            && self.turn_deliverables == other.turn_deliverables
            && self.asks == other.asks
    }
}

impl ChatState {
    /// 历史尾窗折叠(以现有投影为基线;节点 key 幂等,直播帧先行安全)。
    /// 折叠后 running 复位(断连窗口外的中间态不保留)。
    pub fn merge_history(&mut self, events: impl IntoIterator<Item = SessionEvent>) {
        self.merging = true;
        for ev in events {
            self.apply(&ev);
        }
        self.merging = false;
        self.running = false;
        self.compact_running = false;
        self.compact_queued = false;
        // 乐观气泡是直播态:历史载入不回放(与进度行/完成闪同语义)
        self.pending_user.clear();
        self.pending_user_claimed = false;
        // 重放不重现瞬态:进度行与完成闪都是直播态(事件序里有 progress,
        // 但历史载入不该回放一条已经结束的进度条)
        self.compact_progress = None;
        self.compact_settled = None;
        // 清孤儿 born(被折叠掉的头窗节点;防跨会话累积)
        let live: std::collections::HashSet<&str> = self.nodes.iter().map(ChatNode::key).collect();
        self.node_born.retain(|k, _| live.contains(k.as_str()));
        self.retry_deadlines
            .retain(|k, _| live.contains(k.as_str()));
    }
    /// 把仍在「执行中」的调用冻结为「已停止」。
    ///
    /// 用于**冷历史**重放后:日志若停在回合中途(被杀/崩溃,没有 `turn/end`),
    /// 未落定的调用会永远停在 [`ToolState::Running`]——而渲染层为 Running
    /// 挂的是 `repeat()` 扫光动画,一个这样的节点就会让整窗以 60fps 永久
    /// 重绘(实测:CPU 60%+ 且空闲不停)。没有 live turn 再驱动它们,故冻结;
    /// 会话仍在运行时**不要**调用(留给 live 事件收尾)
    pub fn freeze_unfinished_calls(&mut self) {
        for n in &mut self.nodes {
            if let ChatNode::Tool { state, .. } = n
                && *state == ToolState::Running
            {
                *state = ToolState::Stopped;
            }
        }
    }

    /// ask 终局/相位事件 → 行状态(按 callId 定位;id 兜底 = 未建行则忽略)
    fn ask_row_mut(&mut self, data: &Value) -> Option<&mut AskRowState> {
        let call = data["callId"].as_str()?;
        self.asks.get_mut(&format!("call:{call}"))
    }

    /// 应用单个客方事件
    pub fn apply(&mut self, ev: &SessionEvent) {
        match ev.ty.as_str() {
            // 新回合清空任务列表(turn/start 置空,
            // turn/end 保留完成的清单可见)
            "turn/start" => {
                self.running = true;
                self.todos.clear();
                self.turn_started_ms = Some(ev.time);
                // 认领标记(乐观回收状态机):认领先于 turn/start,空闲
                // 发送的下一个 turn/start 必是其认领——hook 拒绝路径
                // (turn/start→turn/end 无 user/message)据此回收 pending
                if !self.pending_user.is_empty() {
                    self.pending_user_claimed = true;
                }
                // 新回合清掉上一个压缩的收尾态(失败行/完成闪不跨轮滞留)
                if self.compact_progress.as_ref().is_some_and(|p| p.terminal()) {
                    self.compact_progress = None;
                }
                self.compact_settled = None;
            }
            "turn/end" => {
                self.running = false;
                // 乐观回收:hook 拒绝的 turn(user/message 未到)不残留
                // 永久 pending 气泡;正常路径 user/message 已先到清空,
                // 此处仅复位认领标记。驱动认领 steer(宿主通知)优先于
                // pending,空闲期通知 turn 会夹在发送与认领之间——条目
                // 仍在队列快照 = 未被本 turn 认领,气泡保留待其自己的
                // user/message 原位替换;已出队且未落档 = 认领后被拒,
                // 照旧回收
                if self.pending_user_claimed {
                    let queued: std::collections::HashSet<&str> =
                        self.queue.iter().map(|e| e.id.as_str()).collect();
                    self.pending_user.retain(|p| queued.contains(p.id.as_str()));
                    self.pending_user_claimed = false;
                }
                // 排队中的压缩任务在回合结束后被驱动取走 → 晋升进行态
                if self.compact_queued {
                    self.compact_queued = false;
                    self.compact_running = true;
                }
                let kind = ev.data["reason"]["kind"].as_str();
                let aborted = kind == Some("aborted");
                let errored = kind == Some("error");
                // 终态收口:残留流式位熄灭(半截正文冻结——「已停止」pill
                // 与动作行得以定稿;引擎对中断/错误路径不发 assistant/message
                // 定稿,不在此熄灭则指示永不出现)
                if aborted || errored {
                    for n in self.nodes.iter_mut() {
                        if let ChatNode::Assistant { streaming, .. } = n {
                            *streaming = false;
                        }
                    }
                }
                // 取消/错误收尾:等待退避中的重试行翻转「已取消」;
                // 未落定的调用显式翻成「已停止」(amber 点,行上不残留
                // 运行扫光;错误收口同理,否则平铺段残留永久扫光)
                if aborted || errored {
                    for n in self.nodes.iter_mut() {
                        match n {
                            ChatNode::Retry { state, .. } if *state == RetryState::Waiting => {
                                *state = RetryState::Cancelled;
                            }
                            ChatNode::Tool { state, .. } if *state == ToolState::Running => {
                                *state = ToolState::Stopped;
                            }
                            _ => {}
                        }
                    }
                }
                // 错误终止(传输失败/悬挂超时):通告行替代收尾行
                if kind == Some("error") {
                    let detail = ev.data["reason"]["error"]["message"]
                        .as_str()
                        .map(str::to_string);
                    self.push_node(ChatNode::Notice {
                        key: format!("turn-error:{}", ev.seq),
                        kind: NoticeKind::TurnError { detail },
                    });
                } else {
                    // 工作区快照变更(turn/end 载荷;git 状态对比产出,
                    // bash 创建文件的盲区兜底):path 已在集合则跳过——
                    // present/diff 已给出更丰富的描述/统计,git 只补缺席。
                    // untracked 新文件带行数统计(added/removed)
                    if let Some(changes) = ev.data["changes"].as_array() {
                        for c in changes {
                            let Some(p) = c["path"].as_str() else {
                                continue;
                            };
                            if self.turn_deliverables.iter().any(|d| d.path == p) {
                                continue;
                            }
                            let stat = match (c["added"].as_u64(), c["removed"].as_u64()) {
                                (Some(a), Some(r)) => Some((a, r)),
                                _ => None,
                            };
                            self.turn_deliverables.push(Deliverable {
                                path: p.to_string(),
                                description: None,
                                stat,
                                announced: false,
                            });
                        }
                    }
                    let deliverables = std::mem::take(&mut self.turn_deliverables);
                    let run_ms = self.turn_started_ms.map_or(0, |t0| (ev.time - t0).max(0));
                    self.turn_started_ms = None;
                    self.push_node(ChatNode::TurnTail {
                        key: format!("turn-end:{}", ev.seq),
                        aborted,
                        turn: ev.data["turn"].as_u64().unwrap_or(0),
                        ended_ms: ev.time,
                        run_ms,
                        deliverables,
                    });
                }
            }
            "user/message" => {
                // 分流(分类权威 source.kind):kind=user → 真实用户
                // 消息;否则注入上下文 → 折叠「注入行」。事件序已由引擎保证
                // 「真实用户先、注入后」,故无需 buffering,按 seq 直接追加。
                let kind = ev.data["source"]["kind"].as_str().unwrap_or("user");
                if kind == "user" {
                    self.push_node(ChatNode::User {
                        key: format!("user:{}", ev.seq),
                        text: content_text(&ev.data["content"]),
                        images: image_blocks(&ev.data["content"]),
                        files: file_blocks(&ev.data["content"]),
                        time: ev.time,
                    });
                    // 乐观气泡原位替换:载荷 id 与 RPC 回执 id 同源
                    // (宿主预分配贯通);历史重放的旧消息 id 不会命中
                    // 现存 pending,天然幂等
                    if let Some(id) = ev.data["id"].as_str() {
                        self.pending_user.retain(|p| p.id != id);
                    }
                } else if matches!(
                    kind,
                    "subagent-settled" | "subagent-message" | "shell-job-settled" | "ask-answer"
                ) {
                    // 结算/回发通知是**模型侧 turn 输入**(steer 注入),
                    // 不是对话内容:不建节点,转录完全不渲染(与 Claude
                    // Code 的 task-notification 同款形态——通知给模型,
                    // 用户看任务面板)。直播帧与历史折叠同走本分支,一处
                    // 判定即无「先渲染后折叠」的闪现;历史里消息本体仍在
                    // (模型可见),只是不进 UI
                } else {
                    self.push_node(ChatNode::Context {
                        key: format!("ctx:{}", ev.seq),
                        content: content_text(&ev.data["content"]),
                        source: ev.data["source"].clone(),
                    });
                }
            }
            // ask 族生命周期:行状态机折叠(无 callId 的骑通道问询不建行)
            "ask/requested" => {
                let Some(call) = ev.data["callId"].as_str() else {
                    return;
                };
                let timeout_sec = ev.data["timeoutSec"].as_i64().unwrap_or(-1);
                let row = AskRowState {
                    rpc_id: ev.data["id"].as_str().unwrap_or_default().to_string(),
                    phase: AskPhase::Open,
                    questions: ev.data["questions"].clone(),
                    deadline_ms: (timeout_sec > 0)
                        .then(|| ev.time + timeout_sec.saturating_mul(1000)),
                    answers: None,
                };
                self.asks.insert(format!("call:{call}"), row);
            }
            "ask/timed-out" => {
                if let Some(row) = self.ask_row_mut(&ev.data) {
                    row.phase = AskPhase::Continued;
                    row.deadline_ms = None;
                }
            }
            "ask/answered" => {
                if let Some(row) = self.ask_row_mut(&ev.data) {
                    row.phase = AskPhase::Answered;
                    row.deadline_ms = None;
                    row.answers = Some(ev.data["answers"].clone());
                }
            }
            "ask/cancelled" => {
                if let Some(row) = self.ask_row_mut(&ev.data) {
                    row.phase = AskPhase::Cancelled;
                    row.deadline_ms = None;
                }
            }
            "agent/inbox/spliced" => {
                // 队列簿记(durable inbox):入队/编辑/转移的 inserted
                // 是重放素材,不是对话内容——排队条目经 session/queue 帧
                // 呈现,认领后由 user/message 渲染(避免重启后双渲染)
            }
            "assistant/chunk" => {
                let text = ev.data["chunk"]["text"].as_str().unwrap_or_default();
                if text.is_empty() {
                    return;
                }
                self.content_ver = self.content_ver.wrapping_add(1);
                self.with_streaming_node(&ev.data, |node| {
                    if let ChatNode::Assistant {
                        text: t, text_ver, ..
                    } = node
                    {
                        t.push_str(text);
                        *text_ver = text_ver.wrapping_add(1);
                    }
                });
            }
            "assistant/reasoning" => {
                let text = ev.data["text"].as_str().unwrap_or_default();
                if text.is_empty() {
                    return;
                }
                self.content_ver = self.content_ver.wrapping_add(1);
                self.with_streaming_node(&ev.data, |node| {
                    // 增量是片段(逐 token,engine 逐条落档)——直接拼接;
                    // 换行属于内容本身
                    if let ChatNode::Assistant { reasoning, .. } = node {
                        reasoning.push_str(text);
                    }
                });
            }
            // 流式残段丢弃标记(重试前):清空该步流式缓冲,与引擎的
            // 累积重置同序——重放与实况同一可见语义(引擎重发前直接
            // 丢弃部分 chunks,RS 日志只追加,以标记达成)
            "assistant/stream-reset" => {
                let key = stream_key(&ev.data);
                if let Some(ix) = self.position(&key)
                    && let ChatNode::Assistant {
                        text,
                        text_ver,
                        reasoning,
                        streaming,
                        ..
                    } = &mut self.nodes[ix]
                {
                    text.clear();
                    *text_ver = text_ver.wrapping_add(1);
                    reasoning.clear();
                    *streaming = true;
                    self.content_ver = self.content_ver.wrapping_add(1);
                }
            }
            // LLM 重试:落折叠行(直播记退避截止时刻供倒计时;重放不记)
            "llm/retry" => {
                let key = format!("retry:{}", ev.seq);
                self.push_node(ChatNode::Retry {
                    key,
                    retry: ev.data["retry"].as_u64().unwrap_or(0) as u32,
                    max_retries: ev.data["maxRetries"].as_u64().unwrap_or(0) as u32,
                    delay_ms: ev.data["delayMs"].as_u64().unwrap_or(0),
                    code: ev.data["code"].as_str().unwrap_or_default().to_string(),
                    message: ev.data["message"].as_str().unwrap_or_default().to_string(),
                    state: RetryState::Waiting,
                });
                if !self.merging {
                    let deadline = std::time::Instant::now()
                        + std::time::Duration::from_millis(
                            ev.data["delayMs"].as_u64().unwrap_or(0),
                        );
                    self.retry_deadlines
                        .insert(format!("retry:{}", ev.seq), deadline);
                }
            }
            // 退避结束、重发开始:最后一个等待中的重试行翻转「已重试」
            "llm/retry-started" => {
                if let Some(ChatNode::Retry { state, .. }) = self.nodes.iter_mut().rev().find(|n| {
                    matches!(
                        n,
                        ChatNode::Retry {
                            state: RetryState::Waiting,
                            ..
                        }
                    )
                }) {
                    *state = RetryState::Started;
                }
            }
            "assistant/message" => {
                let key = stream_key(&ev.data);
                let text = content_text(&ev.data["message"]["content"]);
                let usage = ev.data.get("usage").filter(|u| !u.is_null()).cloned();
                let message_id = ev.data["message"]["id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                // 定稿:流式节点替换为终值(无 chunk 时也直接建)
                match self.position(&key) {
                    Some(ix) => {
                        if let ChatNode::Assistant {
                            text: t,
                            text_ver,
                            streaming,
                            usage: u,
                            message_id: mid,
                            ..
                        } = &mut self.nodes[ix]
                        {
                            if !text.is_empty() {
                                *t = text;
                                *text_ver = text_ver.wrapping_add(1);
                                self.content_ver = self.content_ver.wrapping_add(1);
                            }
                            *streaming = false;
                            if usage.is_some() {
                                *u = usage;
                            }
                            if !message_id.is_empty() {
                                *mid = message_id.clone();
                            }
                        }
                    }
                    None => {
                        self.push_node(ChatNode::Assistant {
                            key,
                            text,
                            text_ver: 1,
                            reasoning: String::new(),
                            streaming: false,
                            usage,
                            message_id,
                        });
                    }
                }
            }
            "tool/call" => {
                let call_id = ev.data["callId"].as_str().unwrap_or_default().to_string();
                let name = ev.data["name"].as_str().unwrap_or_default().to_string();
                let arguments = ev.data["arguments"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                self.push_node(ChatNode::Tool {
                    summary: liuma_core::export::summarize_call(&name, &arguments),
                    key: format!("call:{call_id}"),
                    view: ev.data.get("view").filter(|v| !v.is_null()).cloned(),
                    name: name.clone(),
                    state: ToolState::Running,
                    arguments: arguments.clone(),
                    output: None,
                    images: Vec::new(),
                });
                // 计划卡调用派生:exit_plan_mode 的提交即出卡,
                // 失败路径计划正文不再只活在 80 字符摘要里。参数解析失败
                // 不出卡(与 plan/submitted 同判);正文以 plan/submitted
                // 为权威,此处仅草稿态
                if name == "exit_plan_mode"
                    && let Ok(args) = serde_json::from_str::<Value>(&arguments)
                    && let Some(plan) = args["plan"].as_str().filter(|p| !p.is_empty())
                {
                    self.push_node(ChatNode::Plan {
                        key: format!("plan-call:{call_id}"),
                        plan: plan.to_string(),
                        status: PlanStatus::Draft,
                    });
                }
            }
            "tool/result" => {
                // message.content[0].toolCallId 配对
                let call_id = ev.data["message"]["content"][0]["toolCallId"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let is_error = ev.data["message"]["content"][0]["isError"].as_bool();
                let output = content_text(&ev.data["message"]["content"][0]["content"]);
                // exit_plan_mode 错误结果的计划卡收口标记(节点借用结束后生效)
                let mut plan_call_failed = false;
                if let Some(ix) = self.position(&format!("call:{call_id}"))
                    && let ChatNode::Tool {
                        state,
                        output: o,
                        view,
                        images,
                        name,
                        ..
                    } = &mut self.nodes[ix]
                {
                    *state = if is_error == Some(true) {
                        ToolState::Error
                    } else {
                        ToolState::Done
                    };
                    *o = Some(output);
                    // 计划卡失败收口在节点借用结束后执行(见块尾):
                    // exit_plan_mode 错误结果且未见 plan/submitted → Failed
                    plan_call_failed = is_error == Some(true) && name == "exit_plan_mode";
                    // result 侧视图权威替换:在场覆盖 call 意图,缺席清除
                    // (如编辑失败无 diff → 通用卡)
                    *view = ev.data.get("view").filter(|v| !v.is_null()).cloned();
                    // 结果图片(MCP 图片桥):引用数组权威替换
                    *images = ev
                        .data
                        .get("images")
                        .and_then(|v| v.as_array().cloned())
                        .unwrap_or_default();
                    // 交付收集 = present 宣告 ∪ diff 写入的**并集**
                    // (视图驱动,不 switch 工具名;失败 is_error 不收)。
                    // 两层语义正交:present 宣告 = 交付层(描述,后到
                    // 覆盖),diff 写入 = 变更层(+N -M 统计,后到覆盖;
                    // 兼作改动卡的兜底角色)。字段各自动更新,相遇时
                    // 互补而非互斥。
                    if is_error != Some(true)
                        && let Some(v) = &*view
                    {
                        if v["card"].as_str() == Some("present")
                            && let Some(p) = v["path"].as_str()
                        {
                            let desc = v["description"].as_str().map(str::to_string);
                            match self.turn_deliverables.iter_mut().find(|d| d.path == p) {
                                Some(d) => {
                                    d.description = desc;
                                    d.announced = true;
                                }
                                None => {
                                    self.turn_deliverables.push(Deliverable {
                                        path: p.to_string(),
                                        description: desc,
                                        stat: None,
                                        announced: true,
                                    });
                                }
                            }
                        } else if v["card"].as_str() == Some("diff")
                            && let Some(diffs) = v["diffs"].as_array()
                        {
                            for d in diffs {
                                if let Some(p) = d["path"].as_str() {
                                    // 最后一次编辑的统计(最终态语义);
                                    // 新建(old 缺席)= +N -0
                                    let stat = Some((
                                        d["newText"]
                                            .as_str()
                                            .map_or(0, |t| t.lines().count() as u64),
                                        d["oldText"]
                                            .as_str()
                                            .map_or(0, |t| t.lines().count() as u64),
                                    ));
                                    match self.turn_deliverables.iter_mut().find(|x| x.path == p) {
                                        Some(x) => x.stat = stat,
                                        None => self.turn_deliverables.push(Deliverable {
                                            path: p.to_string(),
                                            description: None,
                                            stat,
                                            announced: false,
                                        }),
                                    }
                                }
                            }
                        }
                    }
                }
                // 计划卡失败收口:错误结果且草稿卡仍在(未见
                // plan/submitted)→ Failed,计划正文保留可见
                if plan_call_failed
                    && let Some(ChatNode::Plan { status, .. }) =
                        self.nodes.iter_mut().rev().find(|n| {
                            matches!(
                                n,
                                ChatNode::Plan {
                                    status: PlanStatus::Draft,
                                    ..
                                }
                            )
                        })
                {
                    *status = PlanStatus::Failed;
                }
            }
            // 计划归档四件:submitted 把就近调用派生草稿卡转正(同一提交
            // 只一张卡;正文以本事件为权威);无草稿卡(旧日志/参数解析
            // 失败)退回落新卡。approved/declined/cancelled 更新最后一个
            // 待批节点的状态(事件序保证配对)
            "plan/submitted" => {
                let plan = ev.data["plan"].as_str().unwrap_or_default().to_string();
                if let Some(ChatNode::Plan {
                    plan: p, status, ..
                }) = self.nodes.iter_mut().rev().find(|n| {
                    matches!(
                        n,
                        ChatNode::Plan {
                            status: PlanStatus::Draft,
                            ..
                        }
                    )
                }) {
                    *p = plan;
                    *status = PlanStatus::Pending;
                } else {
                    self.push_indexed(ChatNode::Plan {
                        key: format!("plan:{}", ev.seq),
                        plan,
                        status: PlanStatus::Pending,
                    });
                }
            }
            // 压缩标记行(历史折叠落档;标记行不
            // 替换被折叠的转写行,展开看摘要)。终局清进行/排队位
            "compaction/summary" => {
                self.compact_running = false;
                self.compact_queued = false;
                if self.compact_progress.take().is_some() {
                    // 完成闪:有进度会话收尾才闪(重放/无进度路径不闪)
                    self.compact_settled =
                        Some((format!("cpt:{}", ev.seq), std::time::Instant::now()));
                }
                self.push_indexed(ChatNode::Compaction {
                    key: format!("cpt:{}", ev.seq),
                    items: ev.data["items"].as_u64(),
                    tokens: ev.data["shadowedTokens"].as_u64(),
                    pruned: ev.data["prunedItems"].as_u64(),
                });
            }
            // 压缩进度(相位 + 真实已生成字符数;引擎按 200ms/64 字符
            // 节流)。进行相位置位并驱动进度行;done 只清进度(summary
            // 先到,完成闪挂在标记行);failed 留一行「压缩未完成」——
            // 自动路径的失败没有 compaction/error,这里是唯一信号
            "compaction/progress" => {
                let phase = ev.data["phase"].as_str().unwrap_or_default();
                let progress = CompactProgress {
                    phase: phase.to_string(),
                    generated_chars: ev.data["generatedChars"].as_u64().unwrap_or(0),
                    elapsed_ms: ev.data["elapsedMs"].as_u64().unwrap_or(0),
                    estimated_tokens: ev.data["estimatedTokens"].as_u64().unwrap_or(0),
                    through_seq: ev.data["throughSeq"].as_u64().unwrap_or(0),
                };
                match phase {
                    "done" => {
                        self.compact_progress = None;
                        self.compact_running = false;
                        self.compact_queued = false;
                    }
                    "failed" => {
                        self.compact_running = false;
                        self.compact_queued = false;
                        self.compact_progress = Some(progress);
                    }
                    _ => {
                        self.compact_running = true;
                        self.compact_queued = false;
                        self.compact_progress = Some(progress);
                    }
                }
            }
            // 压缩终局失败/空(kind 区分:empty=无历史可压 → 中性状态行,
            // 文案渲染期换词典(宿主常量是英文;照 `(tool call only)` 先例,
            // 事件原文不动);error=真实失败 → 红色告警行,文本=消息原文
            // (错误态直显 settlement))
            "compaction/error" => {
                self.compact_running = false;
                self.compact_queued = false;
                let msg = ev.data["message"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| t!("chat.unknown_error").into_owned());
                if ev.data["kind"].as_str() == Some("empty") {
                    self.push_node(ChatNode::CompactStatus {
                        key: format!("cpt-empty:{}", ev.seq),
                    });
                } else {
                    self.push_node(ChatNode::Notice {
                        key: format!("cpt-err:{}", ev.seq),
                        kind: NoticeKind::Compaction {
                            text: msg.to_string(),
                        },
                    });
                }
            }
            "plan/approved" | "plan/declined" | "plan/cancelled" => {
                let status = match ev.ty.as_str() {
                    "plan/approved" => PlanStatus::Approved,
                    "plan/declined" => PlanStatus::Declined,
                    _ => PlanStatus::Cancelled,
                };
                // 事件序保证配对:更新最后一个待批计划节点 →
                // 归档卡状态徽标随之变化(拒绝/取消的界面反馈);
                // Draft 一并防御(终局先于 submitted 的异常序)
                if let Some(ChatNode::Plan { status: s, .. }) =
                    self.nodes.iter_mut().rev().find(|n| {
                        matches!(
                            n,
                            ChatNode::Plan {
                                status: PlanStatus::Pending | PlanStatus::Draft,
                                ..
                            }
                        )
                    })
                {
                    *s = status;
                }
            }
            "todo/write" => {
                self.todos = ev.data["todos"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .map(|t| TodoItem {
                                content: t["content"].as_str().unwrap_or_default().into(),
                                status: t["status"].as_str().unwrap_or("pending").into(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
            }
            "plan/mode" => {
                self.plan_mode = ev.data["active"].as_bool().unwrap_or(false);
            }
            _ => {}
        }
    }

    /// 尾部追加已定位的节点并登记 key 索引(调用方负责 key 去重语义)
    fn push_indexed(&mut self, node: ChatNode) -> usize {
        self.nodes.push(node);
        let ix = self.nodes.len() - 1;
        // node 已移入 nodes,借尾元素 key 登记(免额外克隆)
        let key = self.nodes[ix].key().to_string();
        self.node_index.insert(key, ix);
        ix
    }

    /// 尾部追加节点(key 去重:同 key 已存在则跳过——历史/直播重放幂等)
    pub(crate) fn push_node(&mut self, node: ChatNode) {
        if self.position(node.key()).is_some() {
            return;
        }
        self.record_born(node.key());
        self.push_indexed(node);
    }

    /// 头窗拼接:更早历史页折出的投影节点**前插**(「加载更早」)。
    /// 节点 key 为绝对 turn/step(翻译层 prime 过前缀),无需重编号;
    /// key 去重天然防边界轮重复;node_index 整体重建(全体下移)
    pub fn prepend_history(&mut self, older: ChatState) {
        let mut nodes = older.nodes;
        nodes.append(&mut self.nodes);
        self.nodes = nodes;
        self.node_index.clear();
        for (ix, node) in self.nodes.iter().enumerate() {
            self.node_index.insert(node.key().to_string(), ix);
        }
    }

    /// 记录节点出生(仅直播帧;历史合并载入的行不做入场动画)
    fn record_born(&mut self, key: &str) {
        if !self.merging {
            self.node_born
                .entry(key.to_string())
                .or_insert_with(std::time::Instant::now);
        }
    }

    /// 流式节点定位/创建(assistant chunk/reasoning 目标)
    fn with_streaming_node(&mut self, data: &Value, f: impl FnOnce(&mut ChatNode)) {
        let key = stream_key(data);
        let ix = match self.position(&key) {
            Some(ix) => ix,
            None => {
                self.record_born(&key);
                self.push_indexed(ChatNode::Assistant {
                    key,
                    text: String::new(),
                    text_ver: 0,
                    reasoning: String::new(),
                    streaming: true,
                    usage: None,
                    message_id: String::new(),
                })
            }
        };
        f(&mut self.nodes[ix]);
    }

    fn position(&self, key: &str) -> Option<usize> {
        self.node_index.get(key).copied()
    }

    // 回合用量摘要已退役(旧「耗时 · 首 token · tok/s」文本行):轮尾
    // 统计改由 turn_usage 桶驱动 pill + 详情卡
}

/// 零高节点:空正文且无思考的定稿 Assistant(纯 tool_calls 步的
/// assistant/message 占位)——渲染不产出任何可见内容,行槽仍会为它
/// 生成成员行,渲染层据此整行跳过(否则 py(8) 留 16px 空隙,
/// 展开组行间疏密不均)。
pub(crate) fn invisible_node(n: &ChatNode) -> bool {
    match n {
        // 注入行(AGENTS.md / runtime-context / skill 目录 / 时钟 / 后台
        // 任务通知)**不进聊天流**:可见性判据对 context 节点只放行携带
        // 工具增删的条目,系统提示与普通注入一律不产行。注入仍是
        // **过程项**(不切组),只是没有可见行;原文在检查器/轨迹里可查。
        ChatNode::Context { .. } => true,
        ChatNode::Assistant {
            text,
            reasoning,
            streaming,
            ..
        } => text.is_empty() && reasoning.is_empty() && !streaming,
        _ => false,
    }
}

/// 直播组保留键(未收口尾段的合成组键;turn key 域为 `turn-end:<seq>`
/// 形,前缀控制字符保证永不与之相撞)
pub(crate) const LIVE_TURN_KEY: &str = "\u{0}live";

/// 直播组头活动归类(组头标签 + 活动图标;无运行工具时回退 thinking)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ProcessActivity {
    /// 正在分析请求(无运行工具的回退)
    Thinking,
    /// 正在读取文件
    Read,
    /// 正在读取图片
    ReadImage,
    /// 正在写入文件
    Write,
    /// 正在编辑文件
    Edit,
    /// 正在搜索代码
    Search,
    /// 正在运行命令
    Commands,
    /// 正在运行代码
    Code,
    /// 正在搜索网页
    WebSearch,
    /// 正在访问网页
    WebFetch,
    /// 正在协调子智能体
    Subagents,
    /// 正在更新计划
    Plan,
    /// 等待你的操作
    Questions,
    /// 正在调用工具(未归类兜底)
    Tools,
}

/// 工具名 → 活动归类(组头标签口径;shell 工具模型面名随平台,
/// 与 [`crate::kits::icons::tool_path`] 的图标归类同源不同用)
pub(crate) fn activity_of(name: &str) -> ProcessActivity {
    if name == liuma_sandbox::shell::tool_name() || name == "bash" || name == "shell" {
        return ProcessActivity::Commands;
    }
    match name {
        "file_read" | "read" => ProcessActivity::Read,
        "read_image" => ProcessActivity::ReadImage,
        "grep" | "glob" | "file_search" => ProcessActivity::Search,
        "write" => ProcessActivity::Write,
        "file_edit" | "edit" => ProcessActivity::Edit,
        "code" => ProcessActivity::Code,
        "web_search" => ProcessActivity::WebSearch,
        "web_fetch" => ProcessActivity::WebFetch,
        "subagent" | "subagent_list" => ProcessActivity::Subagents,
        "todo_write" | "goal" => ProcessActivity::Plan,
        "ask" | "ask_user_question" => ProcessActivity::Questions,
        _ => ProcessActivity::Tools,
    }
}

/// 组头详情截断上限(字符)
const LIVE_DETAIL_MAX_CHARS: usize = 160;

/// 直播组头详情:参数 JSON 按展示键序取首个非空值(字符串或字符串
/// 数组),空白折叠、超长截断。取值键序 = 人读参数优先(标题/描述/
/// 任务),命令与路径殿后
pub(crate) fn live_detail(arguments: &str) -> Option<String> {
    const DETAIL_KEYS: [&str; 20] = [
        "title",
        "description",
        "objective",
        "task",
        "task_name",
        "name",
        "question",
        "questions",
        "prompt",
        "message",
        "command",
        "cmd",
        "queries",
        "query",
        "pattern",
        "url",
        "uri",
        "file_path",
        "path",
        "target",
    ];
    let args: Value = serde_json::from_str(arguments).ok()?;
    for key in DETAIL_KEYS {
        let Some(v) = args.get(key) else {
            continue;
        };
        let text = match v {
            Value::String(s) => Some(s.as_str().to_string()),
            Value::Array(a) if !a.is_empty() && a.iter().all(|i| i.is_string()) => Some(
                a.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            _ => None,
        };
        if let Some(t) = text {
            let normalized = t.split_whitespace().collect::<Vec<_>>().join(" ");
            if !normalized.is_empty() {
                return Some(truncate_live_detail(&normalized));
            }
        }
    }
    None
}

/// 按字符截断加省略号(截断点去尾空白)
fn truncate_live_detail(s: &str) -> String {
    if s.chars().count() <= LIVE_DETAIL_MAX_CHARS {
        return s.to_string();
    }
    let cut: String = s.chars().take(LIVE_DETAIL_MAX_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

/// 思考摘要去强调标记(`**` 删除;反引号等其余标记原样保留)
fn strip_bold(s: &str) -> String {
    s.replace("**", "")
}

/// 空行分段范围(段分隔 = 换行 + 空白列 + 换行;返回 `(起, 止)`
/// 字节区间,不含分隔序列)
fn paragraph_ranges(text: &str) -> Vec<(usize, usize)> {
    let b = text.as_bytes();
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\n' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < b.len() && matches!(b[j], b' ' | b'\t' | b'\r') {
            j += 1;
        }
        if j < b.len() && b[j] == b'\n' {
            ranges.push((start, i));
            start = j + 1;
            i = start;
        } else {
            i += 1;
        }
    }
    ranges.push((start, b.len()));
    ranges
}

/// 流式思考摘要:最后一个**首行已收行**段落的首行(去 `**`)。段落
/// 首行写完(段内有换行,或段落已被空行分隔收口)才参与;全部行未
/// 收行时为空——摘要随段落收行逐段前进,不随流内逐字抖动
pub(crate) fn streaming_think_summary(text: &str) -> String {
    let ranges = paragraph_ranges(text);
    let last = ranges.len().saturating_sub(1);
    ranges
        .iter()
        .enumerate()
        .rev()
        .filter_map(|(idx, &(s, e))| match text[s..e].find('\n') {
            Some(off) => Some(text[s..s + off].trim()),
            // 单行段:被分隔序列收口的段,其首行必已收行(分隔以换行
            // 起始);唯尾段例外——尾段行未写完不参与
            None if idx < last => Some(text[s..e].trim()),
            None => None,
        })
        .find(|l| !l.is_empty())
        .map(strip_bold)
        .unwrap_or_default()
}

/// 定稿思考摘要:首个非空行(去 `**`)
pub(crate) fn settled_think_summary(text: &str) -> String {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| strip_bold(l.trim()))
        .unwrap_or_default()
}

/// 直播组头:`(活动, 详情)`。段内最后运行中工具的归类 + 参数详情;
/// 无运行工具回退 thinking + 最后一段已完成思考首行(运行中工具缺
/// 详情不回退——标题单独成立)
pub(crate) fn live_header(
    nodes: &[ChatNode],
    first: usize,
    last: usize,
) -> (ProcessActivity, Option<String>) {
    let mut activity = ProcessActivity::Thinking;
    let mut detail = None;
    let range = nodes.get(first..=last).unwrap_or(&[]);
    for node in range.iter().rev() {
        if let ChatNode::Tool {
            name,
            state: ToolState::Running,
            arguments,
            ..
        } = node
        {
            activity = activity_of(name);
            detail = live_detail(arguments);
            break;
        }
    }
    if activity == ProcessActivity::Thinking {
        for node in range.iter().rev() {
            if let ChatNode::Assistant { reasoning, .. } = node
                && !reasoning.is_empty()
            {
                let summary = streaming_think_summary(reasoning);
                if !summary.is_empty() {
                    detail = Some(summary);
                }
                break;
            }
        }
    }
    (activity, detail)
}

/// 行槽内助手节点的可见部分。回复正文会**切开**过程组、思考行留在
/// 组内——同一节点因此在槽层可能只露一半。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodePart {
    /// 整块(非助手节点;或思考/正文只有其一)
    Full,
    /// 仅思考行(过程组内;正文已切开在组外)
    Reasoning,
    /// 仅正文(过程组外的助手回复)
    Body,
}

/// 轮结束口径(轮控制行文案与折叠默认值的来源)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEnd {
    /// 正常完成(有定稿答复):默认收拢,点控制行展开
    Completed,
    /// 用户中断:恒展开(中断语义由「已停止」pill 承担,过程不藏)
    Aborted,
    /// 错误收尾:恒展开
    Failed,
}

/// 渲染行槽(读取层派生):`ChatNode` 平铺流 → 虚拟化列表行。
/// 每次「连续过程运行」收进一个**组座位**(组头 + 限高滚动体);
/// 助手回复正文留在组外(并由此切开相邻两组);收口轮在多组之前
/// 多一行**轮控制行**。nodes 本体不动(增量投影/merge 幂等根基)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowSlot {
    /// 平铺节点(携带节点下标与可见部分;debug selector/element id
    /// 用原始序号)
    Node {
        /// 节点下标
        ix: usize,
        /// 可见部分(Full = 整块)
        part: NodePart,
    },
    /// 轮控制行:「已完成，用时 X」/「已停止」/「处理失败」。
    /// 折叠态只留这一行 + 最终答复;中断/错误轮不折叠。
    TurnHead {
        /// 收口节点 key(turn-end:<seq> / turn-error:<seq>)
        turn_key: String,
        /// 结束口径
        end: TurnEnd,
        /// 轮墙钟用时(毫秒;0 = 未知)
        run_ms: u64,
    },
    /// 过程组座位:一次连续过程运行(`first..=last` 全是过程项,回复
    /// 正文已在槽层切开)。
    Group {
        /// 组键(按运行起点稳定:`g:<first>`;开合与滚动句柄按它存)
        key: String,
        /// 所属轮键(直播段为 [`LIVE_TURN_KEY`])
        turn_key: String,
        /// 运行内首成员下标
        first: usize,
        /// 运行内末成员下标
        last: usize,
        /// 运行已收口(其后有回复正文,或整轮已收口):组头走分类并词、
        /// 不带扫光;未收口的尾段运行才是「正在X · 详情」
        closed: bool,
        /// 展开态(默认收拢成一行组头,点开才铺成员)
        open: bool,
    },
}

/// 过程项:注入上下文 / 工具调用 / 重试 / **不带正文**的助手步。
///
/// 只有带正文的助手步才是组边界。不带正文的助手步(纯 tool_calls 步、
/// 正文与思考都空)属于过程:它只是没有可见行,**不得切断相邻工具**——
/// 否则「同一段连续过程」会被切成好几个组(实测:一次 git 状态检查 +
/// 一次读文件被拆成「执行了命令」「已读取文件」两个组头)。
fn is_process_node(n: &ChatNode) -> bool {
    match n {
        ChatNode::Context { .. } | ChatNode::Tool { .. } | ChatNode::Retry { .. } => true,
        ChatNode::Assistant { text, .. } => text.is_empty(),
        _ => false,
    }
}

/// 段收口:从 `start` 起第一个收口节点(含)之后的排他下标 + 结束口径。
/// 无收口 = 直播段(未收口尾段)。
fn segment_end(nodes: &[ChatNode], start: usize) -> (usize, Option<(TurnEnd, String, u64)>) {
    for (k, n) in nodes.iter().enumerate().skip(start) {
        match n {
            ChatNode::TurnTail {
                key,
                aborted,
                run_ms,
                ..
            } => {
                let end = if *aborted {
                    TurnEnd::Aborted
                } else {
                    TurnEnd::Completed
                };
                return (k + 1, Some((end, key.clone(), (*run_ms).max(0) as u64)));
            }
            // 只有轮错误通告是段的收口;压缩/本地通告是段内普通行
            ChatNode::Notice {
                key,
                kind: NoticeKind::TurnError { .. },
            } => {
                return (k + 1, Some((TurnEnd::Failed, key.clone(), 0)));
            }
            _ => {}
        }
    }
    (nodes.len(), None)
}

/// 节点流 → 渲染行槽(纯函数)。
///
/// 分段:「段」= 上一收口节点之后至下一收口之间的全部节点(收口节点
/// 属前段)。段内再按**过程运行**切组:过程项(Context / Tool / Retry /
/// 只有思考的助手步)连续成一组;带正文的助手步是边界——正文平铺在
/// 组外,它的思考行留在组内(同一助手步在槽层只露正文半边)。
///
/// 每段:轮前散件(attach 基线注入等)与首个用户消息平铺 → 收口段
/// 一行轮控制行 → 组与正文按原序。正常完成且未展开的轮收拢成控制行
/// 加最终答复;中断/错误收口轮不折叠(过程不藏,「已停止」/错误行
/// 自己交代结局);末尾未收口段 = 直播段,无控制行、组恒在。
pub fn build_row_slots(
    nodes: &[ChatNode],
    open_turns: &std::collections::HashSet<String>,
    open_groups: &std::collections::HashSet<String>,
) -> Vec<RowSlot> {
    let mut slots: Vec<RowSlot> = Vec::with_capacity(nodes.len());
    let mut i = 0;
    while i < nodes.len() {
        let (seg_end, ended) = segment_end(nodes, i);
        // 段内首个用户消息 = 轮次开始;其前的段成员(attach 基线注入等)
        // 不属于任何轮,连同用户消息一起平铺
        let first_user = (i..seg_end).find(|&k| matches!(nodes[k], ChatNode::User { .. }));
        let proc_start = first_user.map_or(i, |u| u + 1);
        for k in i..proc_start {
            slots.push(RowSlot::Node {
                ix: k,
                part: NodePart::Full,
            });
        }
        i = seg_end;
        let Some((end, turn_key, run_ms)) = ended else {
            // 直播段:无控制行,过程照常成组
            push_runs(
                nodes,
                proc_start,
                seg_end,
                LIVE_TURN_KEY,
                false,
                open_groups,
                &mut slots,
            );
            continue;
        };
        // 空轮(只有收口节点:无过程项也无回复)不产控制行——没有可收的
        // 东西,一行禁用的「已停止」只是噪声
        let has_content = nodes[proc_start..seg_end].iter().any(|n| {
            is_process_node(n) || matches!(n, ChatNode::Assistant { text, .. } if !text.is_empty())
        });
        if !has_content {
            for k in proc_start..seg_end {
                slots.push(RowSlot::Node {
                    ix: k,
                    part: NodePart::Full,
                });
            }
            continue;
        }
        // 正常完成 + 未点开 → 收拢:控制行 + 回复正文,过程组收起
        let folded = end == TurnEnd::Completed && !open_turns.contains(&turn_key);
        slots.push(RowSlot::TurnHead {
            turn_key: turn_key.clone(),
            end,
            run_ms,
        });
        if folded {
            // 折叠只收过程组:回复正文照旧平铺(最终答复与中途叙述都在),
            // 收口节点自己交代结局
            for (k, node) in nodes.iter().enumerate().take(seg_end).skip(proc_start) {
                match node {
                    ChatNode::TurnTail { .. } | ChatNode::Notice { .. } => {
                        slots.push(RowSlot::Node {
                            ix: k,
                            part: NodePart::Full,
                        })
                    }
                    ChatNode::Assistant { text, .. } if !text.is_empty() => {
                        slots.push(RowSlot::Node {
                            ix: k,
                            part: NodePart::Body,
                        })
                    }
                    _ => {}
                }
            }
            continue;
        }
        push_runs(
            nodes,
            proc_start,
            seg_end,
            &turn_key,
            true,
            open_groups,
            &mut slots,
        );
    }
    slots
}

/// 段内过程运行切组 + 回复正文平铺(见 [`build_row_slots`])。
///
/// 运行被回复正文切开时**已收口**(组头走分类并词);段末仍在跑的运行
/// 才继承段的收口态——直播段里它仍是「正在X · 详情」。
fn push_runs(
    nodes: &[ChatNode],
    start: usize,
    end: usize,
    turn_key: &str,
    segment_closed: bool,
    open_groups: &std::collections::HashSet<String>,
    slots: &mut Vec<RowSlot>,
) {
    let mut run: Option<(usize, usize)> = None;
    let flush = |run: &mut Option<(usize, usize)>, closed: bool, slots: &mut Vec<RowSlot>| {
        if let Some((first, last)) = run.take() {
            let key = format!("g:{first}");
            slots.push(RowSlot::Group {
                open: open_groups.contains(&key),
                key,
                turn_key: turn_key.to_string(),
                first,
                last,
                closed,
            });
        }
    };
    for (k, node) in nodes.iter().enumerate().take(end).skip(start) {
        if is_process_node(node) {
            run = Some(match run {
                Some((first, _)) => (first, k),
                None => (k, k),
            });
            continue;
        }
        // 带正文的助手步:思考行归前一组,正文切开组
        let split = matches!(
            node,
            ChatNode::Assistant { text, reasoning, .. }
                if !text.is_empty() && !reasoning.is_empty()
        );
        if split {
            run = Some(match run {
                Some((first, _)) => (first, k),
                None => (k, k),
            });
        }
        // 回复正文切开运行 = 该运行已收口
        flush(&mut run, true, slots);
        slots.push(RowSlot::Node {
            ix: k,
            part: if split {
                NodePart::Body
            } else {
                NodePart::Full
            },
        });
    }
    flush(&mut run, segment_closed, slots);
}

/// 组内活动的分类计数(按调用数降序、同数按首现——组头文案口径;
/// 只数工具,思考不是一类活动)
pub fn group_categories(nodes: &[ChatNode], first: usize, last: usize) -> Vec<ProcessActivity> {
    let mut order: Vec<ProcessActivity> = Vec::new();
    let mut counts: std::collections::HashMap<ProcessActivity, usize> =
        std::collections::HashMap::new();
    for n in nodes.get(first..=last).into_iter().flatten() {
        let ChatNode::Tool { name, .. } = n else {
            continue;
        };
        let activity = activity_of(name);
        let slot = counts.entry(activity).or_insert(0);
        if *slot == 0 {
            order.push(activity);
        }
        *slot += 1;
    }
    order.sort_by_key(|a| std::cmp::Reverse(counts.get(a).copied().unwrap_or(0)));
    order
}

/// 收口组头文案:按并词规则拼前三个分类。
///
/// - 空 → 「已完成分析」(只有思考)
/// - 一个 → 该分类
/// - 两个 → `%{first}并%{second}`;两者同以「已」开头时第二项去掉
///   前缀并把首字母小写(中文无变化),避免「已…并已…」
/// - 三个以上 → 逗号连接(后续项仅首字母小写,前缀保留),超出
///   三个以「等」收尾
pub fn process_title(categories: &[ProcessActivity]) -> String {
    let label = |a: ProcessActivity| match a {
        ProcessActivity::Thinking => t!("chat.step_done_thinking").to_string(),
        ProcessActivity::Read => t!("chat.step_done_read").to_string(),
        ProcessActivity::ReadImage => t!("chat.step_done_read_image").to_string(),
        ProcessActivity::Write => t!("chat.step_done_write").to_string(),
        ProcessActivity::Edit => t!("chat.step_done_edit").to_string(),
        ProcessActivity::Search => t!("chat.step_done_search").to_string(),
        ProcessActivity::Commands => t!("chat.step_done_commands").to_string(),
        ProcessActivity::Code => t!("chat.step_done_code").to_string(),
        ProcessActivity::WebSearch => t!("chat.step_done_web_search").to_string(),
        ProcessActivity::WebFetch => t!("chat.step_done_web_fetch").to_string(),
        ProcessActivity::Subagents => t!("chat.step_done_subagents").to_string(),
        ProcessActivity::Plan => t!("chat.step_done_plan").to_string(),
        ProcessActivity::Questions => t!("chat.step_done_questions").to_string(),
        ProcessActivity::Tools => t!("chat.step_done_tools").to_string(),
    };
    let continuation = |s: &str| {
        let mut chars = s.chars();
        match chars.next() {
            Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    };
    let labels: Vec<String> = categories.iter().map(|a| label(*a)).collect();
    let Some(first) = labels.first() else {
        return t!("chat.step_done_thinking").to_string();
    };
    let Some(second) = labels.get(1) else {
        return first.clone();
    };
    if labels.len() == 2 {
        let prefix = t!("chat.step_done_shared_prefix").to_string();
        let shared = !prefix.is_empty()
            && first.starts_with(prefix.as_str())
            && second.starts_with(prefix.as_str());
        let second = if shared {
            continuation(&second[prefix.len()..])
        } else {
            second.clone()
        };
        return t!(
            "chat.step_done_join_two",
            first = first.as_str(),
            second = second.as_str()
        )
        .to_string();
    }
    let title = std::iter::once(first.clone())
        .chain(labels.iter().skip(1).map(|l| continuation(l)))
        .collect::<Vec<_>>()
        .join(t!("chat.step_done_comma").as_ref());
    if categories.len() > 3 {
        t!("chat.step_done_more", title = title.as_str()).to_string()
    } else {
        title
    }
}

// ---- 导航轨锚点(读取层派生)----

/// 锚点类别(轨上点的语义标签)
/// 导航轨锚点:一个用户消息轮次(轨上一个点)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavAnchor {
    /// 行槽下标(点击跳转目标;当前位置高亮也按它比较)。None = 轮次
    /// **尚未加载进列表**(全量索引里有、聊天列尾窗外)——点击 = 向前
    /// 分页覆盖该轮后自动跳转。
    pub slot_ix: Option<usize>,
    /// 用户节点稳定 key(user:<seq>)
    pub key: String,
    /// hover 卡标题:消息首行单行化截断(粗体)
    pub title: String,
    /// hover 卡正文预览:次行起单行化 ~240 字(无次行则为空)
    pub preview: String,
}

/// 全量锚点派生:已加载锚点来自行槽(nav_anchors),未加载锚点来自
/// anchor_index(全量轮次索引,key 不在已投影 nodes 中的部分,排在
/// 已加载之前——seq 更小)。合并序 = seq 升序。
pub fn nav_anchors_full(
    slots: &[RowSlot],
    nodes: &[ChatNode],
    anchor_index: &[(u64, String)],
) -> Vec<NavAnchor> {
    let loaded = nav_anchors(slots, nodes);
    let loaded_keys: std::collections::HashSet<&str> =
        loaded.iter().map(|a| a.key.as_str()).collect();
    // 未加载:key 不在已加载集合(全量索引 seq 与投影 key 的 user:<seq>
    // 对应);title 取索引摘要
    let unloaded: Vec<NavAnchor> = anchor_index
        .iter()
        .filter(|(seq, _)| {
            let key = format!("user:{seq}");
            !loaded_keys.contains(key.as_str())
        })
        .map(|(seq, title)| NavAnchor {
            slot_ix: None,
            key: format!("user:{seq}"),
            title: title.clone(),
            preview: String::new(),
        })
        .collect();
    let mut out = unloaded;
    out.extend(loaded);
    out
}

fn first_and_rest(text: &str) -> (String, String) {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let title = lines.next().unwrap_or_default();
    let rest: Vec<&str> = lines.collect();
    (one_line(title, 48), one_line(&rest.join(" "), 240))
}

/// 行槽 → 导航锚点(纯函数)。**锚点 = 用户消息(轮次开始)**——导航的
/// 语义就是在轮次之间跳转,其余(答复/组行/收尾/通告)一概不是锚点。
/// 视口顶 → 当前轮锚的序号(anchors 中最近一个行槽 ≤ top 的**已加载**
/// 锚;未加载锚 slot_ix=None 永不命中)。导航轨当前位置标记的权威口径
/// ——paint 相期逐帧调用(读实时滚动位),与元素渲染无关,滚动零延迟
/// 跟手;事件回调口径(先于布局 settle)不可与之混用,否则两套判断在
/// 轮次边界互相打架 = 高亮闪烁
pub fn current_nav_ix(anchor_slots: &[Option<usize>], top: usize) -> Option<usize> {
    anchor_slots
        .iter()
        .enumerate()
        .rev()
        .find(|(_, slot)| slot.is_some_and(|ix| ix <= top))
        .map(|(i, _)| i)
}

/// [`current_nav_ix`] 的标记渲染形态(常亮不灭):视口顶落在已加载
/// 首锚**之上**时(历史分页窗口边界切在轮中段——正在看的是更早轮
/// 的尾部,其锚未加载),回落到已加载首锚的前一个锚,而非列表首锚
/// (全量加载时代两者恰好同一;窗口化后回落首锚 = 标记瞬跳到
/// 未加载区顶部)。全未加载兜底 0
pub fn current_nav_ix_clamped(anchor_slots: &[Option<usize>], top: usize) -> usize {
    match current_nav_ix(anchor_slots, top) {
        Some(i) => i,
        None => anchor_slots
            .iter()
            .position(|s| s.is_some())
            .map(|first| first.saturating_sub(1))
            .unwrap_or(0),
    }
}

pub fn nav_anchors(slots: &[RowSlot], nodes: &[ChatNode]) -> Vec<NavAnchor> {
    let mut out = Vec::new();
    for (slot_ix, slot) in slots.iter().enumerate() {
        let RowSlot::Node { ix: n, .. } = slot else {
            continue;
        };
        let Some(ChatNode::User { key, text, .. }) = nodes.get(*n) else {
            continue;
        };
        let (title, preview) = first_and_rest(text);
        out.push(NavAnchor {
            slot_ix: Some(slot_ix),
            key: key.clone(),
            title,
            preview,
        });
    }
    out
}

/// 流式节点 key:a:<turn>:<step>
fn stream_key(data: &Value) -> String {
    format!("a:{}:{}", data["turn"], data["step"])
}

/// content 块数组 → 拼接 text 块文本
fn content_text(content: &Value) -> String {
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"].as_str() == Some("text"))
                .map(|b| b["text"].as_str().unwrap_or_default().to_string())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// 内容里的 image 块(attachment 引用数组;纯字符串 = 空)
fn image_blocks(content: &Value) -> Vec<Value> {
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"].as_str() == Some("image"))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// 内容里的 file 块(attachment 引用数组;纯字符串 = 空)
fn file_blocks(content: &Value) -> Vec<Value> {
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"].as_str() == Some("file"))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// todo_write 行摘要:解析**该次调用自己的 arguments**,
/// 非全局当前态 —— 每行反映本次写入的列表。text 可截断,
/// extra(并行进行中额外数)不可 —— 窄行也不剪掉「还有几条在跑」。
/// 坏 JSON / 非对象根 / todos 非数组 / 元素非对象 → None(回落通用摘要)
pub(crate) struct TodoRowSummary {
    /// `{done}/{total} 已完成`(+ 首个进行中文本)
    pub text: String,
    /// 进行中条目超出首条的数量(0 = 无额外)
    pub extra: usize,
}

pub(crate) fn todo_row_summary(arguments: &str) -> Option<TodoRowSummary> {
    let parsed: Value = serde_json::from_str(arguments).ok()?;
    if !parsed.is_object() {
        return None;
    }
    let todos = parsed["todos"].as_array()?;
    if !todos.iter().all(|t| t.is_object()) {
        return None;
    }
    let done = todos.iter().filter(|t| t["status"] == "completed").count();
    let actives: Vec<&str> = todos
        .iter()
        .filter(|t| t["status"] == "in_progress")
        .filter_map(|t| t["content"].as_str())
        .collect();
    let mut text = t!("chat.todo_done", done = done, total = todos.len()).into_owned();
    // 首个进行中文本可用(非空白)才挂名 + 计额外数
    let mut extra = 0;
    if let Some(first) = actives.first().filter(|c| !c.trim().is_empty()) {
        text.push_str(&format!(" · {}", one_line(first, 60)));
        extra = actives.len() - 1;
    }
    Some(TodoRowSummary { text, extra })
}

/// 单行化 + 字符级截断
fn one_line(s: &str, max: usize) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&collapsed, max)
}

/// 工作区根前缀剥离(显示面):绝对路径落在根内 → 相对形,其余原样
/// (折叠行摘要与卡横幅 path 共用)
pub(crate) fn relativize(root: Option<&str>, text: &str) -> String {
    let Some(root) = root
        .map(|r| r.trim_end_matches('/'))
        .filter(|r| !r.is_empty())
    else {
        return text.to_string();
    };
    for sep in [format!("{root}/"), format!("{root}\\")] {
        if let Some(rest) = text.strip_prefix(&sep) {
            return rest.to_string();
        }
    }
    text.to_string()
}

/// 字符级截断(中文安全)
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use liuma_core::export::summarize_call;

    /// 视口顶 → 当前轮锚:最近一个行槽 ≤ top 的已加载锚(未加载锚
    /// slot_ix=None 永不命中;钉底跟随态 top=行槽数,命中最后一个)。
    /// 导航轨 paint 相期位置标记的权威口径回归锁
    #[test]
    fn current_nav_ix_picks_nearest_loaded_anchor_at_or_above_top() {
        let slots = vec![Some(0), Some(5), None, Some(9)];
        assert_eq!(current_nav_ix(&slots, 0), Some(0), "视口在第 1 轮");
        assert_eq!(current_nav_ix(&slots, 4), Some(0), "未过下一锚维持第 1 轮");
        assert_eq!(current_nav_ix(&slots, 5), Some(1), "边界行本身上算跨入");
        assert_eq!(current_nav_ix(&slots, 8), Some(1));
        assert_eq!(current_nav_ix(&slots, 7), Some(1), "未加载锚不是位置");
        assert_eq!(current_nav_ix(&slots, 100), Some(3), "钉底 = 最后一个");
        assert_eq!(current_nav_ix(&[], 0), None, "无锚无位置");
        assert_eq!(current_nav_ix(&[None, None], 5), None, "全未加载无位置");
    }

    /// 标记渲染形态的回落:窗口边界之上(正在看更早轮的尾部、其锚
    /// 未加载)落到已加载首锚的**前一个**锚,而非列表首锚——全量加载
    /// 时代两者恰好同一,窗口化后回落首锚 = 标记瞬跳到未加载区顶部
    #[test]
    fn clamped_nav_ix_falls_back_to_anchor_before_loaded_window() {
        // 10 锚:前 6 未加载,后 4 已加载(行槽 50/60/70/80)
        let slots = vec![
            None,
            None,
            None,
            None,
            None,
            None,
            Some(50),
            Some(60),
            Some(70),
            Some(80),
        ];
        // 命中区:与 current_nav_ix 一致
        assert_eq!(current_nav_ix_clamped(&slots, 80), 9);
        assert_eq!(
            current_nav_ix_clamped(&slots, 55),
            6,
            "仍在锚 6 区间(锚 7 行号 60 未到)"
        );
        assert_eq!(current_nav_ix_clamped(&slots, 60), 7);
        assert_eq!(
            current_nav_ix_clamped(&slots, 50),
            6,
            "最老已加载锚命中自身"
        );
        // 边界区(50 之上):回落到已加载首锚(6)的前一个 = 5,
        // 而不是跳到列表首锚 0
        assert_eq!(current_nav_ix_clamped(&slots, 49), 5);
        assert_eq!(current_nav_ix_clamped(&slots, 0), 5);
        // 全量加载(无未加载锚):之上回落首锚 0(与旧行为一致)
        let full = vec![Some(0), Some(5), None, Some(9)];
        assert_eq!(current_nav_ix_clamped(&full, 0), 0);
        // 全未加载 / 空:兜底 0
        assert_eq!(current_nav_ix_clamped(&[None, None], 5), 0);
        assert_eq!(current_nav_ix_clamped(&[], 0), 0);
    }

    /// session/queue items 解析:queued/steering 分流、preview 拼接、
    /// 非文本块置不可编辑
    #[test]
    fn parse_queue_items_splits_placement_and_preview() {
        let items = serde_json::json!([
            {
                "id": "q1",
                "placement": "queued",
                "message": { "role": "user", "content": [ { "type": "text", "text": "排队的问题" } ] }
            },
            {
                "id": "s1",
                "placement": "steering",
                "message": { "role": "user", "content": [
                    { "type": "text", "text": "插队的" },
                    { "type": "image", "url": "x" }
                ] }
            }
        ]);
        let entries = parse_queue_items(items.as_array().cloned().unwrap());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].placement, QueuePlacement::Queued);
        assert_eq!(entries[0].preview, "排队的问题");
        assert_eq!(entries[0].text.as_deref(), Some("排队的问题"));
        assert_eq!(entries[1].placement, QueuePlacement::Steering);
        assert_eq!(entries[1].preview, "插队的");
        assert_eq!(entries[1].text, None, "含图片块 = 不可编辑");
    }

    /// 回归锁(queue dock 多行预览溢出):多行消息的 preview 必须单行化
    /// (空白折叠 + 截断)——truncate 对硬换行失效,多行原文会溢出 36px
    /// 行框被 composer 盖住;编辑面 text 保留原文不受归一化污染
    #[test]
    fn parse_queue_items_normalizes_preview_to_single_line() {
        let items = serde_json::json!([
            {
                "id": "q1",
                "placement": "queued",
                "message": { "role": "user", "content": [ { "type": "text",
                    "text": "  第一行\n第二行\r\n\t第三行\u{3000}末行  " } ] }
            },
            {
                "id": "s1",
                "placement": "steering",
                "message": { "role": "user", "content": [
                    { "type": "text", "text": "a\n\nb" },
                    { "type": "image", "url": "x" }
                ] }
            }
        ]);
        let entries = parse_queue_items(items.as_array().cloned().unwrap());
        assert_eq!(entries.len(), 2);
        // 纯文本条目:preview 单行化,text 保留原文(编辑链路回填输入框)
        assert_eq!(entries[0].preview, "第一行 第二行 第三行 末行");
        assert_eq!(
            entries[0].text.as_deref(),
            Some("  第一行\n第二行\r\n\t第三行\u{3000}末行  ")
        );
        assert_eq!(entries[0].full_text(), entries[0].text.as_deref().unwrap());
        // 含非文本块条目:不可编辑,full_text 回落单行 preview
        assert_eq!(entries[1].preview, "a b");
        assert_eq!(entries[1].text, None);
        assert_eq!(entries[1].full_text(), "a b");
        // 任何条目的展示面都不再携带换行
        for e in &entries {
            assert!(!e.preview.contains('\n'), "preview 必须单行");
        }
    }

    use liuma_core::proto::SurfaceOp;
    use serde_json::json;

    fn ev(ty: &str, seq: u64, data: Value) -> SessionEvent {
        SessionEvent {
            ty: ty.into(),
            seq,
            time: 0,
            data,
            source_event_seqs: None,
            surface_op: Some(SurfaceOp::Append),
            ignorable: None,
        }
    }

    /// 回归锁(乐观替换):pending_user 在同 id 的 user/message 到达后
    /// 原位消解(节点入列、气泡出列);不同 id 不误伤
    #[test]
    fn optimistic_pending_user_resolves_by_id() {
        let mut st = ChatState::default();
        st.pending_user.push(PendingUser {
            id: "m-1".into(),
            text: "hello".into(),
        });
        st.apply(&ev(
            "user/message",
            2,
            json!({
                "id": "m-1", "role": "user",
                "content": [ { "type": "text", "text": "hello" } ],
                "source": { "kind": "user" },
            }),
        ));
        assert!(
            st.nodes.iter().any(|n| n.key() == "user:2"),
            "user/message 节点应入列"
        );
        assert!(st.pending_user.is_empty(), "同 id pending 应消解");
        // 不同 id:不消解(下一轮再收)
        st.pending_user.push(PendingUser {
            id: "m-2".into(),
            text: "other".into(),
        });
        st.apply(&ev(
            "user/message",
            5,
            json!({
                "id": "m-9", "role": "user",
                "content": [ { "type": "text", "text": "x" } ],
                "source": { "kind": "user" },
            }),
        ));
        assert_eq!(st.pending_user.len(), 1, "不同 id 不误伤");
    }

    /// 回归锁(乐观回收):hook 拒绝路径 turn/start→turn/end(无
    /// user/message)不残留永久 pending 气泡;正常收尾也复位认领标记
    #[test]
    fn optimistic_pending_user_cleared_on_unclaimed_turn_end() {
        let mut st = ChatState::default();
        st.pending_user.push(PendingUser {
            id: "m-1".into(),
            text: "hi".into(),
        });
        st.apply(&ev("turn/start", 1, json!({ "turn": 1 })));
        assert!(st.pending_user_claimed, "turn/start 应置认领标记");
        st.apply(&ev(
            "turn/end",
            3,
            json!({ "reason": { "kind": "blocked" } }),
        ));
        assert!(
            st.pending_user.is_empty(),
            "拒绝收尾的 turn 不得残留 pending"
        );
        assert!(!st.pending_user_claimed, "认领标记应复位");
    }

    /// 回归锁(通知 turn 夹层):驱动认领 steer(宿主通知)优先于
    /// pending,空闲期通知 turn 会插在用户发送与认领之间——条目仍在
    /// 队列快照(未被认领)时 turn/end 不得回收其乐观气泡;出队后仍
    /// 无 user/message(hook 拒绝)才回收
    #[test]
    fn optimistic_pending_user_survives_intervening_notice_turn() {
        let mut st = ChatState::default();
        st.pending_user.push(PendingUser {
            id: "m-1".into(),
            text: "hi".into(),
        });
        // 队列快照仍含 m-1(queued 待认领)→ 通知 turn 的 start/end
        // 全程不得回收
        st.queue = vec![QueueEntry {
            id: "m-1".into(),
            placement: QueuePlacement::Queued,
            preview: "hi".into(),
            text: Some("hi".into()),
        }];
        st.apply(&ev("turn/start", 1, json!({ "turn": 2 })));
        st.apply(&ev(
            "turn/end",
            3,
            json!({ "reason": { "kind": "end_turn" } }),
        ));
        assert_eq!(st.pending_user.len(), 1, "未认领条目跨通知 turn 保留");

        // 驱动认领(出队帧先于其 turn)后被 hook 拒绝 → 回收
        st.queue.clear();
        st.apply(&ev("turn/start", 4, json!({ "turn": 3 })));
        st.apply(&ev(
            "turn/end",
            6,
            json!({ "reason": { "kind": "blocked" } }),
        ));
        assert!(
            st.pending_user.is_empty(),
            "出队后拒绝收尾的 turn 回收 pending"
        );
    }

    /// 全回合样本(对齐 translate.rs 测试序列)
    fn full_turn() -> Vec<SessionEvent> {
        vec![
            ev("turn/start", 1, json!({ "turn": 1 })),
            ev(
                "user/message",
                2,
                json!({
                    "id": "u1", "role": "user",
                    "content": [ { "type": "text", "text": "hi" } ],
                    "source": { "kind": "user" },
                }),
            ),
            ev("step/start", 3, json!({ "turn": 1, "step": 1 })),
            ev(
                "assistant/chunk",
                4,
                json!({ "turn": 1, "step": 1, "chunk": { "type": "text-delta", "index": 0, "text": "he" } }),
            ),
            ev(
                "assistant/chunk",
                5,
                json!({ "turn": 1, "step": 1, "chunk": { "type": "text-delta", "index": 0, "text": "llo" } }),
            ),
            ev(
                "assistant/reasoning",
                6,
                json!({ "turn": 1, "step": 1, "text": "思考中" }),
            ),
            ev(
                "assistant/message",
                7,
                json!({
                    "turn": 1, "step": 1,
                    "message": { "id": "m1", "role": "assistant",
                        "content": [ { "type": "text", "text": "hello" } ] },
                    "usage": { "durationMs": 100, "ttftMs": 50, "outputTokens": 5 },
                }),
            ),
            ev(
                "tool/call",
                8,
                json!({ "turn": 1, "step": 2, "callId": "8", "name": "bash", "arguments": "{\"command\":\"ls -la\"}" }),
            ),
            ev(
                "tool/result",
                9,
                json!({
                    "turn": 1, "step": 2,
                    "message": { "id": "m2", "role": "user", "content": [ {
                        "type": "tool-result", "toolCallId": "8",
                        "content": [ { "type": "text", "text": "file-a" } ],
                        "isError": false } ] },
                }),
            ),
            ev(
                "assistant/message",
                10,
                json!({
                    "turn": 1, "step": 3,
                    "message": { "id": "m3", "role": "assistant",
                        "content": [ { "type": "text", "text": "done" } ] },
                }),
            ),
            ev(
                "turn/end",
                11,
                json!({ "turn": 1, "reason": { "kind": "completed" } }),
            ),
        ]
    }

    #[test]
    fn full_turn_projection() {
        let mut st = ChatState::default();
        st.merge_history(full_turn());
        assert!(!st.running);
        let keys: Vec<&str> = st.nodes.iter().map(|n| n.key()).collect();
        assert_eq!(
            keys,
            vec!["user:2", "a:1:1", "call:8", "a:1:3", "turn-end:11"]
        );
        // 用户节点来自 user/message(seq 2)
        assert!(matches!(&st.nodes[0], ChatNode::User { text, .. } if text == "hi"));
        // 助手流式定稿:chunk 拼接被定稿文本覆盖,usage 附着
        match &st.nodes[1] {
            ChatNode::Assistant {
                text,
                reasoning,
                streaming,
                usage,
                ..
            } => {
                assert_eq!(text, "hello");
                assert_eq!(reasoning, "思考中");
                assert!(!streaming);
                assert_eq!(usage.as_ref().unwrap()["durationMs"], 100);
            }
            other => panic!("expected assistant, got {other:?}"),
        }
        // 工具配对:running → done,输出附着
        match &st.nodes[2] {
            ChatNode::Tool {
                name,
                summary,
                state,
                output,
                ..
            } => {
                assert_eq!(name, "bash");
                assert_eq!(summary, "ls -la");
                assert_eq!(*state, ToolState::Done);
                assert_eq!(output.as_deref(), Some("file-a"));
            }
            other => panic!("expected tool, got {other:?}"),
        }
        // 回合收尾:轮号/收尾时刻入节点(turn_usage 桶由 stats 帧另行喂入)
        match &st.nodes[4] {
            ChatNode::TurnTail {
                aborted,
                turn,
                ended_ms,
                run_ms,
                ..
            } => {
                assert!(!aborted);
                assert_eq!(*turn, 1);
                assert_eq!(*ended_ms, 0);
                assert_eq!(*run_ms, 0);
            }
            other => panic!("expected turn tail, got {other:?}"),
        }
    }

    /// 轮墙钟用时:turn/start 与 turn/end 信封时刻差
    #[test]
    fn turn_tail_tracks_run_ms() {
        let mut st = ChatState::default();
        let mut start = ev("turn/start", 1, json!({ "turn": 1 }));
        start.time = 1_000;
        st.apply(&start);
        let mut end = ev(
            "turn/end",
            9,
            json!({ "turn": 1, "reason": { "kind": "completed" } }),
        );
        end.time = 31_000;
        st.apply(&end);
        match &st.nodes[0] {
            ChatNode::TurnTail { turn, run_ms, .. } => {
                assert_eq!(*turn, 1);
                assert_eq!(*run_ms, 30_000);
            }
            other => panic!("expected turn tail, got {other:?}"),
        }
    }

    /// 压缩事件对:summary → Compaction 标记行(带统计);error →
    /// Notice 通告(手动压缩失败/空反馈)
    #[test]
    fn compaction_events_project_marker_and_notice() {
        // 进行位由 /compact 受理置位(本地),终局事件清位
        let mut st = ChatState {
            compact_running: true,
            ..Default::default()
        };
        st.apply(&ev(
            "compaction/summary",
            2,
            json!({ "summary": "ckpt body", "items": 5, "shadowedTokens": 1234,
                    "prunedItems": 2 }),
        ));
        assert!(!st.compact_running, "summary 终局应清进行位");
        match &st.nodes[0] {
            ChatNode::Compaction {
                key,
                items,
                tokens,
                pruned,
            } => {
                assert_eq!(key, "cpt:2", "key 的 seq 就是台账定位锚");
                assert_eq!(*items, Some(5));
                assert_eq!(*tokens, Some(1234));
                assert_eq!(*pruned, Some(2), "已生效裁掉条数");
            }
            other => panic!("expected compaction marker, got {other:?}"),
        }
        // kind=empty → 中性状态行;节点只带 key,文案渲染期取词典
        // (宿主载荷里的英文 settlement 不落节点 = 结构上锁死中英混排
        // 的回归)
        st.compact_running = true;
        st.apply(&ev(
            "compaction/error",
            3,
            json!({ "kind": "empty", "message": "No compactable history yet." }),
        ));
        assert!(!st.compact_running, "empty 终局应清进行位");
        match &st.nodes[1] {
            ChatNode::CompactStatus { key } => assert_eq!(key, "cpt-empty:3"),
            other => panic!("expected compact status, got {other:?}"),
        }
        assert!(
            !format!("{:?}", st.nodes[1]).contains("No compactable history yet."),
            "空反馈节点不得携带宿主英文原文(渲染期走词典)"
        );
        // kind=error(真实失败)→ 红色通告,文本 = 消息原文(无「压缩:」前缀)
        st.apply(&ev(
            "compaction/error",
            4,
            json!({ "kind": "error", "message": "boom" }),
        ));
        match &st.nodes[2] {
            ChatNode::Notice {
                key,
                kind: NoticeKind::Compaction { text },
            } => {
                assert_eq!(key, "cpt-err:4");
                assert_eq!(text, "boom");
            }
            other => panic!("expected notice, got {other:?}"),
        }
        // 旧日志无 kind(向后兼容):视作真实失败走红色通告
        st.apply(&ev("compaction/error", 5, json!({ "message": "legacy" })));
        match &st.nodes[3] {
            ChatNode::Notice {
                kind: NoticeKind::Compaction { text },
                ..
            } => assert_eq!(text, "legacy"),
            other => panic!("expected notice, got {other:?}"),
        }
    }

    /// 排队中的压缩任务在回合结束后晋升为进行态(turn/end;驱动仅在
    /// turn 间隙取压缩任务),终局事件清位
    #[test]
    fn compact_queued_promotes_on_turn_end() {
        let mut st = ChatState {
            compact_queued: true,
            ..Default::default()
        };
        st.apply(&ev("turn/end", 1, json!({ "reason": {} })));
        assert!(!st.compact_queued, "回合结束应清排队位");
        assert!(st.compact_running, "排队应晋升为进行态");
        st.apply(&ev(
            "compaction/error",
            2,
            json!({ "kind": "empty", "message": "No compactable history yet." }),
        ));
        assert!(!st.compact_running, "终局事件应清进行位");
    }

    /// 进度直播态:置进行位 + 快照入状态;终局 done 清进度、summary
    /// 收尾时挂完成闪(有进度会话才闪);failed 留「压缩未完成」行,
    /// 下一轮 turn/start 清掉
    #[test]
    fn compaction_progress_streams_and_settles() {
        let mut st = ChatState::default();
        st.apply(&ev(
            "compaction/progress",
            1,
            json!({ "phase": "summarize", "generatedChars": 0, "elapsedMs": 0,
                    "estimatedTokens": 8000, "throughSeq": 42, "items": 9, "manual": false }),
        ));
        assert!(st.compact_running, "进度即进行态(自动路径此前没有开始信号)");
        let p = st.compact_progress.clone().expect("进度快照");
        assert_eq!(p.phase, "summarize");
        assert_eq!(p.percent(), 0.0);

        st.apply(&ev(
            "compaction/progress",
            2,
            json!({ "phase": "summarize", "generatedChars": 1000, "elapsedMs": 900,
                    "estimatedTokens": 8000, "throughSeq": 42, "items": 9, "manual": false }),
        ));
        let p = st.compact_progress.clone().expect("进度快照");
        assert_eq!(p.percent(), 50.0, "分母 = 8000×0.25 = 2000 字符");
        // 封顶 99:完成由 done 相位宣告,条不自己宣布完成
        assert_eq!(
            CompactProgress {
                generated_chars: u64::MAX,
                ..p.clone()
            }
            .percent(),
            99.0
        );
        assert_eq!(p.elapsed_ms, 900);

        // summary 先到:清进度 + 挂完成闪(键 = 该标记行)
        st.apply(&ev(
            "compaction/summary",
            3,
            json!({ "summary": "s", "throughSeq": 42, "items": 9, "shadowedTokens": 8000 }),
        ));
        assert!(st.compact_progress.is_none(), "落档即清进度");
        assert!(!st.compact_running);
        assert_eq!(
            st.compact_settled.as_ref().map(|(k, _)| k.as_str()),
            Some("cpt:3"),
            "完成闪挂在刚落档的标记行上"
        );
        // done 相位只清进度(不覆盖完成闪)
        st.apply(&ev(
            "compaction/progress",
            4,
            json!({ "phase": "done", "generatedChars": 1000, "elapsedMs": 1200,
                    "estimatedTokens": 8000, "throughSeq": 42, "items": 9, "manual": false }),
        ));
        assert!(st.compact_progress.is_none());
        assert!(st.compact_settled.is_some());
        // 下一轮开场清收尾态
        st.apply(&ev("turn/start", 5, json!({ "turn": 2 })));
        assert!(st.compact_settled.is_none(), "完成闪不跨轮滞留");
    }

    /// 失败终局:自动路径没有 compaction/error,这行是唯一信号;
    /// 静态留行,下一轮清
    #[test]
    fn compaction_progress_failed_lingers_until_next_turn() {
        let mut st = ChatState {
            compact_running: true,
            ..Default::default()
        };
        st.apply(&ev(
            "compaction/progress",
            1,
            json!({ "phase": "failed", "generatedChars": 300, "elapsedMs": 1200,
                    "estimatedTokens": 8000, "throughSeq": 7, "items": 3, "manual": false }),
        ));
        assert!(!st.compact_running, "终局即收进行位(不再呼吸)");
        let p = st.compact_progress.as_ref().expect("失败行留驻");
        assert!(p.terminal() && p.phase == "failed");
        st.apply(&ev("turn/start", 2, json!({ "turn": 2 })));
        assert!(st.compact_progress.is_none(), "新回合清失败行");
    }

    /// 进度与完成闪纯属 UI 瞬态:只有它们不同的两个状态必须相等
    /// (否则每 tick 都判不等 → 触发全量重绘)
    #[test]
    fn progress_fields_stay_out_of_equality() {
        let base = ChatState::default;
        let mut a = base();
        let mut b = base();
        // 逐字段都不同(含折叠身份);仍在相等性之外
        a.compact_progress = Some(CompactProgress {
            phase: "summarize".into(),
            generated_chars: 10,
            elapsed_ms: 5,
            estimated_tokens: 8000,
            through_seq: 42,
        });
        b.compact_progress = Some(CompactProgress {
            phase: "judge".into(),
            generated_chars: 9999,
            elapsed_ms: 900,
            estimated_tokens: 8001,
            through_seq: 43,
        });
        b.compact_settled = Some(("cpt:1".into(), std::time::Instant::now()));
        assert_eq!(a, b, "进度/完成闪不入相等性");
    }

    /// 错误终止的回合:Notice 通告替代收尾行,running 复位
    /// (translate 把 durable turn/error 译为 turn/end{kind:"error"})
    #[test]
    fn error_turn_end_projects_notice() {
        let mut st = ChatState::default();
        st.apply(&ev("turn/start", 1, json!({ "turn": 1 })));
        st.apply(&ev("user/message", 2, json!({ "content": "hi" })));
        assert!(st.running);
        st.apply(&ev(
            "turn/end",
            3,
            json!({
                "turn": 1,
                "reason": {
                    "kind": "error",
                    "error": { "code": "TRANSPORT", "message": "读超时" },
                },
            }),
        ));
        assert!(!st.running);
        match st.nodes.last() {
            Some(ChatNode::Notice {
                kind: NoticeKind::TurnError { detail },
                ..
            }) => {
                let detail = detail.as_deref().unwrap_or_default();
                assert!(detail.contains("读超时"), "通告应含错误信息: {detail}");
            }
            other => panic!("expected notice, got {other:?}"),
        }
        assert!(
            !st.nodes
                .iter()
                .any(|n| matches!(n, ChatNode::TurnTail { .. })),
            "错误终止不应再出收尾行"
        );
    }

    /// 4a:user/message + source.kind≠user 折叠为注入行(渲染紧跟用户气泡)。
    /// 事件序已由引擎保证「真实用户先、注入后」,注入行按 seq 直接追加,
    /// 无需缓冲;注入行由 source.kind 分流识别。
    #[test]
    fn context_message_projects_after_user() {
        let mut st = ChatState::default();
        // 注入行(user/message + source.kind=agent-instructions)
        st.apply(&ev(
            "user/message",
            1,
            json!({
                "id": "ctx-1",
                "role": "user",
                "content": [ { "type": "text", "text": "AGENTS.md 全文" } ],
                "source": { "kind": "agent-instructions", "form": "instructions" },
            }),
        ));
        // 真实用户消息
        st.apply(&ev(
            "user/message",
            2,
            json!({ "content": [ { "type": "text", "text": "hi" } ] }),
        ));
        assert_eq!(st.nodes.len(), 2);
        match &st.nodes[0] {
            ChatNode::Context {
                content, source, ..
            } => {
                assert_eq!(content, "AGENTS.md 全文");
                assert_eq!(source["kind"], "agent-instructions");
            }
            other => panic!("第 0 节点应为注入行,实际 {other:?}"),
        }
        match &st.nodes[1] {
            ChatNode::User { text, .. } => assert_eq!(text, "hi"),
            other => panic!("第 1 节点应为用户消息,实际 {other:?}"),
        }
    }

    /// 结算/回发通知(user/message + 非用户 source.kind)是模型侧
    /// turn 输入:投影不为其建任何节点——转录完全不渲染(与 Claude
    /// Code 的 task-notification 同款形态:通知给模型,用户看任务面板);
    /// 直播与历史同走本分支,无「先渲染后折叠」的闪现
    #[test]
    fn settlement_notices_project_to_nothing() {
        let mut st = ChatState::default();
        for kind in ["subagent-settled", "subagent-message", "shell-job-settled"] {
            st.apply(&ev(
                "user/message",
                1,
                json!({
                    "id": "notice-1",
                    "role": "user",
                    "content": [ { "type": "text", "text": "Background job settled." } ],
                    "source": {
                        "kind": kind,
                        "form": "notice",
                        "summary": "Background job settled.",
                        "senderSessionId": "s-1",
                    },
                }),
            ));
        }
        // 真实用户消息不受影响
        st.apply(&ev(
            "user/message",
            2,
            json!({ "content": [ { "type": "text", "text": "hi" } ] }),
        ));
        assert_eq!(st.nodes.len(), 1, "通知零节点,仅剩用户消息");
        match &st.nodes[0] {
            ChatNode::User { text, .. } => assert_eq!(text, "hi"),
            other => panic!("唯一节点应为用户消息,实际 {other:?}"),
        }
    }

    /// ask 族事件折叠:requested 建行(deadline 派生)→ timed-out 转
    /// Continued(清 deadline)→ answered 收 Answered + 答案批;无 callId
    /// 的骑通道问询不建行。
    #[test]
    fn ask_events_fold_row_phases() {
        let mut st = ChatState::default();
        st.apply(&ev(
            "ask/requested",
            10,
            json!({
                "id": "r1", "callId": "7", "timeoutSec": 30,
                "questions": [ { "id": "q1", "question": "继续?" } ],
            }),
        ));
        let row = st.asks.get("call:7").expect("requested 建行");
        assert_eq!(row.rpc_id, "r1");
        assert_eq!(row.phase, AskPhase::Open);
        // ev 夹具信封时间恒 0:deadline = 0 + timeoutSec
        assert_eq!(
            row.deadline_ms,
            Some(30_000),
            "deadline = 信封时间 + timeoutSec"
        );
        // 无 callId(骑通道)不建行
        st.apply(&ev(
            "ask/requested",
            11,
            json!({ "id": "r2", "timeoutSec": -1, "questions": [] }),
        ));
        assert_eq!(st.asks.len(), 1);
        // 超时 → Continued
        st.apply(&ev(
            "ask/timed-out",
            50,
            json!({ "id": "r1", "callId": "7" }),
        ));
        let row = st.asks.get("call:7").expect("行仍在");
        assert_eq!(row.phase, AskPhase::Continued);
        assert_eq!(row.deadline_ms, None, "转持续清倒计时");
        // 迟到应答 → Answered + 答案
        st.apply(&ev(
            "ask/answered",
            60,
            json!({
                "id": "r1", "callId": "7", "late": true,
                "answers": [ { "id": "q1", "selected": ["是"] } ],
            }),
        ));
        let row = st.asks.get("call:7").expect("行仍在");
        assert_eq!(row.phase, AskPhase::Answered);
        assert_eq!(
            row.answers
                .as_ref()
                .and_then(|a| a[0]["selected"][0].as_str()),
            Some("是")
        );
        // 取消相位(另一行)
        st.apply(&ev(
            "ask/requested",
            70,
            json!({ "id": "r3", "callId": "9", "timeoutSec": -1, "questions": [] }),
        ));
        st.apply(&ev(
            "ask/cancelled",
            80,
            json!({ "id": "r3", "callId": "9" }),
        ));
        assert_eq!(
            st.asks.get("call:9").map(|r| r.phase),
            Some(AskPhase::Cancelled)
        );
    }

    /// 迟到应答通知(ask-answer steer 的 user/message)不建节点:答案
    /// 归宿是 ask 行摘要与只读回看,转录不重复渲染注入文本。
    #[test]
    fn late_answer_notice_not_rendered() {
        let mut st = ChatState::default();
        st.apply(&ev(
            "user/message",
            1,
            json!({
                "id": "notice-late",
                "role": "user",
                "content": [ {
                    "type": "text",
                    "text": "{\"kind\":\"answer_to_pending_question\",\"callId\":\"7\"}"
                } ],
                "source": { "kind": "ask-answer", "callId": "7", "outcome": "answered" },
            }),
        ));
        assert!(st.nodes.is_empty(), "迟到应答通知零节点");
    }

    /// 被杀回合(日志无 `turn/end`)重放后,未落定的调用必须冻结为
    /// `Stopped`。
    ///
    /// 否则它永远停 `Running`,而渲染层为 Running 挂的是 `repeat()` 扫光
    /// 动画——一个这样的节点就让整窗 60fps 永久重绘(实测 CPU 60%+、
    /// 空闲不停)。真机取证:会话日志末尾 `tool/call` 之后直接是
    /// `decision/answered`,既无 `tool/result` 也无 `turn/end`
    #[test]
    fn killed_turn_freezes_unfinished_calls() {
        let mut st = ChatState::default();
        st.merge_history(vec![
            ev("turn/start", 1, json!({ "turn": 1 })),
            ev("step/start", 2, json!({ "turn": 1, "step": 1 })),
            ev(
                "tool/call",
                3,
                json!({ "name": "bash", "arguments": {}, "callId": "c1" }),
            ),
            // 日志到此为止:没有 tool/result、没有 turn/end
        ]);
        let state_of = |st: &ChatState| {
            st.nodes.iter().find_map(|n| match n {
                ChatNode::Tool { state, .. } => Some(*state),
                _ => None,
            })
        };
        assert_eq!(
            state_of(&st),
            Some(ToolState::Running),
            "重放后本就应停在执行中(引擎未收尾)"
        );
        st.freeze_unfinished_calls();
        assert_eq!(
            state_of(&st),
            Some(ToolState::Stopped),
            "冻结后不得再留 Running(否则扫光动画永久烧 CPU)"
        );
        // 已有终局的调用不受影响
        let mut done = ChatState::default();
        done.merge_history(vec![
            ev("turn/start", 1, json!({ "turn": 1 })),
            ev(
                "tool/call",
                2,
                json!({ "turn": 1, "step": 1, "callId": "c9",
                        "name": "bash", "arguments": "{}" }),
            ),
            ev(
                "tool/result",
                3,
                json!({
                    "turn": 1, "step": 1,
                    "message": { "id": "m9", "role": "user", "content": [ {
                        "type": "tool-result", "toolCallId": "c9",
                        "content": [ { "type": "text", "text": "ok" } ],
                        "isError": false } ] },
                }),
            ),
        ]);
        let before = state_of(&done);
        done.freeze_unfinished_calls();
        assert_eq!(state_of(&done), before, "已落定的调用不应被改写");
    }

    #[test]
    fn streaming_chunks_accumulate() {
        let mut st = ChatState::default();
        st.apply(&ev(
            "assistant/chunk",
            1,
            json!({ "turn": 1, "step": 1, "chunk": { "text": "ab" } }),
        ));
        st.apply(&ev(
            "assistant/chunk",
            2,
            json!({ "turn": 1, "step": 1, "chunk": { "text": "cd" } }),
        ));
        assert_eq!(st.nodes.len(), 1);
        match &st.nodes[0] {
            ChatNode::Assistant {
                text, streaming, ..
            } => {
                assert_eq!(text, "abcd");
                assert!(streaming);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn tool_error_pairs() {
        let mut st = ChatState::default();
        st.apply(&ev(
            "tool/call",
            1,
            json!({ "turn": 1, "step": 1, "callId": "1", "name": "bash", "arguments": "{}" }),
        ));
        st.apply(&ev("tool/result", 2, json!({
            "message": { "content": [ { "type": "tool-result", "toolCallId": "1", "isError": true,
                "content": [ { "type": "text", "text": "boom" } ] } ] },
        })));
        match &st.nodes[0] {
            ChatNode::Tool { state, output, .. } => {
                assert_eq!(*state, ToolState::Error);
                assert_eq!(output.as_deref(), Some("boom"));
            }
            other => panic!("{other:?}"),
        }
    }

    /// present 宣告收集:成功 result 的 present 视图 → TurnTail 交付
    #[test]
    fn present_result_collects_deliverable() {
        let mut st = ChatState::default();
        st.apply(&ev(
            "tool/call",
            1,
            json!({ "turn": 1, "step": 1, "callId": "c1", "name": "present", "arguments": "{}" }),
        ));
        st.apply(&ev("tool/result", 2, json!({
            "message": { "content": [ { "type": "tool-result", "toolCallId": "c1", "isError": false,
                "content": [ { "type": "text", "text": "presented a.md" } ] } ] },
            "view": { "card": "present", "path": "a.md", "description": "季度汇总" },
        })));
        st.apply(&ev(
            "turn/end",
            3,
            json!({ "turn": 1, "reason": { "kind": "done" } }),
        ));
        match &st.nodes.last() {
            Some(ChatNode::TurnTail { deliverables, .. }) => {
                assert_eq!(
                    deliverables,
                    &vec![Deliverable {
                        path: "a.md".into(),
                        description: Some("季度汇总".into()),
                        stat: None,
                        announced: true,
                    }]
                );
            }
            other => panic!("{other:?}"),
        }
    }

    /// 去重:同 path 重复宣告保留首见位置、description 后到覆盖
    #[test]
    fn present_dedup_keeps_first_position_last_description() {
        let mut st = ChatState::default();
        st.apply(&ev(
            "tool/call",
            1,
            json!({ "turn": 1, "step": 1, "callId": "c1", "name": "present", "arguments": "{}" }),
        ));
        st.apply(&ev("tool/result", 2, json!({
            "message": { "content": [ { "type": "tool-result", "toolCallId": "c1", "isError": false,
                "content": [ { "type": "text", "text": "ok" } ] } ] },
            "view": { "card": "present", "path": "a.md", "description": "v1" },
        })));
        st.apply(&ev(
            "tool/call",
            3,
            json!({ "turn": 1, "step": 2, "callId": "c2", "name": "present", "arguments": "{}" }),
        ));
        st.apply(&ev("tool/result", 4, json!({
            "message": { "content": [ { "type": "tool-result", "toolCallId": "c2", "isError": false,
                "content": [ { "type": "text", "text": "ok" } ] } ] },
            "view": { "card": "present", "path": "b.md", "description": null },
        })));
        st.apply(&ev(
            "tool/call",
            5,
            json!({ "turn": 1, "step": 3, "callId": "c3", "name": "present", "arguments": "{}" }),
        ));
        st.apply(&ev("tool/result", 6, json!({
            "message": { "content": [ { "type": "tool-result", "toolCallId": "c3", "isError": false,
                "content": [ { "type": "text", "text": "ok" } ] } ] },
            "view": { "card": "present", "path": "a.md", "description": "v2 定稿" },
        })));
        st.apply(&ev(
            "turn/end",
            7,
            json!({ "turn": 1, "reason": { "kind": "done" } }),
        ));
        match &st.nodes.last() {
            Some(ChatNode::TurnTail { deliverables, .. }) => {
                assert_eq!(
                    deliverables,
                    &vec![
                        Deliverable {
                            path: "a.md".into(),
                            description: Some("v2 定稿".into()),
                            stat: None,
                            announced: true,
                        },
                        Deliverable {
                            path: "b.md".into(),
                            description: None,
                            stat: None,
                            announced: true,
                        },
                    ]
                );
            }
            other => panic!("{other:?}"),
        }
    }

    /// 失败宣告(文件不存在等)不进产物网格
    #[test]
    fn present_error_result_not_collected() {
        let mut st = ChatState::default();
        st.apply(&ev(
            "tool/call",
            1,
            json!({ "turn": 1, "step": 1, "callId": "c1", "name": "present", "arguments": "{}" }),
        ));
        st.apply(&ev("tool/result", 2, json!({
            "message": { "content": [ { "type": "tool-result", "toolCallId": "c1", "isError": true,
                "content": [ { "type": "text", "text": "not accessible" } ] } ] },
            "view": { "card": "present", "path": "gone.md" },
        })));
        st.apply(&ev(
            "turn/end",
            3,
            json!({ "turn": 1, "reason": { "kind": "done" } }),
        ));
        match &st.nodes.last() {
            Some(ChatNode::TurnTail { deliverables, .. }) => {
                assert!(deliverables.is_empty(), "{deliverables:?}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// 产物单一来源 = present 宣告:diff 视图不再派生交付(旧分支移除
    /// 的回归锁,防回潮)
    #[test]
    fn diff_falls_back_with_stat() {
        let mut st = ChatState::default();
        st.apply(&ev(
            "tool/call",
            1,
            json!({ "turn": 1, "step": 1, "callId": "c1", "name": "file_edit", "arguments": "{}" }),
        ));
        st.apply(&ev("tool/result", 2, json!({
            "message": { "content": [ { "type": "tool-result", "toolCallId": "c1", "isError": false,
                "content": [ { "type": "text", "text": "ok" } ] } ] },
            "view": { "card": "diff", "diffs": [ { "path": "a.md", "oldText": null, "newText": "x\ny\nz" } ] },
        })));
        st.apply(&ev(
            "turn/end",
            3,
            json!({ "turn": 1, "reason": { "kind": "done" } }),
        ));
        match &st.nodes.last() {
            Some(ChatNode::TurnTail { deliverables, .. }) => {
                assert_eq!(
                    deliverables,
                    &vec![Deliverable {
                        path: "a.md".into(),
                        description: None,
                        stat: Some((3, 0)),
                        announced: false,
                    }]
                );
            }
            other => panic!("{other:?}"),
        }
    }

    /// 两层互补:present 宣告后再编辑同文件 → 描述保留、统计更新
    #[test]
    fn present_and_diff_merge_orthogonally() {
        let mut st = ChatState::default();
        st.apply(&ev(
            "tool/call",
            1,
            json!({ "turn": 1, "step": 1, "callId": "c1", "name": "present", "arguments": "{}" }),
        ));
        st.apply(&ev("tool/result", 2, json!({
            "message": { "content": [ { "type": "tool-result", "toolCallId": "c1", "isError": false,
                "content": [ { "type": "text", "text": "ok" } ] } ] },
            "view": { "card": "present", "path": "a.md", "description": "季度汇总" },
        })));
        st.apply(&ev(
            "tool/call",
            3,
            json!({ "turn": 1, "step": 2, "callId": "c2", "name": "file_edit", "arguments": "{}" }),
        ));
        st.apply(&ev("tool/result", 4, json!({
            "message": { "content": [ { "type": "tool-result", "toolCallId": "c2", "isError": false,
                "content": [ { "type": "text", "text": "ok" } ] } ] },
            "view": { "card": "diff", "diffs": [ { "path": "a.md", "oldText": "old line", "newText": "n1\nn2" } ] },
        })));
        st.apply(&ev(
            "turn/end",
            5,
            json!({ "turn": 1, "reason": { "kind": "done" } }),
        ));
        match &st.nodes.last() {
            Some(ChatNode::TurnTail { deliverables, .. }) => {
                assert_eq!(
                    deliverables,
                    &vec![Deliverable {
                        path: "a.md".into(),
                        description: Some("季度汇总".into()),
                        stat: Some((2, 1)),
                        announced: true,
                    }]
                );
            }
            other => panic!("{other:?}"),
        }
    }

    /// git 快照兜底:turn/end 载荷 changes → 变更层(path 已在集合
    /// 则跳过,present/diff 优先)
    #[test]
    fn turn_end_changes_payload_merges() {
        let mut st = ChatState::default();
        st.apply(&ev(
            "tool/call",
            1,
            json!({ "turn": 1, "step": 1, "callId": "c1", "name": "present", "arguments": "{}" }),
        ));
        st.apply(&ev("tool/result", 2, json!({
            "message": { "content": [ { "type": "tool-result", "toolCallId": "c1", "isError": false,
                "content": [ { "type": "text", "text": "ok" } ] } ] },
            "view": { "card": "present", "path": "a.md", "description": "宣告的" },
        })));
        st.apply(&ev(
            "turn/end",
            3,
            json!({
                "turn": 1, "reason": { "kind": "done" },
                "changes": [
                    { "path": "a.md" },
                    { "path": "bash-made.txt", "added": 2, "removed": 0 },
                ],
            }),
        ));
        match &st.nodes.last() {
            Some(ChatNode::TurnTail { deliverables, .. }) => {
                // a.md 已被宣告(描述优先)不重复;bash-made.txt 兜底进
                assert_eq!(
                    deliverables,
                    &vec![
                        Deliverable {
                            path: "a.md".into(),
                            description: Some("宣告的".into()),
                            stat: None,
                            announced: true,
                        },
                        Deliverable {
                            path: "bash-made.txt".into(),
                            description: None,
                            stat: Some((2, 0)),
                            announced: false,
                        },
                    ]
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn aborted_tail_and_idempotent_push() {
        let mut st = ChatState::default();
        st.apply(&ev("turn/start", 1, json!({ "turn": 1 })));
        assert!(st.running);
        st.apply(&ev(
            "turn/end",
            2,
            json!({ "turn": 1, "reason": { "kind": "aborted", "reason": { "kind": "legacy" } } }),
        ));
        assert!(matches!(
            &st.nodes[0],
            ChatNode::TurnTail { aborted: true, .. }
        ));
        // 重放同 key 幂等
        st.apply(&ev(
            "turn/end",
            2,
            json!({ "turn": 1, "reason": { "kind": "aborted" } }),
        ));
        assert_eq!(st.nodes.len(), 1);
    }

    /// todos 的 turn 生命周期:turn/start 清空、
    /// turn/end 保留 —— 回合结束清单仍可见,新回合开始才重置
    #[test]
    fn todos_cleared_on_turn_start_kept_on_turn_end() {
        let mut st = ChatState::default();
        st.apply(&ev(
            "todo/write",
            1,
            json!({ "todos": [ { "content": "a", "status": "pending" } ] }),
        ));
        st.apply(&ev(
            "turn/end",
            2,
            json!({ "turn": 1, "reason": { "kind": "done" } }),
        ));
        assert_eq!(st.todos.len(), 1, "turn/end 保留清单");
        st.apply(&ev("turn/start", 3, json!({ "turn": 2 })));
        assert!(st.todos.is_empty(), "turn/start 清空清单");
    }

    #[test]
    fn spliced_inserted_and_todo_plan() {
        let mut st = ChatState::default();
        // splice 的 inserted 是队列簿记,不渲染为对话节点
        // (排队条目经 session/queue 帧呈现,认领后 user/message 渲染)
        st.apply(&ev(
            "agent/inbox/spliced",
            1,
            json!({
                "target": "next-turn", "start": 0, "removedCount": 0,
                "inserted": [ { "id": "q1", "content": "排队消息" } ],
            }),
        ));
        assert!(st.nodes.is_empty(), "splice inserted 不产生聊天节点");
        st.apply(&ev(
            "todo/write",
            2,
            json!({ "todos": [
            { "content": "a", "status": "completed" },
            { "content": "b", "status": "pending" },
        ] }),
        ));
        assert_eq!(st.todos.len(), 2);
        assert_eq!(st.todos[0].status, "completed");
        st.apply(&ev("plan/mode", 3, json!({ "active": true })));
        assert!(st.plan_mode);
    }

    /// 计划归档卡状态流转:submitted 落 Pending;declined → Declined
    /// (拒绝留在 plan 模式,徽标「已拒绝」);再次提交 → 新 Pending
    #[test]
    fn plan_archive_status_flips_with_terminal_events() {
        let mut st = ChatState::default();
        st.apply(&ev("plan/submitted", 1, json!({ "plan": "# 方案" })));
        match st.nodes.last() {
            Some(ChatNode::Plan { status, .. }) => {
                assert_eq!(*status, PlanStatus::Pending);
            }
            other => panic!("应落计划卡:{other:?}"),
        }
        st.apply(&ev(
            "plan/declined",
            2,
            json!({ "plan": "# 方案", "feedback": "改用 OAuth" }),
        ));
        match st.nodes.last() {
            Some(ChatNode::Plan { status, .. }) => assert_eq!(*status, PlanStatus::Declined),
            other => panic!("计划卡应在场:{other:?}"),
        }
        // 拒绝后再提交 → 新的待批卡
        st.apply(&ev("plan/submitted", 3, json!({ "plan": "# 方案 v2" })));
        assert!(
            st.nodes
                .iter()
                .filter(|n| matches!(n, ChatNode::Plan { .. }))
                .count()
                == 2,
            "两次提交两张卡"
        );
        match st.nodes.last() {
            Some(ChatNode::Plan { status, .. }) => assert_eq!(*status, PlanStatus::Pending),
            other => panic!("新卡应为待批:{other:?}"),
        }
        st.apply(&ev("plan/approved", 4, json!({ "plan": "# 方案 v2" })));
        match st.nodes.last() {
            Some(ChatNode::Plan { status, .. }) => assert_eq!(*status, PlanStatus::Approved),
            other => panic!("终态应为已批准:{other:?}"),
        }
    }

    #[test]
    fn summary_truncates_cjk() {
        let long = "很".repeat(100);
        assert_eq!(truncate_chars(&long, 80).chars().count(), 81); // 80 + 省略号
        assert_eq!(
            summarize_call(liuma_sandbox::shell::tool_name(), "{\"command\":\"ls\"}"),
            "ls"
        );
    }

    /// 调用派生计划卡:exit_plan_mode 的 tool/call 即出卡
    /// (Draft);plan/submitted 就近转正不重复推卡,正文以事件为权威;
    /// 错误结果把未见 submitted 的草稿卡收口 Failed(提交失败的正文
    /// 仍可见);参数解析失败与非计划调用不出卡;旧日志兜底落新卡
    #[test]
    fn plan_card_derives_from_call_and_survives_failure() {
        let mut st = ChatState::default();
        st.apply(&ev(
            "tool/call",
            1,
            json!({
                "callId": "c1", "name": "exit_plan_mode",
                "arguments": "{\"plan\": \"# 方案 步骤\"}",
            }),
        ));
        match st.nodes.last() {
            Some(ChatNode::Plan {
                key,
                plan,
                status: PlanStatus::Draft,
            }) => {
                assert_eq!(key, "plan-call:c1");
                assert_eq!(plan, "# 方案 步骤");
            }
            other => panic!("调用即出草稿卡:{other:?}"),
        }
        st.apply(&ev("plan/submitted", 2, json!({ "plan": "# 方案 v2" })));
        assert_eq!(
            st.nodes
                .iter()
                .filter(|n| matches!(n, ChatNode::Plan { .. }))
                .count(),
            1,
            "同一提交只一张卡"
        );
        match st.nodes.last() {
            Some(ChatNode::Plan {
                plan,
                status: PlanStatus::Pending,
                ..
            }) => assert_eq!(plan, "# 方案 v2", "正文以 submitted 为权威"),
            other => panic!("应转待批:{other:?}"),
        }
        st.apply(&ev("plan/approved", 3, json!({ "plan": "# 方案 v2" })));
        assert!(matches!(
            st.nodes.last(),
            Some(ChatNode::Plan {
                status: PlanStatus::Approved,
                ..
            })
        ));

        // 失败路径:调用出卡 → 错误结果收口 Failed(无 submitted)
        st.apply(&ev(
            "tool/call",
            4,
            json!({
                "callId": "c2", "name": "exit_plan_mode",
                "arguments": "{\"plan\": \"## 小标题\"}",
            }),
        ));
        assert!(matches!(
            st.nodes.last(),
            Some(ChatNode::Plan {
                status: PlanStatus::Draft,
                ..
            })
        ));
        st.apply(&ev(
            "tool/result",
            5,
            json!({
                "message": { "content": [ { "toolCallId": "c2", "isError": true,
                    "content": [ { "type": "text", "text": "requires a # heading" } ] } ] },
            }),
        ));
        match st.nodes.last() {
            Some(ChatNode::Plan {
                plan,
                status: PlanStatus::Failed,
                ..
            }) => assert_eq!(plan, "## 小标题", "失败提交的正文保留可见"),
            other => panic!("错误结果应收口 Failed:{other:?}"),
        }

        // 参数解析失败不出卡;非 exit_plan_mode 不出卡
        st.apply(&ev(
            "tool/call",
            6,
            json!({
                "callId": "c3", "name": "exit_plan_mode", "arguments": "{broken",
            }),
        ));
        st.apply(&ev(
            "tool/call",
            7,
            json!({
                "callId": "c4", "name": "bash", "arguments": "{\"command\": \"ls\"}",
            }),
        ));
        assert_eq!(
            st.nodes
                .iter()
                .filter(|n| matches!(n, ChatNode::Plan { .. }))
                .count(),
            2,
            "解析失败与非计划调用不出卡"
        );
        // 旧日志兜底:无前置调用的 submitted 仍落新卡
        st.apply(&ev("plan/submitted", 8, json!({ "plan": "# 旧日志" })));
        let last = st.nodes.last().unwrap();
        assert!(
            matches!(last, ChatNode::Plan { key, status: PlanStatus::Pending, .. } if key.starts_with("plan:")),
            "无草稿卡退回落新卡:{last:?}"
        );
    }

    /// 折叠行摘要键序:file_edit 显 path(修 new_text 错显)、
    /// file_search content 优先、workflow 取 steps 首元素;
    /// bash 优先 description(必填意图说明),
    /// 旧日志无 description 回退 command
    #[test]
    fn summary_key_order_per_tool() {
        assert_eq!(
            summarize_call(
                liuma_sandbox::shell::tool_name(),
                "{\"command\":\"git status\",\"description\":\"Show working tree status\"}"
            ),
            "Show working tree status"
        );
        assert_eq!(
            summarize_call(
                liuma_sandbox::shell::tool_name(),
                "{\"command\":\"git status\"}"
            ),
            "git status"
        );
        assert_eq!(
            summarize_call(
                "file_edit",
                "{\"new_text\":\"fn main() {}\",\"old_text\":\"…\",\"path\":\"src/main.rs\"}"
            ),
            "src/main.rs"
        );
        assert_eq!(
            summarize_call(
                "file_search",
                "{\"content\":\"needle\",\"glob\":\"**/*.rs\"}"
            ),
            "needle"
        );
        assert_eq!(
            summarize_call("file_search", "{\"glob\":\"**/*.rs\",\"path\":\"/w\"}"),
            "**/*.rs"
        );
        assert_eq!(
            summarize_call("workflow", "{\"steps\":[\"第一步 规划\",\"第二步 执行\"]}"),
            "第一步 规划"
        );
        assert_eq!(
            summarize_call("subagent", "{\"task\":\"调查布局\"}"),
            "调查布局"
        );
        assert_eq!(
            summarize_call("goal", "{\"action\":\"add\",\"task\":\"t\"}"),
            "add"
        );
        // todo_write 无通用键序:回落参数首串(行专属摘要另由 todo_row_summary)
        assert_eq!(
            summarize_call(
                "todo_write",
                "{\"todos\":[{\"content\":\"c\",\"status\":\"pending\"}]}"
            ),
            "c"
        );
        // 未列工具:回落参数首串
        assert_eq!(summarize_call("whatever", "{\"x\":\"首串\"}"), "首串");
    }

    /// 工具配对 + 渲染意图投影:call 侧意图在场、result 侧权威替换、
    /// 失败清除(call 意图不残留)
    #[test]
    fn tool_pair_carries_view() {
        let mut st = ChatState::default();
        st.apply(&ev(
            "tool/call",
            1,
            json!({ "turn": 1, "step": 1, "callId": "7", "name": "bash",
                    "arguments": "{\"command\":\"ls -la\"}" }),
        ));
        st.apply(&ev(
            "tool/result",
            2,
            json!({
                "turn": 1, "step": 1,
                "message": { "content": [ {
                    "type": "tool-result", "toolCallId": "7",
                    "content": [ { "type": "text", "text": "file-a" } ],
                    "isError": false,
                } ] },
                "view": { "card": "terminal", "exitCode": 0, "signal": null, "cwd": "/w/proj" },
            }),
        ));
        match &st.nodes[0] {
            ChatNode::Tool {
                state,
                output,
                view,
                ..
            } => {
                assert!(matches!(state, ToolState::Done));
                assert_eq!(output.as_deref(), Some("file-a"));
                let v = view.as_ref().expect("result 侧视图在场");
                assert_eq!(v["card"], "terminal");
                assert_eq!(v["exitCode"], 0);
                assert_eq!(v["cwd"], "/w/proj");
            }
            other => panic!("工具节点缺失:{other:?}"),
        }

        // call 侧意图(file_edit 运行中)+ 失败清除
        st.apply(&ev(
            "tool/call",
            3,
            json!({ "turn": 1, "step": 2, "callId": "9", "name": "file_edit",
                    "arguments": "{\"path\":\"a.txt\",\"old_text\":\"o\",\"new_text\":\"n\"}",
                    "view": { "card": "diff", "diffs": [ { "path": "a.txt",
                        "oldText": "o", "newText": "n" } ] } }),
        ));
        match &st.nodes[1] {
            ChatNode::Tool { view, .. } => {
                assert_eq!(view.as_ref().unwrap()["card"], "diff", "call 侧意图在场");
            }
            other => panic!("{other:?}"),
        }
        st.apply(&ev(
            "tool/result",
            4,
            json!({
                "turn": 1, "step": 2,
                "message": { "content": [ {
                    "type": "tool-result", "toolCallId": "9",
                    "content": [ { "type": "text", "text": "old_text not found" } ],
                    "isError": true,
                } ] },
            }),
        ));
        match &st.nodes[1] {
            ChatNode::Tool { state, view, .. } => {
                assert!(matches!(state, ToolState::Error));
                assert!(view.is_none(), "失败无视图 → 通用卡");
            }
            other => panic!("{other:?}"),
        }
    }

    /// 多行 bash 命令的折叠摘要恒单行(空白序列折一空格)
    #[test]
    fn summary_collapses_multiline_command() {
        let script = "for f in $(ls *.rs); do\n  rustfmt --edition 2024 \"$f\" && \\\n    echo ok \"$f\"\ndone";
        let summary = summarize_call(
            "bash",
            &format!("{{\"command\":{}}}", serde_json::json!(script)),
        );
        assert!(!summary.contains('\n'), "摘要应单行:{summary}");
        assert!(
            summary.starts_with("for f in $(ls *.rs); do rustfmt"),
            "空白应折一空格:{summary}"
        );
    }

    /// todo_write 行摘要:解析该次调用 args(计数 + 首个进行中 + 并行额外数;
    /// 坏 JSON / 非数组 / 非对象项 → None)
    #[test]
    fn todo_row_summary_from_call_args() {
        let s = todo_row_summary(
            r#"{"todos": [
            {"content": "任务一", "status": "completed"},
            {"content": "任务二", "status": "in_progress"},
            {"content": "任务三", "status": "in_progress"}
        ]}"#,
        )
        .unwrap();
        assert_eq!(s.text, "1/3 已完成 · 任务二");
        assert_eq!(s.extra, 1, "并行额外数独立于可截断文本");

        // 无进行中:仅计数
        let s = todo_row_summary(
            r#"{"todos": [{"content": "a", "status": "completed"}, {"content": "b", "status": "pending"}]}"#,
        )
        .unwrap();
        assert_eq!(s.text, "1/2 已完成");
        assert_eq!(s.extra, 0);

        // 首个进行中内容空白:仅计数,不挂名不计数额外
        let s =
            todo_row_summary(r#"{"todos": [{"content": " ", "status": "in_progress"}]}"#).unwrap();
        assert_eq!(s.text, "0/1 已完成");
        assert_eq!(s.extra, 0);

        // 坏输入 → None(回落通用摘要)
        assert!(todo_row_summary("not json").is_none());
        assert!(todo_row_summary(r#"{"action": "add"}"#).is_none());
        assert!(todo_row_summary(r#"{"todos": [42]}"#).is_none());
    }

    /// 工作区根相对化:根内绝对路径剥前缀,其余原样
    #[test]
    fn relativize_strips_workspace_root() {
        assert_eq!(
            relativize(Some("/w/proj"), "/w/proj/src/main.rs"),
            "src/main.rs"
        );
        assert_eq!(relativize(Some("/w/proj/"), "/w/proj/a.txt"), "a.txt");
        assert_eq!(relativize(Some("/w/proj"), "/other/a.txt"), "/other/a.txt");
        assert_eq!(relativize(None, "/w/proj/a.txt"), "/w/proj/a.txt");
        assert_eq!(relativize(Some("/w/proj"), "相对路径.rs"), "相对路径.rs");
    }

    /// node_born(入场动画年龄门控素材):直播帧记录;历史合并载入不记
    /// (载入行不做入场动画);合并收尾清孤儿键(防跨会话累积)。
    #[test]
    fn node_born_live_only_and_orphan_prune() {
        let mut st = ChatState::default();
        st.apply(&ev(
            "user/message",
            2,
            json!({
                "id": "u1", "role": "user",
                "content": [ { "type": "text", "text": "hi" } ],
                "source": { "kind": "user" },
            }),
        ));
        st.apply(&ev(
            "assistant/chunk",
            3,
            json!({ "turn": 1, "step": 1, "chunk": { "type": "text-delta", "index": 0, "text": "he" } }),
        ));
        assert!(st.node_born.contains_key("user:2"), "直播 user 应记 born");
        assert!(st.node_born.contains_key("a:1:1"), "流式首建应记 born");

        // 纯历史载入:全程 merging → 不记
        let mut hist = ChatState::default();
        hist.merge_history(full_turn());
        assert!(hist.node_born.is_empty(), "历史载入不应记 born");

        // 折叠合并到已有直播态:既有 born 保留,历史新增节点不记
        st.merge_history(full_turn());
        assert!(st.node_born.contains_key("user:2"));
        assert!(!st.node_born.contains_key("call:8"), "历史补入不记 born");
        assert!(!st.node_born.contains_key("turn-end:11"));

        // 孤儿清理:折叠掉(或手工塞入)的键不残留
        st.node_born
            .insert("call:ghost".into(), std::time::Instant::now());
        st.merge_history(vec![]);
        assert!(!st.node_born.contains_key("call:ghost"));
    }

    // ---- 行槽派生层(build_row_slots)----

    use std::collections::HashSet as SlotSet;

    /// 简写断言:槽序列 → 可读形(t = 轮控制行,u = 放开限高的组)
    fn slot_shapes(slots: &[RowSlot]) -> Vec<String> {
        slots
            .iter()
            .map(|s| match s {
                RowSlot::Node { ix, part } => match part {
                    NodePart::Full => format!("n{ix}"),
                    NodePart::Reasoning => format!("r{ix}"),
                    NodePart::Body => format!("b{ix}"),
                },
                RowSlot::TurnHead { turn_key, .. } => format!("t[{turn_key}]"),
                RowSlot::Group {
                    turn_key,
                    first,
                    last,
                    open,
                    ..
                } => {
                    // 大写 = 展开态(组体可见),小写 = 收拢成一行组头
                    let head = if *open { "G" } else { "g" };
                    let key = if turn_key == LIVE_TURN_KEY {
                        "live"
                    } else {
                        turn_key.as_str()
                    };
                    format!("{head}[{first}..={last}]{key}")
                }
            })
            .collect()
    }

    /// 全收(无放开组)的行槽
    fn slots_all_folded(nodes: &[ChatNode], open_turns: &SlotSet<String>) -> Vec<RowSlot> {
        build_row_slots(nodes, open_turns, &SlotSet::new())
    }

    /// 事件序列 → 投影节点
    fn project(events: Vec<SessionEvent>) -> Vec<ChatNode> {
        let mut st = ChatState::default();
        for e in &events {
            st.apply(e);
        }
        st.nodes
    }

    /// 通知不产节点后的轮次形状:通知+答案的轮(轮 1)按普通轮折叠;
    /// 通知-only 段(轮 2)只剩答案——平铺可见,不产「已思考」空组头
    #[test]
    fn settlement_notice_turns_collapse_like_plain_turns() {
        let none = SlotSet::new();
        let evs = vec![
            // 轮 1:工具 + 中途通知(shell job 结算)+ 答案
            ev("turn/start", 1, json!({ "turn": 1 })),
            ev(
                "user/message",
                2,
                json!({ "content": [ { "type": "text", "text": "run it" } ] }),
            ),
            ev(
                "assistant/message",
                3,
                json!({
                    "turn": 1, "step": 1,
                    "message": { "id": "m1", "role": "assistant",
                        "content": [ { "type": "text", "text": "starting" } ] },
                }),
            ),
            ev(
                "tool/call",
                4,
                json!({ "turn": 1, "step": 2, "callId": "8", "name": "bash", "arguments": "{}" }),
            ),
            ev(
                "user/message",
                5,
                json!({
                    "content": [ { "type": "text", "text": "Background job 1 (x) finished successfully." } ],
                    "source": { "kind": "shell-job-settled", "form": "notice" },
                }),
            ),
            ev(
                "assistant/message",
                6,
                json!({
                    "turn": 1, "step": 2,
                    "message": { "id": "m2", "role": "assistant",
                        "content": [ { "type": "text", "text": "job settled, all good" } ] },
                }),
            ),
            ev(
                "turn/end",
                7,
                json!({ "turn": 1, "reason": { "kind": "done" } }),
            ),
            // 轮 2:通知-only 段(通知唤醒的续答轮)
            ev(
                "user/message",
                8,
                json!({
                    "content": [ { "type": "text", "text": "Background subagent s-1 finished." } ],
                    "source": { "kind": "subagent-settled", "form": "notice" },
                }),
            ),
            ev(
                "assistant/message",
                9,
                json!({
                    "turn": 2, "step": 1,
                    "message": { "id": "m3", "role": "assistant",
                        "content": [ { "type": "text", "text": "noted" } ] },
                }),
            ),
            ev(
                "turn/end",
                10,
                json!({ "turn": 2, "reason": { "kind": "done" } }),
            ),
        ];
        let nodes = project(evs);
        // 通知零节点:流里只剩用户消息/助手/工具/收尾
        assert!(
            !nodes.iter().any(|n| matches!(n, ChatNode::Context { .. })),
            "通知不得产出 Context 节点"
        );

        let slots = slots_all_folded(&nodes, &none);
        let shapes = slot_shapes(&slots);
        // 节点序(通知剔除后):user0 a1(starting) tool2 a3(答案)
        // tail4 | a5(noted) tail6。两轮都收口 → 各一行轮控制行;
        // 收拢态只收过程组,回复正文(中途叙述 + 最终答复)照旧平铺
        assert_eq!(
            shapes,
            vec![
                "n0",             // 用户消息
                "t[turn-end:7]",  // 轮 1 控制行
                "b1",             // 轮 1 中途叙述
                "b3",             // 轮 1 最终答复
                "n4",             // 轮 1 收尾行
                "t[turn-end:10]", // 轮 2 控制行(纯答案轮同样有)
                "b5",             // 轮 2 答复
                "n6",             // 轮 2 收尾行
            ],
            "通知剔除后的槽形状: {shapes:?}"
        );
    }

    /// 一轮多组回归锁:**每段回复正文切一个组**,所以一轮可以有多个组,
    /// 不是「一轮一个组」——2 轮、第 1 轮两次回复 + steering(用户中途
    /// 插话),共 3 个组。带正文的助手步即刻收口当前过程运行并自成一个
    /// 平铺正文节点。
    #[test]
    fn reply_text_segments_split_groups() {
        let nodes = vec![
            ChatNode::User {
                key: "user:1".into(),
                text: "old question".into(),
                images: vec![],
                files: Vec::new(),
                time: 0,
            },
            ChatNode::Assistant {
                key: "a:1:1".into(),
                text: String::new(),
                text_ver: 0,
                reasoning: "old analysis".into(),
                streaming: false,
                usage: None,
                message_id: String::new(),
            },
            ChatNode::Assistant {
                key: "a:1:2".into(),
                text: "old progress".into(),
                text_ver: 1,
                reasoning: String::new(),
                streaming: false,
                usage: None,
                message_id: String::new(),
            },
            ChatNode::User {
                key: "user:4".into(),
                text: "old direction".into(),
                images: vec![],
                files: Vec::new(),
                time: 0,
            },
            ChatNode::Assistant {
                key: "a:1:3".into(),
                text: String::new(),
                text_ver: 0,
                reasoning: "old follow-up analysis".into(),
                streaming: false,
                usage: None,
                message_id: String::new(),
            },
            ChatNode::Assistant {
                key: "a:1:4".into(),
                text: "old answer".into(),
                text_ver: 1,
                reasoning: String::new(),
                streaming: false,
                usage: None,
                message_id: String::new(),
            },
            ChatNode::User {
                key: "user:8".into(),
                text: "current question".into(),
                images: vec![],
                files: Vec::new(),
                time: 0,
            },
            ChatNode::Assistant {
                key: "a:2:1".into(),
                text: String::new(),
                text_ver: 0,
                reasoning: "current analysis".into(),
                streaming: false,
                usage: None,
                message_id: String::new(),
            },
        ];
        let none = SlotSet::new();
        let shapes = slot_shapes(&slots_all_folded(&nodes, &none));
        let groups: Vec<&String> = shapes.iter().filter(|s| s.starts_with('g')).collect();
        assert_eq!(groups.len(), 3, "2 轮 → 3 个组,实得 {shapes:?}");
        assert_eq!(
            shapes,
            vec![
                "n0",
                "g[1..=1]live",
                "n2",
                "n3",
                "g[4..=4]live",
                "n5",
                "n6",
                "g[7..=7]live"
            ],
            "回复正文切开组,steering 点也切开;尾段组未收口"
        );
    }

    /// 组不许多切:纯 tool_calls 的助手步(正文与思考都空)是**过程**
    /// 不是边界——一段「命令 + 空步 + 读文件」必须只产出一个组头。
    /// 回归锁:此前把空助手步当边界,同一段被拆成「执行了命令」
    /// 「已读取文件」两个组,整轮组数随空步数量漂移。
    #[test]
    fn empty_assistant_steps_never_split_a_group() {
        let evs = vec![
            ev("turn/start", 1, json!({ "turn": 1 })),
            ev("user/message", 2, json!({ "content": "接下来做什么" })),
            ev(
                "tool/call",
                3,
                json!({ "turn": 1, "step": 1, "callId": "a", "name": "bash", "arguments": "{}" }),
            ),
            // 纯 tool_calls 步(无正文、无思考)
            ev(
                "assistant/message",
                4,
                json!({
                    "turn": 1, "step": 1,
                    "message": { "id": "m1", "role": "assistant",
                        "content": [ { "type": "tool-call", "id": "b", "name": "read",
                            "arguments": "{}" } ] },
                }),
            ),
            ev(
                "tool/call",
                5,
                json!({ "turn": 1, "step": 2, "callId": "b", "name": "file_read", "arguments": "{}" }),
            ),
            ev(
                "assistant/message",
                6,
                json!({ "turn": 1, "step": 2,
                    "message": { "id": "m2", "role": "assistant",
                        "content": [ { "type": "text", "text": "" } ] } }),
            ),
            ev(
                "turn/end",
                7,
                json!({ "turn": 1, "reason": { "kind": "done" } }),
            ),
        ];
        let nodes = project(evs);
        // 0 user / 1 call:a / 2 空步 / 3 call:b / 4 空答案步 / 5 tail
        let none = SlotSet::new();
        assert_eq!(
            slot_shapes(&slots_all_folded(&nodes, &none)),
            vec!["n0", "t[turn-end:7]", "n5"],
            "无正文轮:控制行 + 收尾行;两个工具属同一组(折叠态不可见)"
        );
        let mut open = SlotSet::new();
        open.insert("turn-end:7".to_string());
        assert_eq!(
            slot_shapes(&slots_all_folded(&nodes, &open)),
            vec!["n0", "t[turn-end:7]", "g[1..=4]turn-end:7", "n5"],
            "命令 + 空步 + 读文件必须同属一个组"
        );
        // 组头分类:执行了命令 + 已读取文件 → 并词
        assert_eq!(
            group_categories(&nodes, 1, 4),
            vec![ProcessActivity::Commands, ProcessActivity::Read]
        );
    }

    /// 标准轮回放回归锁:一轮内「思考组 + 各类工具组」与 5 段回复正文
    /// 交错,投影后的组序列与组头并词逐字锁定——
    ///
    ///   user / 轮控制行 / 已完成分析 / 正文 / 执行了命令，已调用工具，
    ///   已读取文件 / 正文 / 执行了命令并已调用工具 / 正文 / 执行了命令
    ///   ×2 与正文交错 / 收尾
    ///
    /// 覆盖:纯思考组(组内无工具 → 「已完成分析」)、三类并词(逗号连
    /// 接,计数降序、同数按首现序)、两类并词(「并」连接,首项不带
    /// 「已」前缀时第二项保留前缀)。**不得**改成「一轮一组」或按空助
    /// 手步切组——那会改变组数与组头文案。
    #[test]
    fn standard_turn_replay_locks_group_titles_and_rows() {
        // 带正文回复步(切组边界)
        fn reply_step(evs: &mut Vec<SessionEvent>, seq: &mut u64, step: u64, text: &str) {
            *seq += 1;
            evs.push(ev(
                "assistant/message",
                *seq,
                json!({
                    "turn": 1, "step": step,
                    "message": { "id": format!("m{step}"), "role": "assistant",
                        "content": [ { "type": "text", "text": text } ] },
                }),
            ));
        }
        // 过程步:空正文助手步 + 可选思考 + 工具调用(每调用独立 callId)
        fn process_step(
            evs: &mut Vec<SessionEvent>,
            seq: &mut u64,
            step: u64,
            think: &str,
            tools: &[&str],
        ) {
            *seq += 1;
            evs.push(ev(
                "assistant/message",
                *seq,
                json!({
                    "turn": 1, "step": step,
                    "message": { "id": format!("m{step}"), "role": "assistant",
                        "content": [ { "type": "text", "text": "" } ] },
                }),
            ));
            if !think.is_empty() {
                *seq += 1;
                evs.push(ev(
                    "assistant/reasoning",
                    *seq,
                    json!({ "turn": 1, "step": step, "text": think }),
                ));
            }
            for &name in tools {
                *seq += 1;
                evs.push(ev(
                    "tool/call",
                    *seq,
                    json!({
                        "turn": 1, "step": step, "callId": format!("c{}", *seq),
                        "name": name, "arguments": "{}"
                    }),
                ));
            }
        }
        let mut evs = vec![
            ev("turn/start", 1, json!({ "turn": 1 })),
            ev(
                "user/message",
                2,
                json!({ "content": [ { "type": "text", "text": "把构建问题再查一遍" } ] }),
            ),
        ];
        let (mut seq, mut step) = (evs.len() as u64, 0u64);
        step += 1;
        process_step(&mut evs, &mut seq, step, "先梳理构建链路", &[]);
        step += 1;
        reply_step(&mut evs, &mut seq, step, "构建链路梳理完毕");
        // 命令×2 压序,未归类工具、读文件各一 → 三类并词按 计数,首现序
        step += 1;
        process_step(
            &mut evs,
            &mut seq,
            step,
            "",
            &["bash", "run_tests", "bash", "file_read"],
        );
        step += 1;
        reply_step(&mut evs, &mut seq, step, "第一段进展");
        step += 1;
        process_step(&mut evs, &mut seq, step, "", &["bash", "run_tests"]);
        step += 1;
        reply_step(&mut evs, &mut seq, step, "第二段进展");
        step += 1;
        process_step(&mut evs, &mut seq, step, "", &["bash", "bash"]);
        step += 1;
        reply_step(&mut evs, &mut seq, step, "第三段进展");
        step += 1;
        process_step(&mut evs, &mut seq, step, "", &["bash"]);
        step += 1;
        reply_step(&mut evs, &mut seq, step, "最终结论");
        seq += 1;
        evs.push(ev(
            "turn/end",
            seq,
            json!({ "turn": 1, "reason": { "kind": "done" } }),
        ));
        let nodes = project(evs);
        // 展开态(turn 点开):控制行 + 组与正文交错
        let mut open = SlotSet::new();
        for n in &nodes {
            if let ChatNode::TurnTail {
                key,
                aborted: false,
                ..
            } = n
            {
                open.insert(key.clone());
            }
        }
        let slots = build_row_slots(&nodes, &open, &SlotSet::new());
        let titles: Vec<String> = slots
            .iter()
            .filter_map(|s| match s {
                RowSlot::Group {
                    first,
                    last,
                    closed,
                    ..
                } => {
                    assert!(*closed, "该轮已收口,组应为 closed");
                    Some(process_title(&group_categories(&nodes, *first, *last)))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            titles,
            vec![
                "已完成分析",
                "执行了命令，已调用工具，已读取文件",
                "执行了命令并已调用工具",
                "执行了命令",
                "执行了命令",
            ],
            "组头并词文案逐字锁定"
        );
        let replies = slots
            .iter()
            .filter(|s| {
                matches!(
                    s,
                    RowSlot::Node { ix, .. } if matches!(nodes.get(*ix), Some(ChatNode::Assistant { text, .. }) if !text.is_empty())
                )
            })
            .count();
        assert_eq!(replies, 5, "5 段回复正文各自平铺在组外");
        // 收拢态:过程整段收起,只留控制行 + 回复正文 + 收尾
        let folded = build_row_slots(&nodes, &SlotSet::new(), &SlotSet::new());
        assert!(
            !folded.iter().any(|s| matches!(s, RowSlot::Group { .. })),
            "收拢态不得残留组座位"
        );
        assert!(folded.iter().any(|s| matches!(s, RowSlot::TurnHead { .. })));
        assert_eq!(folded.len(), 8, "user + 控制行 + 5 段回复 + 收尾");
    }

    /// 基本收拢:think-only + 工具收进组行;正文与收尾行在外;
    /// 直播段(无收口)平铺;open_turns 展开 = 组头 + 成员全平铺
    #[test]
    fn row_slots_collapse_open_and_live() {
        let mut evs = vec![
            ev("turn/start", 1, json!({ "turn": 1 })),
            ev(
                "user/message",
                2,
                json!({ "content": [ { "type": "text", "text": "hi" } ] }),
            ),
        ];
        evs.push(ev(
            "assistant/message",
            3,
            json!({
                "turn": 1, "step": 1,
                "message": { "id": "m1", "role": "assistant",
                    "content": [ { "type": "text", "text": "" } ] },
            }),
        ));
        evs.push(ev(
            "assistant/reasoning",
            4,
            json!({ "turn": 1, "step": 1, "text": "思考中" }),
        ));
        evs.push(ev(
            "tool/call",
            5,
            json!({ "turn": 1, "step": 2, "callId": "8", "name": "bash", "arguments": "{}" }),
        ));
        evs.push(ev(
            "turn/end",
            6,
            json!({ "turn": 1, "reason": { "kind": "done" } }),
        ));
        let nodes = project(evs);
        // 节点序:user:2, a:1:1(空正文+reasoning → 过程项), call:8, turn-end:6
        assert_eq!(nodes.len(), 4);
        assert!(is_process_node(&nodes[1]) && is_process_node(&nodes[2]));

        // open_turns 空 = 全收:控制行在场,过程项收进折叠;无定稿答复
        // (本段助手正文为空)故只余控制行与收尾行
        let none = SlotSet::new();
        assert_eq!(
            slot_shapes(&slots_all_folded(&nodes, &none)),
            vec!["n0", "t[turn-end:6]", "n3"]
        );

        // 展开:控制行 + 组座位(成员收进座位体,仅收尾行在外)
        let mut open = SlotSet::new();
        open.insert("turn-end:6".to_string());
        assert_eq!(
            slot_shapes(&slots_all_folded(&nodes, &open)),
            vec!["n0", "t[turn-end:6]", "g[1..=2]turn-end:6", "n3"]
        );

        // 直播段(截掉收尾事件):无控制行,过程直接成组
        let live = &nodes[..3];
        assert_eq!(
            slot_shapes(&slots_all_folded(live, &none)),
            vec!["n0", "g[1..=2]live"]
        );
        assert_eq!(
            slot_shapes(&build_row_slots(live, &none, &SlotSet::new())),
            vec!["n0", "g[1..=2]live"]
        );
    }

    /// 活动归类:shell 模型面名(随平台)+ 各工具族 + 兜底
    #[test]
    fn activity_of_maps_tool_families() {
        assert_eq!(
            activity_of(liuma_sandbox::shell::tool_name()),
            ProcessActivity::Commands
        );
        assert_eq!(activity_of("bash"), ProcessActivity::Commands);
        assert_eq!(activity_of("file_read"), ProcessActivity::Read);
        assert_eq!(activity_of("read"), ProcessActivity::Read);
        assert_eq!(activity_of("read_image"), ProcessActivity::ReadImage);
        assert_eq!(activity_of("grep"), ProcessActivity::Search);
        assert_eq!(activity_of("glob"), ProcessActivity::Search);
        assert_eq!(activity_of("file_search"), ProcessActivity::Search);
        assert_eq!(activity_of("write"), ProcessActivity::Write);
        assert_eq!(activity_of("file_edit"), ProcessActivity::Edit);
        assert_eq!(activity_of("code"), ProcessActivity::Code);
        assert_eq!(activity_of("web_search"), ProcessActivity::WebSearch);
        assert_eq!(activity_of("web_fetch"), ProcessActivity::WebFetch);
        assert_eq!(activity_of("subagent"), ProcessActivity::Subagents);
        assert_eq!(activity_of("subagent_list"), ProcessActivity::Subagents);
        assert_eq!(activity_of("todo_write"), ProcessActivity::Plan);
        assert_eq!(activity_of("goal"), ProcessActivity::Plan);
        assert_eq!(activity_of("ask"), ProcessActivity::Questions);
        assert_eq!(activity_of("ask_user_question"), ProcessActivity::Questions);
        assert_eq!(activity_of("present"), ProcessActivity::Tools);
        assert_eq!(activity_of("nonexistent"), ProcessActivity::Tools);
    }

    /// 组头详情:展示键序取首个非空值;字符串数组拼接;空白折叠;
    /// 超长按字符截断加省略号(截断点去尾空白)
    #[test]
    fn live_detail_follows_key_priority_and_truncates() {
        assert_eq!(live_detail("{}"), None);
        assert_eq!(live_detail("not json"), None);
        assert_eq!(
            live_detail(r#"{"command":"ls -la","title":"Find jar"}"#),
            Some("Find jar".to_string()),
            "title 先于 command"
        );
        assert_eq!(
            live_detail(r#"{"command":"ls   -la\n"}"#),
            Some("ls -la".to_string()),
            "空白折叠去尾"
        );
        assert_eq!(
            live_detail(r#"{"queries":["a","b"]}"#),
            Some("a, b".to_string())
        );
        assert_eq!(
            live_detail(r#"{"title":"  "}"#),
            None,
            "纯空白值跳过,继续后键"
        );
        let long = "字".repeat(200);
        let got = live_detail(&format!(r#"{{"description":"{long}"}}"#)).unwrap();
        assert_eq!(got.chars().count(), 160);
        assert!(got.ends_with('…'));
        let spaced = format!("{}  ", "a".repeat(159));
        let got = live_detail(&format!(r#"{{"description":"{spaced}b"}}"#)).unwrap();
        assert!(got.ends_with('…'), "截断点尾空白应去除: {got:?}");
        assert!(!got.ends_with(" …"));
    }

    /// 组头分类计数:按调用数降序,同数按首现(决定文案里谁领起)
    #[test]
    fn group_categories_rank_by_count_then_first_seen() {
        let mk = |name: &str| ChatNode::Tool {
            key: format!("call:{name}"),
            name: name.into(),
            summary: String::new(),
            state: ToolState::Done,
            arguments: "{}".into(),
            output: None,
            view: None,
            images: Vec::new(),
        };
        let nodes = vec![
            mk("bash"),
            mk("grep"),
            mk("bash"),
            mk("file_read"),
            mk("bash"),
        ];
        assert_eq!(
            group_categories(&nodes, 0, 4),
            vec![
                ProcessActivity::Commands,
                ProcessActivity::Search,
                ProcessActivity::Read
            ]
        );
    }

    /// 收口组头文案:单类直出、两类并词(同前缀去重)、三类逗号、
    /// 超过三类「等」;无工具 → 「已完成分析」
    #[test]
    fn process_title_composes_categories() {
        use ProcessActivity as A;
        assert_eq!(process_title(&[]), "已完成分析");
        assert_eq!(process_title(&[A::Commands]), "执行了命令");
        assert_eq!(
            process_title(&[A::Commands, A::Read]),
            "执行了命令并已读取文件",
            "首项不以「已」开头 → 第二项保留前缀"
        );
        assert_eq!(
            process_title(&[A::Read, A::Search]),
            "已读取文件并搜索代码",
            "同「已」前缀 → 第二项去前缀"
        );
        // 三类以上只把首字母小写(中文无变化),前缀一律保留
        assert_eq!(
            process_title(&[A::Read, A::Search, A::Commands]),
            "已读取文件，已搜索代码，执行了命令"
        );
        assert_eq!(
            process_title(&[A::Read, A::Search, A::Commands, A::Plan]),
            "已读取文件，已搜索代码，执行了命令，更新了计划等"
        );
    }

    /// 组默认收拢成一行;点开才产展开槽
    #[test]
    fn open_groups_expand_the_seat() {
        let nodes = vec![
            ChatNode::User {
                key: "user:1".into(),
                text: "hi".into(),
                images: vec![],
                files: Vec::new(),
                time: 0,
            },
            ChatNode::Tool {
                key: "call:a".into(),
                name: "bash".into(),
                summary: String::new(),
                state: ToolState::Done,
                arguments: "{}".into(),
                output: None,
                view: None,
                images: Vec::new(),
            },
        ];
        let mut groups = SlotSet::new();
        groups.insert("g:1".to_string());
        assert_eq!(
            slot_shapes(&build_row_slots(&nodes, &SlotSet::new(), &groups)),
            vec!["n0", "G[1..=1]live"],
            "展开态标记为 G"
        );
        assert_eq!(
            slot_shapes(&build_row_slots(&nodes, &SlotSet::new(), &SlotSet::new())),
            vec!["n0", "g[1..=1]live"],
            "默认收拢只占一行组头"
        );
    }

    /// 流式思考摘要:首行收行前为空;随段落收行逐段前进;`**` 去除
    #[test]
    fn streaming_think_summary_advances_per_paragraph() {
        assert_eq!(streaming_think_summary("还没写完的一行"), "");
        assert_eq!(streaming_think_summary("第一行\n第二行还在写"), "第一行");
        assert_eq!(streaming_think_summary("第一段\n\n第二段开头"), "第一段");
        assert_eq!(
            streaming_think_summary("第一段\n\n第二段首行\n第二段次行"),
            "第二段首行"
        );
        assert_eq!(
            streaming_think_summary("`code` 与 **加粗** 混排\n后续行"),
            "`code` 与 加粗 混排",
            "只去 `**`,反引号原样"
        );
        assert_eq!(streaming_think_summary(""), "");
        assert_eq!(
            streaming_think_summary("\n\n\n第二段"),
            "",
            "空段不参与,唯一行未收行为空"
        );
    }

    /// 定稿思考摘要:首个非空行(去 `**`)
    #[test]
    fn settled_think_summary_takes_first_nonempty_line() {
        assert_eq!(settled_think_summary("首行\n次行"), "首行");
        assert_eq!(settled_think_summary("\n \n次行"), "次行");
        assert_eq!(settled_think_summary("**结论** 是对的"), "结论 是对的");
        assert_eq!(settled_think_summary(""), "");
    }

    /// 直播组头:运行中工具 → 归类 + 参数详情;无运行工具 → thinking
    /// + 最后一段已完成思考首行
    #[test]
    fn live_header_prefers_running_tool_then_reasoning() {
        let mk_tool = |name: &str, state: ToolState, args: &str| ChatNode::Tool {
            key: format!("call:{name}"),
            name: name.to_string(),
            summary: String::new(),
            state,
            arguments: args.to_string(),
            output: None,
            view: None,
            images: Vec::new(),
        };
        let mk_asst = |reasoning: &str| ChatNode::Assistant {
            key: "a:1:1".into(),
            text: String::new(),
            text_ver: 0,
            reasoning: reasoning.to_string(),
            streaming: true,
            usage: None,
            message_id: String::new(),
        };
        // 运行中工具优先(即使更早有思考)
        let nodes = vec![
            mk_asst("想想\n\n第二段"),
            mk_tool("bash", ToolState::Running, r#"{"description":"Find jar"}"#),
        ];
        assert_eq!(
            live_header(&nodes, 0, 1),
            (ProcessActivity::Commands, Some("Find jar".to_string()))
        );
        // 无运行工具 → thinking + 最后一段已完成思考首行(流式尾段
        // 未收行不参与)
        let nodes = vec![mk_asst("想想\n\n第二段开头还没完")];
        assert_eq!(
            live_header(&nodes, 0, 0),
            (ProcessActivity::Thinking, Some("想想".to_string()))
        );
        // 无运行工具且无已完成思考行 → 详情缺席(标题单独成立)
        let nodes = vec![mk_asst("还没收行")];
        assert_eq!(live_header(&nodes, 0, 0), (ProcessActivity::Thinking, None));
        // 运行中工具缺详情 → 不回退思考
        let nodes = vec![mk_asst("想想"), mk_tool("grep", ToolState::Running, "{}")];
        assert_eq!(live_header(&nodes, 0, 1), (ProcessActivity::Search, None));
        // 已落定工具不算运行中
        let nodes = vec![mk_tool(
            "bash",
            ToolState::Done,
            r#"{"description":"done cmd"}"#,
        )];
        assert_eq!(live_header(&nodes, 0, 0), (ProcessActivity::Thinking, None));
    }

    /// 直播段混入中断死段:中断收口(TurnTail aborted)界前恒平铺,
    /// 界后过程项才归直播组——「中断不折叠」语义不被直播组吞掉
    #[test]
    fn live_group_starts_after_aborted_boundary() {
        let mut evs = vec![
            ev("turn/start", 1, json!({ "turn": 1 })),
            ev("user/message", 2, json!({ "content": "第一问" })),
            ev(
                "assistant/reasoning",
                3,
                json!({ "turn": 1, "step": 1, "text": "想想" }),
            ),
            ev(
                "tool/call",
                4,
                json!({ "turn": 1, "step": 1, "callId": "1", "name": "bash", "arguments": "{}" }),
            ),
            ev(
                "turn/end",
                5,
                json!({ "turn": 1, "reason": { "kind": "aborted" } }),
            ),
            ev("turn/start", 6, json!({ "turn": 2 })),
            ev("user/message", 7, json!({ "content": "第二问" })),
            ev(
                "assistant/reasoning",
                8,
                json!({ "turn": 2, "step": 1, "text": "再想" }),
            ),
            ev(
                "tool/call",
                9,
                json!({ "turn": 2, "step": 1, "callId": "2", "name": "grep", "arguments": "{}" }),
            ),
        ];
        let nodes = project(std::mem::take(&mut evs));
        // 0 user1, 1 think1, 2 tool1, 3 tail(aborted), 4 user2, 5 think2, 6 tool2
        let none = SlotSet::new();
        // 中断轮也成组(控制行「已停止」+ 组,恒展开);界后直播段
        // 无控制行,过程直接成组
        assert_eq!(
            slot_shapes(&slots_all_folded(&nodes, &none)),
            vec![
                "n0",
                "t[turn-end:5]",
                "g[1..=2]turn-end:5",
                "n3",
                "n4",
                "g[5..=6]live"
            ]
        );
    }

    /// 零高节点口径:空正文+无思考的定稿 Assistant(纯 tool_calls 步
    /// 占位)= 不可见;有正文/有思考/流式中都可见
    #[test]
    fn invisible_node_only_for_empty_finalized_assistant() {
        let mk = |text: &str, reasoning: &str, streaming: bool| ChatNode::Assistant {
            key: "a:1:1".into(),
            text: text.into(),
            text_ver: 1,
            reasoning: reasoning.into(),
            streaming,
            usage: None,
            message_id: String::new(),
        };
        assert!(invisible_node(&mk("", "", false)));
        assert!(!invisible_node(&mk("先看一下", "", false)));
        assert!(!invisible_node(&mk("", "思考中", false)));
        assert!(!invisible_node(&mk("", "", true)));
        assert!(!invisible_node(&ChatNode::User {
            key: "user:1".into(),
            text: "hi".into(),
            images: vec![],
            files: Vec::new(),
            time: 0,
        }));
    }

    /// attach 基线注入先于轮次开始:注入行 + 用户消息平铺(用户消息
    /// 不被组区间收掉),组只收用户消息之后的过程项。真实会话形态:
    /// AGENTS.md 基线在 attach 时落档,首条用户消息晚于它——组区间
    /// 跨越用户消息会把轮次开始折叠进「思考与工具」(实测回归)。
    #[test]
    fn row_slots_baseline_injection_before_first_user_stays_flat() {
        let evs = vec![
            // attach 基线注入(无 turn,先于首条用户消息)
            ev(
                "user/message",
                1,
                json!({
                    "content": [ { "type": "text", "text": "AGENTS.md 基线" } ],
                    "source": { "kind": "agent-instructions" },
                }),
            ),
            ev("turn/start", 2, json!({ "turn": 1 })),
            ev("user/message", 3, json!({ "content": "规划接下来的开发" })),
            // 轮内注入(runtime context,过程项)
            ev(
                "user/message",
                4,
                json!({
                    "content": [ { "type": "text", "text": "runtime context" } ],
                    "source": { "kind": "plugin" },
                }),
            ),
            ev(
                "tool/call",
                5,
                json!({ "turn": 1, "step": 1, "callId": "a", "name": "bash", "arguments": "{}" }),
            ),
            // 最终答复
            ev(
                "assistant/message",
                6,
                json!({
                    "turn": 1, "step": 2,
                    "message": { "id": "m1", "role": "assistant",
                        "content": [ { "type": "text", "text": "计划如下" } ] },
                }),
            ),
            ev(
                "turn/end",
                7,
                json!({ "turn": 1, "reason": { "kind": "done" } }),
            ),
        ];
        let nodes = project(evs);
        // 0 基线注入, 1 用户消息, 2 轮内注入, 3 call, 4 答复, 5 turn-end
        assert_eq!(nodes.len(), 6);
        let none = SlotSet::new();
        // 收拢:基线注入与用户消息在组外,控制行 + 最终答复
        assert_eq!(
            slot_shapes(&slots_all_folded(&nodes, &none)),
            vec!["n0", "n1", "t[turn-end:7]", "b4", "n5"]
        );
        // 展开态:成员收进座位体,基线注入与用户消息保持组外
        let mut open = SlotSet::new();
        open.insert("turn-end:7".to_string());
        assert_eq!(
            slot_shapes(&slots_all_folded(&nodes, &open)),
            vec![
                "n0",
                "n1",
                "t[turn-end:7]",
                "g[2..=3]turn-end:7",
                "n4",
                "n5"
            ]
        );
    }

    /// 错误收口轮:仍按过程运行成组(错误轮不折叠,过程行照常归组),
    /// 正文切开相邻两组,归属由错误行交代。
    /// **不得回退成平铺**:平铺会让行距从 6px 涨到 16px、并丢掉组头。
    #[test]
    fn row_slots_error_tail_turn_groups_like_plain_turns() {
        let mut evs = vec![
            ev("turn/start", 1, json!({ "turn": 1 })),
            ev("user/message", 2, json!({ "content": "hi" })),
            // 注入行(过程项)
            ev(
                "user/message",
                3,
                json!({
                    "content": [ { "type": "text", "text": "AGENTS.md" } ],
                    "source": { "kind": "agent-instructions" },
                }),
            ),
            // 中途正文(内容节点)
            ev(
                "assistant/message",
                4,
                json!({
                    "turn": 1, "step": 1,
                    "message": { "id": "m1", "role": "assistant",
                        "content": [ { "type": "text", "text": "先看一下" } ] },
                }),
            ),
            ev(
                "tool/call",
                5,
                json!({ "turn": 1, "step": 2, "callId": "a", "name": "bash", "arguments": "{}" }),
            ),
            // think-only(过程项)
            ev(
                "assistant/reasoning",
                6,
                json!({ "turn": 1, "step": 3, "text": "继续" }),
            ),
            ev(
                "assistant/message",
                7,
                json!({
                    "turn": 1, "step": 3,
                    "message": { "id": "m2", "role": "assistant",
                        "content": [ { "type": "text", "text": "" } ] },
                }),
            ),
            ev(
                "tool/call",
                8,
                json!({ "turn": 1, "step": 4, "callId": "b", "name": "goal", "arguments": "{}" }),
            ),
            // 错误收口:Notice 替代收尾行
            ev(
                "turn/end",
                9,
                json!({ "turn": 1, "reason": { "kind": "error", "error": { "message": "x" } } }),
            ),
        ];
        let nodes = project(std::mem::take(&mut evs));
        // 0 user, 1 ctx, 2 正文, 3 call:a, 4 think-only, 5 call:b, 6 notice
        let none = SlotSet::new();
        // 错误控制行 + 两个过程组(注入 / call+think+call),正文切开
        // 两组且留在组外
        assert_eq!(
            slot_shapes(&slots_all_folded(&nodes, &none)),
            vec![
                "n0",
                "t[turn-error:9]",
                "g[1..=1]turn-error:9",
                "n2",
                "g[3..=5]turn-error:9",
                "n6"
            ]
        );
    }

    /// 取消收尾(aborted TurnTail):过程照常成组且**不折叠**——
    /// 中断轮的结局由「已停止」pill 与轮控制行交代,过程不藏。**不得
    /// 回退成平铺**。
    #[test]
    fn row_slots_aborted_turn_groups_but_never_folds() {
        let mut evs = vec![
            ev("turn/start", 1, json!({ "turn": 1 })),
            ev("user/message", 2, json!({ "content": "接下来做什么" })),
        ];
        // 两个叙述步(reasoning + chunk 正文)
        for (seq, step, reasoning, text) in [
            (3, 1, "先看状态", "Let me understand the state."),
            (7, 2, "重新应用改动", "Let me re-apply the zip change."),
        ] {
            evs.push(ev(
                "assistant/reasoning",
                seq,
                json!({ "turn": 1, "step": step, "text": reasoning }),
            ));
            evs.push(ev(
                "assistant/chunk",
                seq + 1,
                json!({ "turn": 1, "step": step,
                    "chunk": { "type": "text-delta", "index": 0, "text": text } }),
            ));
            evs.push(ev(
                "tool/call",
                seq + 2,
                json!({ "turn": 1, "step": step, "callId": format!("{step}"), "name": "bash", "arguments": "{}" }),
            ));
        }
        // 软取消收尾(reason.kind=aborted → aborted TurnTail)
        evs.push(ev(
            "turn/end",
            10,
            json!({ "turn": 1, "reason": { "kind": "aborted" } }),
        ));
        let nodes = project(evs);
        // 0 user, 1 叙①, 2 tool1, 3 叙②, 4 tool2, 5 tail(aborted)
        // 正文切开过程组,思考行留组内;控制行恒在场且 open_turns
        // 为空也照样展开(中断轮不收)
        let none = SlotSet::new();
        assert_eq!(
            slot_shapes(&slots_all_folded(&nodes, &none)),
            vec![
                "n0",
                "t[turn-end:10]",
                "g[1..=1]turn-end:10",
                "b1",
                "g[2..=3]turn-end:10",
                "b3",
                "g[4..=4]turn-end:10",
                "n5"
            ]
        );
    }

    /// 中断/错误收口:未落定的调用显式翻成 Stopped(渲染层 amber
    /// 状态点、撤扫光),已配对成功/失败的调用不受影响;残留流式位
    /// 同步熄灭(半截正文冻结,pill/动作行定稿)
    #[test]
    fn aborted_flip_running_tools_to_stopped() {
        let mut st = ChatState::default();
        st.apply(&ev("turn/start", 1, json!({ "turn": 1 })));
        st.apply(&ev(
            "tool/call",
            2,
            json!({ "turn": 1, "step": 1, "callId": "a", "name": "bash", "arguments": "{}" }),
        ));
        st.apply(&ev(
            "tool/result",
            3,
            json!({
                "message": { "content": [ { "type": "tool-result", "toolCallId": "a",
                    "content": [ { "type": "text", "text": "ok" } ] } ] },
            }),
        ));
        st.apply(&ev(
            "tool/call",
            4,
            json!({ "turn": 1, "step": 2, "callId": "b", "name": "bash", "arguments": "{}" }),
        ));
        st.apply(&ev(
            "assistant/chunk",
            5,
            json!({ "turn": 1, "step": 3, "chunk": { "text": "半截" } }),
        ));
        st.apply(&ev(
            "turn/end",
            6,
            json!({ "turn": 1, "reason": { "kind": "aborted" } }),
        ));
        assert_eq!(
            [st.nodes[0].clone(), st.nodes[1].clone()]
                .iter()
                .map(|n| match n {
                    ChatNode::Tool { state, .. } => *state,
                    other => panic!("{other:?}"),
                })
                .collect::<Vec<_>>(),
            [ToolState::Done, ToolState::Stopped]
        );
        assert!(
            matches!(&st.nodes[2], ChatNode::Assistant { streaming: false, text, .. } if text == "半截"),
            "中断应冻结残留流式位"
        );
    }

    /// DeepSeek 真实形态:每步 reasoning+过渡文本。段内**最后一个**正文
    /// = 最终答复(留在外面),中间叙述连同思考一并收进组;
    /// 收口重置答案标记(答案判定不跨段)
    #[test]
    fn row_slots_interim_narration_folds_final_answer_stays() {
        let evs = vec![
            ev("turn/start", 1, json!({ "turn": 1 })),
            ev("user/message", 2, json!({ "content": "hi" })),
            // 中间叙述步①(思考+文本)
            ev(
                "assistant/reasoning",
                3,
                json!({ "turn": 1, "step": 1, "text": "先看状态" }),
            ),
            ev(
                "assistant/message",
                4,
                json!({
                    "turn": 1, "step": 1,
                    "message": { "id": "m1", "role": "assistant",
                        "content": [ { "type": "text", "text": "Let me check the state." } ] },
                }),
            ),
            ev(
                "tool/call",
                5,
                json!({ "turn": 1, "step": 2, "callId": "a", "name": "bash", "arguments": "{}" }),
            ),
            // 中间叙述步②
            ev(
                "assistant/reasoning",
                6,
                json!({ "turn": 1, "step": 3, "text": "跑测试" }),
            ),
            ev(
                "assistant/message",
                7,
                json!({
                    "turn": 1, "step": 3,
                    "message": { "id": "m2", "role": "assistant",
                        "content": [ { "type": "text", "text": "Run the tests now." } ] },
                }),
            ),
            // 最终答复(段内最后正文)
            ev(
                "assistant/message",
                8,
                json!({
                    "turn": 1, "step": 4,
                    "message": { "id": "m3", "role": "assistant",
                        "content": [ { "type": "text", "text": "全部通过,结论如下。" } ] },
                }),
            ),
            ev(
                "turn/end",
                9,
                json!({ "turn": 1, "reason": { "kind": "done" } }),
            ),
        ];
        let nodes = project(evs);
        // 0 user, 1 叙①(reasoning+text), 2 tool, 3 叙②, 4 答案, 5 tail
        let none = SlotSet::new();
        // 收拢:控制行 + 回复正文(折叠只收过程组;中途叙述与最终答复
        // 都是回复正文,照旧平铺)
        assert_eq!(
            slot_shapes(&slots_all_folded(&nodes, &none)),
            vec!["n0", "t[turn-end:9]", "b1", "b3", "b4", "n5"]
        );
        // 展开:正文切开过程组——叙①/叙② 各自成组,其思考行留在组内,
        // 正文半边平铺在组外
        let mut open = SlotSet::new();
        open.insert("turn-end:9".to_string());
        assert_eq!(
            slot_shapes(&slots_all_folded(&nodes, &open)),
            vec![
                "n0",
                "t[turn-end:9]",
                "g[1..=1]turn-end:9",
                "b1",
                "g[2..=3]turn-end:9",
                "b3",
                "n4",
                "n5"
            ]
        );
    }

    /// LLM 重试行投影:llm/retry 落等待行(直播记退避截止),started
    /// 翻转「已重试」,取消收尾翻转「已取消」;重放不记截止;过程项
    /// 归组(收口后随轮折叠)
    #[test]
    fn retry_row_state_evolution() {
        let mut st = ChatState::default();
        st.apply(&ev("turn/start", 1, json!({ "turn": 1 })));
        st.apply(&ev(
            "llm/retry",
            2,
            json!({
                "turn": 1, "step": 1, "retry": 1, "maxRetries": 5,
                "delayMs": 1500, "code": "TRANSPORT", "message": "连接失败",
            }),
        ));
        assert!(st.nodes.last().is_some_and(|n| matches!(
            n,
            ChatNode::Retry {
                state: RetryState::Waiting,
                retry: 1,
                max_retries: 5,
                delay_ms: 1500,
                ..
            }
        )));
        assert!(
            st.retry_deadlines.contains_key("retry:2"),
            "直播帧应记退避截止时刻"
        );
        // started:最后一个等待行翻转
        st.apply(&ev(
            "llm/retry-started",
            3,
            json!({ "turn": 1, "step": 1, "retry": 1 }),
        ));
        assert!(st.nodes.iter().all(|n| matches!(
            n,
            ChatNode::Retry {
                state: RetryState::Started,
                ..
            }
        )));

        // 等待中被取消 → 「已取消」
        let mut st2 = ChatState::default();
        st2.apply(&ev(
            "llm/retry",
            1,
            json!({ "retry": 1, "maxRetries": 5, "delayMs": 8000 }),
        ));
        st2.apply(&ev(
            "turn/end",
            2,
            json!({ "turn": 1, "reason": { "kind": "aborted", "reason": { "kind": "legacy" } } }),
        ));
        assert!(
            st2.nodes.iter().any(|n| matches!(
                n,
                ChatNode::Retry {
                    state: RetryState::Cancelled,
                    ..
                }
            )),
            "取消后等待行应翻转「已取消」"
        );

        // 重放路径:merge 不记截止时刻(倒计时为静态排定值)
        let mut hist = ChatState::default();
        hist.merge_history(vec![ev(
            "llm/retry",
            1,
            json!({ "retry": 1, "maxRetries": 5, "delayMs": 8000 }),
        )]);
        assert!(hist.retry_deadlines.is_empty(), "历史载入不记倒计时截止");
    }

    /// stream-reset 清空该步流式缓冲(重试丢弃语义):已拼 chunk 清空、
    /// 保持流式态,后续 chunk 从零续拼
    #[test]
    fn stream_reset_clears_streaming_buffer() {
        let mut st = ChatState::default();
        for text in ["残", "段"] {
            st.apply(&ev(
                "assistant/chunk",
                text.len() as u64,
                json!({ "turn": 1, "step": 1, "chunk": { "type": "text-delta", "index": 0, "text": text } }),
            ));
        }
        st.apply(&ev(
            "assistant/stream-reset",
            9,
            json!({ "turn": 1, "step": 1 }),
        ));
        match &st.nodes[0] {
            ChatNode::Assistant {
                text,
                reasoning: _,
                streaming,
                ..
            } => {
                assert!(text.is_empty(), "残段应被清空: {text}");
                assert!(*streaming, "清空后仍为流式态");
            }
            other => panic!("{other:?}"),
        }
        st.apply(&ev(
            "assistant/chunk",
            10,
            json!({ "turn": 1, "step": 1, "chunk": { "type": "text-delta", "index": 0, "text": "成功" } }),
        ));
        match &st.nodes[0] {
            ChatNode::Assistant { text, .. } => assert_eq!(text, "成功"),
            other => panic!("{other:?}"),
        }
    }

    /// 重试行 = 过程项:收口后随轮折叠进组(与 Think/工具同待遇)
    #[test]
    fn retry_row_folds_into_turn_group() {
        let evs = vec![
            ev("turn/start", 1, json!({ "turn": 1 })),
            ev("user/message", 2, json!({ "content": "hi" })),
            ev(
                "llm/retry",
                3,
                json!({
                    "turn": 1, "step": 1, "retry": 1, "maxRetries": 5,
                    "delayMs": 500, "code": "TIMEOUT", "message": "读超时",
                }),
            ),
            ev(
                "assistant/message",
                4,
                json!({
                    "turn": 1, "step": 1,
                    "message": { "id": "m1", "role": "assistant",
                        "content": [ { "type": "text", "text": "答案" } ] },
                }),
            ),
            ev(
                "turn/end",
                5,
                json!({ "turn": 1, "reason": { "kind": "completed" } }),
            ),
        ];
        let mut st = ChatState::default();
        for e in &evs {
            st.apply(e);
        }
        let nodes = st.nodes.clone();
        assert!(is_process_node(&nodes[1]), "重试行应为过程项");
        let none = SlotSet::new();
        // 0 user, 1 retry(过程), 2 答案, 3 tail → 收拢态:控制行 + 答复
        assert_eq!(
            slot_shapes(&slots_all_folded(&nodes, &none)),
            vec!["n0", "t[turn-end:5]", "b2", "n3"]
        );
        // 展开态:重试行随过程收进组
        let mut open = SlotSet::new();
        open.insert("turn-end:5".to_string());
        assert_eq!(
            slot_shapes(&slots_all_folded(&nodes, &open)),
            vec!["n0", "t[turn-end:5]", "g[1..=1]turn-end:5", "n2", "n3"]
        );
    }

    /// 导航锚点:**只有用户消息(轮次开始)**是锚点;答复/组行/收尾/
    /// 通告一概不是(导航 = 轮次间跳转)
    #[test]
    fn nav_anchors_pick_user_turn_starts() {
        let mut evs = vec![
            ev("turn/start", 1, json!({ "turn": 1 })),
            ev(
                "user/message",
                2,
                json!({ "content": [ { "type": "text", "text": "第一问" } ] }),
            ),
            ev(
                "assistant/reasoning",
                3,
                json!({ "turn": 1, "step": 1, "text": "想想" }),
            ),
            ev(
                "tool/call",
                4,
                json!({ "turn": 1, "step": 1, "callId": "1", "name": "bash", "arguments": "{}" }),
            ),
            ev("turn/end", 5, json!({ "turn": 1, "cancelled": "token" })),
            ev("turn/start", 6, json!({ "turn": 2 })),
            ev(
                "user/message",
                7,
                json!({ "content": [ { "type": "text", "text": "第二问" } ] }),
            ),
            ev(
                "assistant/reasoning",
                8,
                json!({ "turn": 2, "step": 1, "text": "再想" }),
            ),
            ev(
                "assistant/chunk",
                9,
                // 客方形态:翻译层把引擎 delta 包进 chunk.text(勿用原始流字段)
                json!({ "turn": 2, "step": 1, "chunk": { "type": "text-delta", "index": 0, "text": "中间叙述" } }),
            ),
            ev(
                "tool/call",
                10,
                json!({ "turn": 2, "step": 2, "callId": "2", "name": "bash", "arguments": "{}" }),
            ),
            ev(
                "assistant/chunk",
                11,
                json!({ "turn": 2, "step": 3, "chunk": { "type": "text-delta", "index": 0, "text": "最终答复内容" } }),
            ),
            ev(
                "turn/end",
                12,
                json!({ "turn": 2, "reason": { "kind": "done" } }),
            ),
        ];
        let nodes = project(std::mem::take(&mut evs));
        let none = SlotSet::new();
        let slots = slots_all_folded(&nodes, &none);
        let anchors = nav_anchors(&slots, &nodes);
        // 两轮 → 恰好两个用户消息锚点;key 为用户节点稳定 key
        assert_eq!(anchors.len(), 2);
        assert_eq!(anchors[0].slot_ix, Some(0));
        assert_eq!(anchors[0].key, "user:2");
        // 单行消息:标题 = 全文,预览为空(卡只出标题行)
        assert_eq!(anchors[0].title, "第一问");
        assert_eq!(anchors[0].preview, "");
        assert_eq!(anchors[1].slot_ix, Some(3));
        assert_eq!(anchors[1].key, "user:7");
        assert_eq!(anchors[1].title, "第二问");
        // 轮2 展开:成员行不占点,用户锚点仍在(槽位按当时行槽计)
        let mut open = SlotSet::new();
        open.insert("turn-end:12".to_string());
        let anchors_open = nav_anchors(&slots_all_folded(&nodes, &open), &nodes);
        assert_eq!(anchors_open.len(), 2, "展开不减用户锚点");
    }

    /// hover 卡两级拆分:标题 = 首行单行化截 48,预览 = 次行起拼接截
    /// 240;空行跳过,单行消息预览为空(截断带省略号)
    #[test]
    fn nav_anchor_title_preview_split() {
        let (t, p) = first_and_rest("第一行标题\n\n第二行正文\n第三行正文");
        assert_eq!(t, "第一行标题");
        assert_eq!(p, "第二行正文 第三行正文");
        let (t, p) = first_and_rest("只有一行");
        assert_eq!(t, "只有一行");
        assert_eq!(p, "");
        let (t, _) = first_and_rest(&"长".repeat(60));
        assert_eq!(t.chars().count(), 49, "超长首行截 48 + 省略号");
        let (_, p) = first_and_rest(&format!("标题\n{}", "正".repeat(300)));
        assert_eq!(p.chars().count(), 241, "预览截 240 + 省略号");
    }

    /// 节点 key 索引回归锁:多步交错流式的 chunk 各自命中正确节点,
    /// 索引与线性扫描同答案,同日志双次 merge 幂等。原 `position`
    /// 线性扫在万级节点 × 数万 chunk 下是 O(N²)(打开长会话主线程
    /// 冻结的第二来源),索引化后答案不得漂移。
    #[test]
    fn node_index_agrees_with_linear_scan_and_merge_is_idempotent() {
        // 60 步 × 每步 3 chunk 交错(交错保证定位跨已存在节点)
        let mut evs: Vec<SessionEvent> = vec![ev("turn/start", 1, json!({ "turn": 1 }))];
        let mut seq = 1u64;
        let mut expect: Vec<(String, String)> = Vec::new();
        for step in 1..=60u64 {
            for part in ["一", "二", "三"] {
                seq += 1;
                evs.push(ev(
                    "assistant/chunk",
                    seq,
                    json!({
                        "turn": 1, "step": step,
                        "chunk": { "type": "text-delta", "index": 0, "text": part },
                    }),
                ));
            }
            expect.push((format!("a:1:{step}"), "一二三".to_string()));
        }
        let mut state = ChatState::default();
        for e in &evs {
            state.apply(e);
        }
        // 每 chunk 落在各自 step 节点,全文按步拼接
        for (key, text) in &expect {
            let node = state.nodes.iter().find(|n| n.key() == key);
            let ChatNode::Assistant { text: got, .. } = node.expect("节点存在") else {
                panic!("{key} 不是 Assistant 节点");
            };
            assert_eq!(got, text, "{key} 正文拼接");
        }
        // 索引与线性扫描逐 key 同答案(不变式直锁)
        for (ix, node) in state.nodes.iter().enumerate() {
            assert_eq!(
                state.node_index.get(node.key()),
                Some(&ix),
                "{} 索引下标漂移",
                node.key()
            );
        }
        // merge_history 与逐事件 apply 同结果(索引不改变回放语义;
        // chunk 是增量 delta,重复 merge 本就叠加,不在此断言)
        let mut merged = ChatState::default();
        merged.merge_history(evs);
        assert_eq!(merged.nodes, state.nodes, "merge 与 apply 等价");
    }
}
