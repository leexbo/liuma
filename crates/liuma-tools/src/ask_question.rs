//! ask_user_question 工具。
//!
//! 模型调用 `ask_user_question` 向用户提一组问题(单选/多选/自定义),等待
//! 用户应答;两种等待语义由 `timeout` 参数决定:
//! - `timeout > 0`(缺省 120):计时等待——超时前应答则作为**同一 tool-call
//!   的 tool/result** 回填(`{"answers":[{id, selected[], custom?}]}`),模型
//!   继续同一 turn;超时则回填 `{"pending":true, callId, message}`,问题转
//!   continued(仍可答),迟到应答经 steer 通道以
//!   `answer_to_pending_question` 用户消息注入后续 step。
//! - `timeout = -1`:无限阻塞(仅在必须先有答案才能继续时使用)。
//!
//! 取消仅经会话中断;用户放弃(桌面 ✕)不结束请求,只收起卡。

use std::pin::Pin;
use std::sync::Arc;

use serde_json::{Value, json};

use liuma_agent_loop::{ToolCallRequest, ToolOutput, ToolPort};

/// 一个问题选项
#[derive(Debug, Clone)]
pub struct QuestionOption {
    /// 选项 label(用户可见;推荐项约定 label 后追加 "(Recommended)")
    pub label: String,
    /// 选项说明(权衡/影响;可缺省)
    pub description: Option<String>,
}

/// 一个问题项
#[derive(Debug, Clone)]
pub struct QuestionItem {
    /// 稳定 id(应答时原样回显)
    pub id: String,
    /// 问题文本
    pub question: String,
    /// 可选标题(heading,如 "Confirm"/"Choose Mode")
    pub header: Option<String>,
    /// 可选选项(单选默认;multi_select 时多选)
    pub options: Vec<QuestionOption>,
    /// 是否多选
    pub multi_select: bool,
}

/// 缺省计时等待(秒);`timeout` 入参缺席时生效
pub const DEFAULT_ASK_TIMEOUT_SEC: i64 = 120;

/// 用户应答端口(宿主注入;实现方:liuma-core AppHost)。
/// ask:落 pending → 广播 question/requested → await 用户 respond / 超时。
/// 返回 tool/result 文本:应答 = `{"answers":[...]}`;
/// `timeout_sec > 0` 且超时 = `{"pending":true, callId, message}`(问题转
/// continued,迟到应答走 steer);取消/中断 → Err(点名文案)。
pub trait AskQuestionPort: Send + Sync {
    /// 提问(单个 ask 可含多问题);`call_id` = tool/call 日志 seq(行键,
    /// 可空),`timeout_sec` = 等待秒数(-1 阻塞)。
    fn ask(
        &self,
        session_id: &str,
        call_id: &str,
        questions: &[QuestionItem],
        timeout_sec: i64,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;
}

/// ask_user_question 工具(第 13 个工具;standard preset 开启)
pub struct AskQuestionTool {
    port: Arc<dyn AskQuestionPort>,
    current: String,
}

impl AskQuestionTool {
    /// 构造(port = 宿主问答面;current = 归属会话 id)
    pub fn new(port: Arc<dyn AskQuestionPort>, current: &str) -> Self {
        Self {
            port,
            current: current.to_string(),
        }
    }
}

impl ToolPort for AskQuestionTool {
    fn specs(&self) -> Vec<Value> {
        vec![json!({
            "type": "function",
            "function": {
                "name": "ask_user_question",
                "description": "Ask the user a concise question when you need confirmation, a choice, or missing information before proceeding. Send one or more questions, each with a stable id that will be echoed in the answer. An answer item with an empty `selected` array means the user skipped the question: treat it as 'no answer' — never choose an option on the user's behalf and never continue the skipped decision; re-ask with different wording or stop and wait for the user's explicit direction. A `pending` result means no answer batch arrived before the timeout and the user can still answer; it is not a skipped answer: continue useful independent work, and do not treat it as permission — the user's reply will arrive as a user message identified as answer_to_pending_question with this callId.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "questions": {
                            "type": "array",
                            "description": "Questions to ask the user before continuing.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": { "type": "string", "description": "Stable id for this question; echoed in the answer." },
                                    "question": { "type": "string", "description": "The specific question to ask the user." },
                                    "header": { "type": "string", "description": "Optional short heading for the question, such as \"Confirm\" or \"Choose Mode\"." },
                                    "options": {
                                        "type": "array",
                                        "description": "Optional choices. If you recommend one, put it first and append \"(Recommended)\" to that label.",
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "label": { "type": "string", "description": "Short user-facing option label." },
                                                "description": { "type": "string", "description": "One sentence explaining the tradeoff or impact." }
                                            },
                                            "required": ["label"]
                                        }
                                    },
                                    "multi_select": { "type": "boolean", "description": "Whether the user may select more than one option. Defaults to false." }
                                },
                                "required": ["id", "question"]
                            }
                        },
                        "timeout": {
                            "type": "integer",
                            "description": "Wait seconds for the entire batch (default 120); omit unless the user specifies a duration. Use -1 only when an answer is required before proceeding."
                        }
                    },
                    "required": ["questions"]
                }
            }
        })]
    }

    fn execute(&mut self, call: &ToolCallRequest) -> impl Future<Output = ToolOutput> + Send {
        let port = self.port.clone();
        let current = self.current.clone();
        let call_id = call.id.clone();
        async move {
            let questions = match parse_questions(&call.arguments) {
                Ok(q) => q,
                Err(e) => {
                    return ToolOutput {
                        output: format!("ask_user_question 参数无效:{e}"),
                        success: false,
                        ..Default::default()
                    };
                }
            };
            let timeout_sec = match parse_timeout(&call.arguments) {
                Ok(t) => t,
                Err(e) => {
                    return ToolOutput {
                        output: format!("ask_user_question 参数无效:{e}"),
                        success: false,
                        ..Default::default()
                    };
                }
            };
            match port.ask(&current, &call_id, &questions, timeout_sec).await {
                Ok(text) => ToolOutput {
                    output: text,
                    success: true,
                    ..Default::default()
                },
                Err(e) => ToolOutput {
                    output: e,
                    success: false,
                    ..Default::default()
                },
            }
        }
    }
}

/// 从工具入参解析 questions(id/question 必填;options label 必填)。
/// arguments 容忍 JSON 字符串(与 BashTool/FileTools/todo 同策略):模型对话方言
/// 把 tool 参数作为字符串下发,先尝试解析,为对象则原样使用。
fn parse_questions(args: &Value) -> Result<Vec<QuestionItem>, String> {
    let args = args_or_json_string(args);
    let arr = args["questions"]
        .as_array()
        .ok_or_else(|| "缺少 questions 数组".to_string())?;
    if arr.is_empty() {
        return Err("questions 不能为空".to_string());
    }
    arr.iter()
        .enumerate()
        .map(|(i, q)| {
            // id 宽容兜底:缺席/空串自动生成 q1/q2…(应答回带同值,语义
            // 不损;部分模型无视 required 的 id,硬报错只会白耗一轮)
            let id = match q["id"].as_str() {
                Some(s) if !s.trim().is_empty() => s.to_string(),
                _ => format!("q{}", i + 1),
            };
            let question = q["question"]
                .as_str()
                .ok_or_else(|| "每个 question 需要 question 文本".to_string())?
                .to_string();
            let header = q["header"].as_str().map(String::from);
            let options = q["options"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|o| {
                            Ok(QuestionOption {
                                label: o["label"]
                                    .as_str()
                                    .ok_or_else(|| "选项需要 label".to_string())?
                                    .to_string(),
                                description: o["description"].as_str().map(String::from),
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()
                })
                .transpose()?
                .unwrap_or_default();
            let multi_select = q["multi_select"].as_bool().unwrap_or(false);
            Ok(QuestionItem {
                id,
                question,
                header,
                options,
                multi_select,
            })
        })
        .collect()
}

/// 把工具入参统一成对象:若为 JSON 字符串则解析,否则原样。
fn args_or_json_string(args: &Value) -> Value {
    if let Some(s) = args.as_str() {
        serde_json::from_str(s).unwrap_or_else(|_| json!({}))
    } else {
        args.clone()
    }
}

/// 解析 `timeout` 入参:-1(无限阻塞)或 1..=2147483 秒;缺席取缺省。
fn parse_timeout(args: &Value) -> Result<i64, String> {
    let args = args_or_json_string(args);
    match args.get("timeout") {
        None | Some(Value::Null) => Ok(DEFAULT_ASK_TIMEOUT_SEC),
        Some(v) => match v.as_i64() {
            Some(t) if t == -1 || (1..=2_147_483).contains(&t) => Ok(t),
            _ => Err("timeout 必须是 -1 或 1..=2147483 的整数秒".to_string()),
        },
    }
}

// ── 生命周期落档(事件信封 + 冷恢复 fold)──────────────────────────
// ask 族事件:ask/requested → 终局 ask/answered | ask/cancelled;
// ask/timed-out 非终局(超时后仍可答)。事件是桌面行状态机与冷恢复
// re-ask 的事实源;questions 形状与 question/requested 帧同构。

/// 一次未收口的问答(冷恢复 re-ask 的输入)
#[derive(Debug, Clone)]
pub struct PendingAskEntry {
    /// ask/requested 事件 id(发起时的 rpc_id)
    pub id: String,
    /// 关联 tool/call 日志 seq(行键;缺席 = 无行归属)
    pub call_id: Option<String>,
    /// 原问题集(re-ask 原样重发)
    pub questions: Vec<QuestionItem>,
    /// 等待秒数(-1 阻塞;re-ask 沿用)
    pub timeout_sec: i64,
}

/// questions → 事件/帧共用的 JSON 数组形状(与 proto Question 序列化同构:
/// header/description 为 null 而非缺席)。迟到应答的 steer 文本与
/// ask/requested 事件共用此形状
pub fn questions_to_event_json(questions: &[QuestionItem]) -> Value {
    json!(
        questions
            .iter()
            .map(|q| {
                json!({
                    "id": q.id,
                    "question": q.question,
                    "header": q.header,
                    "options": q.options.iter().map(|o| json!({
                        "label": o.label,
                        "description": o.description,
                    })).collect::<Vec<_>>(),
                    "multiSelect": q.multi_select,
                })
            })
            .collect::<Vec<_>>()
    )
}

/// 事件 questions → 问题集(缺 id/question 的畸形条目跳过,不猜不补)
fn questions_from_event_json(v: &Value) -> Vec<QuestionItem> {
    v.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|q| {
                    Some(QuestionItem {
                        id: q["id"].as_str()?.to_string(),
                        question: q["question"].as_str()?.to_string(),
                        header: q["header"].as_str().map(String::from),
                        options: q["options"]
                            .as_array()
                            .map(|opts| {
                                opts.iter()
                                    .filter_map(|o| {
                                        Some(QuestionOption {
                                            label: o["label"].as_str()?.to_string(),
                                            description: o["description"]
                                                .as_str()
                                                .map(String::from),
                                        })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        multi_select: q["multiSelect"].as_bool().unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `ask/requested` 事件信封(全集:id/callId/questions/timeoutSec)。
pub fn ask_requested_envelope(
    id: &str,
    call_id: Option<&str>,
    questions: &[QuestionItem],
    timeout_sec: i64,
    ts: i64,
) -> liuma_session::EventEnvelope {
    let mut data = json!({
        "id": id,
        "questions": questions_to_event_json(questions),
        "timeoutSec": timeout_sec,
    });
    if let Some(cid) = call_id {
        data["callId"] = json!(cid);
    }
    liuma_session::EventEnvelope::new("ask/requested", ts, data)
}

/// `ask/answered` 事件信封(answers 来自应答载荷;late = 超时后的迟到应答)。
pub fn ask_answered_envelope(
    id: &str,
    call_id: Option<&str>,
    answers: Option<&Value>,
    late: bool,
    ts: i64,
) -> liuma_session::EventEnvelope {
    let mut data = json!({ "id": id, "late": late });
    if let Some(cid) = call_id {
        data["callId"] = json!(cid);
    }
    if let Some(answers) = answers {
        data["answers"] = answers.clone();
    }
    liuma_session::EventEnvelope::new("ask/answered", ts, data)
}

/// `ask/timed-out` / `ask/cancelled` 事件信封(非终局/终局收口)。
pub fn ask_terminal_envelope(
    ty: &str,
    id: &str,
    call_id: Option<&str>,
    ts: i64,
) -> liuma_session::EventEnvelope {
    let mut data = json!({ "id": id });
    if let Some(cid) = call_id {
        data["callId"] = json!(cid);
    }
    liuma_session::EventEnvelope::new(ty, ts, data)
}

/// 待答折叠:ask/requested 之后无同键终局(answered/cancelled)的条目,
/// 按 seq 序。键 = callId,缺席回落事件 id——冷恢复 re-ask 换新 rpc_id,
/// 同 callId 的终局同样收口(防重启后重复 re-ask)。timed-out 非终局,
/// 条目保留(仍可答)。
pub fn pending_asks(log: &liuma_session::EventLog) -> Vec<PendingAskEntry> {
    let key_of = |data: &Value| -> Option<String> {
        data["callId"]
            .as_str()
            .map(String::from)
            .or_else(|| data["id"].as_str().map(String::from))
    };
    let mut pending: Vec<PendingAskEntry> = Vec::new();
    for ev in log.iter() {
        match ev.r#type.as_str() {
            "ask/requested" => {
                // 畸形条目(缺 id)跳过:fold 不猜不补
                let Some(entry) = pending_ask_entry(&ev.data) else {
                    continue;
                };
                let key = entry.call_id.clone().unwrap_or_else(|| entry.id.clone());
                pending.retain(|e| e.call_id.clone().unwrap_or_else(|| e.id.clone()) != key);
                pending.push(entry);
            }
            "ask/answered" | "ask/cancelled" => {
                if let Some(key) = key_of(&ev.data) {
                    pending.retain(|e| e.call_id.clone().unwrap_or_else(|| e.id.clone()) != key);
                }
            }
            _ => {}
        }
    }
    pending
}

/// `ask/requested` 事件数据 → 条目(缺 id 视为畸形,跳过)
fn pending_ask_entry(data: &Value) -> Option<PendingAskEntry> {
    Some(PendingAskEntry {
        id: data["id"].as_str()?.to_string(),
        call_id: data["callId"].as_str().map(String::from),
        questions: questions_from_event_json(&data["questions"]),
        timeout_sec: data["timeoutSec"].as_i64().unwrap_or(-1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 模型面 skip 契约锁:空 selected = 用户跳过,描述必须载明
    /// 「不得代选、不得推进,应重问或停止等待」(真机事故:模型把空
    /// selected 读成「按推荐项继续」并擅自推进)
    #[test]
    fn description_carries_skip_contract() {
        let tool = AskQuestionTool::new(Arc::new(RecordingPort), "ws");
        let spec = tool.specs()[0]["function"]["description"]
            .as_str()
            .expect("描述应在场")
            .to_string();
        assert!(
            spec.contains("empty `selected` array means the user skipped"),
            "描述应载明空选择的 skip 语义"
        );
        assert!(
            spec.contains("never choose an option on the user's behalf"),
            "描述应禁止代用户选择"
        );
        assert!(
            spec.contains("wait for the user's explicit direction"),
            "描述应要求停止等待用户明确指示"
        );
    }

    /// 模型面 pending 契约锁:超时 pending ≠ 跳过,描述必须载明
    /// 「仍可答、继续独立工作、不得视为许可、迟到回复形状」(对齐 dsh
    /// timed 描述;真机事故的同族风险:模型把「没答」读成默许)
    #[test]
    fn description_carries_pending_contract() {
        let tool = AskQuestionTool::new(Arc::new(RecordingPort), "ws");
        let spec = tool.specs()[0]["function"]["description"]
            .as_str()
            .expect("描述应在场")
            .to_string();
        assert!(
            spec.contains("A `pending` result means no answer batch arrived before the timeout"),
            "描述应载明 pending 语义"
        );
        assert!(
            spec.contains("do not treat it as permission"),
            "描述应封死「没答=默许」"
        );
        assert!(
            spec.contains("answer_to_pending_question"),
            "描述应写明迟到回复的标识"
        );
    }

    /// timeout 入参校验:-1 / 1..=2147483 / 缺省 120 / 越界与非整数拒。
    #[test]
    fn parse_timeout_validates() {
        let args = |v: Value| v;
        assert_eq!(
            parse_timeout(&args(json!({ "questions": [] }))).unwrap(),
            DEFAULT_ASK_TIMEOUT_SEC,
            "缺席取缺省"
        );
        assert_eq!(parse_timeout(&args(json!({ "timeout": -1 }))).unwrap(), -1);
        assert_eq!(parse_timeout(&args(json!({ "timeout": 1 }))).unwrap(), 1);
        assert_eq!(
            parse_timeout(&args(json!({ "timeout": 2_147_483 }))).unwrap(),
            2_147_483
        );
        for bad in [0, -2, 2_147_484] {
            assert!(
                parse_timeout(&args(json!({ "timeout": bad }))).is_err(),
                "{bad} 应被拒"
            );
        }
        assert!(parse_timeout(&args(json!({ "timeout": "120" }))).is_err());
        // JSON 字符串方言(OpenAI 兼容 wire)
        let wire = json!(serde_json::to_string(&json!({ "timeout": 30 })).unwrap());
        assert_eq!(parse_timeout(&wire).unwrap(), 30);
    }

    #[derive(Default)]
    struct RecordingPort;

    impl AskQuestionPort for RecordingPort {
        fn ask(
            &self,
            _session_id: &str,
            _call_id: &str,
            _questions: &[QuestionItem],
            _timeout_sec: i64,
        ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
            Box::pin(std::future::ready(Err("未应答".into())))
        }
    }

    #[test]
    fn parse_questions_valid() {
        let args = json!({
            "questions": [
                { "id": "q1", "question": "继续?", "multi_select": false,
                  "options": [ { "label": "是", "description": "进行" }, { "label": "否" } ] },
                { "id": "q2", "question": "多选?", "multi_select": true,
                  "options": [ { "label": "A" }, { "label": "B" } ] },
            ]
        });
        let q = parse_questions(&args).unwrap();
        assert_eq!(q.len(), 2);
        assert_eq!(q[0].id, "q1");
        assert_eq!(q[0].options.len(), 2);
        assert_eq!(q[0].options[0].description.as_deref(), Some("进行"));
        assert!(!q[0].multi_select);
        assert!(q[1].multi_select);
    }

    #[test]
    fn parse_questions_autogenerates_missing_id() {
        // id 宽容兜底:缺席自动生成 q1(应答回带同值);显式空串同途
        let args = json!({ "questions": [
            { "question": "无 id" },
            { "id": "  ", "question": "空白 id" },
            { "id": "keep", "question": "显式 id 保留" },
        ]});
        let q = parse_questions(&args).unwrap();
        assert_eq!(q[0].id, "q1");
        assert_eq!(q[1].id, "q2");
        assert_eq!(q[2].id, "keep");
    }

    #[test]
    fn parse_questions_rejects_empty() {
        assert!(parse_questions(&json!({ "questions": [] })).is_err());
        assert!(parse_questions(&json!({})).is_err());
    }

    #[test]
    fn parse_questions_accepts_json_string_args() {
        // deepseek/OpenAI 对话方言把 tool 参数作为字符串下发:arguments 是 JSON 文本
        let payload = serde_json::to_string(&json!({
            "questions": [ { "id": "q1", "question": "继续?", "options": [ { "label": "是" } ] } ]
        }))
        .unwrap();
        let args = json!(payload); // Value::String(JSON 文本)
        let q = parse_questions(&args).unwrap();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].id, "q1");
    }

    // ── 生命周期落档 fold ─────────────────────────────────────────

    use liuma_session::{EventEnvelope, EventLog};

    fn log_of(events: &[EventEnvelope]) -> EventLog {
        let mut log = EventLog::new();
        for (i, ev) in events.iter().enumerate() {
            let mut ev = ev.clone();
            ev.seq = i as u64 + 1;
            log.append(ev).unwrap();
        }
        log
    }

    fn requested(id: &str, call_id: Option<&str>, ts: i64) -> EventEnvelope {
        ask_requested_envelope(
            id,
            call_id,
            &[QuestionItem {
                id: "q1".into(),
                question: "继续?".into(),
                header: None,
                options: vec![QuestionOption {
                    label: "是".into(),
                    description: None,
                }],
                multi_select: false,
            }],
            -1,
            ts,
        )
    }

    /// requested 无终局 → 在;answered/cancelled → 收口;timed-out → 保留。
    #[test]
    fn pending_asks_folds_lifecycle() {
        assert!(pending_asks(&EventLog::new()).is_empty(), "无事件无待答");
        let unresolved = log_of(&[requested("r1", None, 1)]);
        let entries = pending_asks(&unresolved);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "r1");
        assert_eq!(entries[0].questions.len(), 1);
        assert_eq!(entries[0].timeout_sec, -1);
        for terminal in ["ask/answered", "ask/cancelled"] {
            let log = log_of(&[
                requested("r1", None, 1),
                ask_terminal_envelope(terminal, "r1", None, 2),
            ]);
            assert!(pending_asks(&log).is_empty(), "{terminal} 应收口待答");
        }
        // timed-out 非终局:仍可答,条目保留
        let timed_out = log_of(&[
            requested("r1", None, 1),
            ask_terminal_envelope("ask/timed-out", "r1", None, 2),
        ]);
        assert_eq!(pending_asks(&timed_out).len(), 1);
    }

    /// 键 = callId(缺席回落 id):re-ask 换新 rpc_id,同 callId 终局同样
    /// 收口——否则每次重启都会对同一 tool 行重复 re-ask。
    #[test]
    fn pending_asks_folds_by_call_id() {
        let log = log_of(&[
            requested("r1", Some("call:7"), 1),
            // 冷恢复 re-ask:新 rpc、同 callId
            requested("r2", Some("call:7"), 2),
            // 迟到应答落在新 rpc 上,但 callId 相同 → 整链收口
            ask_answered_envelope("r2", Some("call:7"), None, true, 3),
        ]);
        assert!(
            pending_asks(&log).is_empty(),
            "同 callId 的终局应收口全部同键条目"
        );
        // 无 callId 时按 id 折叠:不同 id 互不干扰
        let log = log_of(&[
            requested("r1", None, 1),
            requested("r2", None, 2),
            ask_terminal_envelope("ask/cancelled", "r1", None, 3),
        ]);
        let entries = pending_asks(&log);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "r2");
    }

    /// questions 事件形状往返:envelope 落档 → fold 反解,字段不丢。
    #[test]
    fn questions_roundtrip_through_event_shape() {
        let questions = vec![
            QuestionItem {
                id: "q1".into(),
                question: "选模式?".into(),
                header: Some("Choose Mode".into()),
                options: vec![QuestionOption {
                    label: "甲".into(),
                    description: Some("说明".into()),
                }],
                multi_select: true,
            },
            QuestionItem {
                id: "q2".into(),
                question: "继续?".into(),
                header: None,
                options: vec![],
                multi_select: false,
            },
        ];
        let ev = ask_requested_envelope("r1", Some("call:3"), &questions, 120, 1);
        let log = log_of(&[ev]);
        let entries = pending_asks(&log);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].call_id.as_deref(), Some("call:3"));
        assert_eq!(entries[0].timeout_sec, 120);
        let got = &entries[0].questions;
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].header.as_deref(), Some("Choose Mode"));
        assert_eq!(got[0].options[0].description.as_deref(), Some("说明"));
        assert!(got[0].multi_select);
        assert!(got[1].options.is_empty());
    }
}
