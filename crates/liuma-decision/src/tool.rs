//! decide 工具:让模型主动咨询决策模型(接入形态 b「决策即工具」)。
//!
//! 隐私最小面:state 只含模型在参数里显式给出的 `context` 文本,
//! 不自动携带会话内容;工具描述明示「内容将发送至配置的决策端点」。
//! 审计:每次调用缓冲 `decision/asked` + `decision/answered` 事件对,
//! 经 `ToolPort::take_state_events` 由引擎在唯一写入口取走落档
//! (工具不直写日志的单边界规则)。
//!
//! 失败语义:端口任何 Err → `success: false` + 原因文本回灌模型,
//! 不中断 loop(与工具族一致)。

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::client::DecisionError;
use crate::thresholds::{MAX_QUESTIONS_PER_REQUEST, STATE_MAX_CHARS};
use crate::types::{Answer, DecisionRequest, Question};

/// 工具名(模型面;与注册表键 `decide` 解耦,同 bash/pwsh 先例)
pub const DECIDE_TOOL_NAME: &str = "decide";

/// decide 工具(端口缺席不装配;见 mount 层)
pub struct DecideTool {
    port: Arc<dyn crate::DecisionPort>,
    /// 请求 model 字段(装配配置;审计事件记录)
    model: String,
    /// 缓冲的状态事件对(引擎 tool/result 后取走)
    pending: Mutex<Vec<(String, Value)>>,
}

impl DecideTool {
    /// 构造(port 缺 = mount 层跳过;model 用于请求与审计)
    pub fn new(port: Arc<dyn crate::DecisionPort>, model: String) -> Self {
        Self {
            port,
            model,
            pending: Mutex::new(Vec::new()),
        }
    }

    /// 缓冲一对审计事件
    fn record(&self, asked: Value, answered: Value) {
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(("decision/asked".into(), asked));
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(("decision/answered".into(), answered));
    }

    /// 把模型参数翻译为协议问题;非法条目返回错误文本
    fn parse_questions(value: &Value) -> Result<Vec<(String, Question)>, String> {
        let items = value
            .as_array()
            .ok_or_else(|| "questions 必须是数组".to_string())?;
        if items.is_empty() {
            return Err("questions 不能为空".to_string());
        }
        let mut out = Vec::new();
        for item in items {
            let id = item["id"]
                .as_str()
                .ok_or_else(|| "每个问题需要字符串 id".to_string())?
                .to_string();
            let instructions = item["instructions"]
                .as_str()
                .ok_or_else(|| format!("问题 {id} 缺 instructions"))?
                .to_string();
            let instructions = Value::String(instructions);
            let kind = item["kind"].as_str().unwrap_or_default();
            let question = match kind {
                "noul" => Question::Noul {
                    instructions,
                    criteria: None,
                },
                "choice" => {
                    let options = item["options"]
                        .as_array()
                        .ok_or_else(|| format!("问题 {id} 的 choice 需要 options 数组"))?;
                    if options.len() < 2 || options.len() > 255 {
                        return Err(format!("问题 {id} 的选项数需在 2..=255"));
                    }
                    let mut criteria = std::collections::BTreeMap::new();
                    for option in options {
                        let name = option
                            .as_str()
                            .ok_or_else(|| format!("问题 {id} 的选项必须为字符串"))?;
                        criteria.insert(name.to_string(), String::new());
                    }
                    Question::Choice {
                        instructions,
                        criteria,
                    }
                }
                "score" => {
                    let levels = item["levels"]
                        .as_array()
                        .ok_or_else(|| format!("问题 {id} 的 score 需要 levels 数组"))?;
                    if levels.len() < 2 {
                        return Err(format!("问题 {id} 的等级数至少 2"));
                    }
                    let criteria = levels
                        .iter()
                        .map(|l| {
                            l.as_str()
                                .map(String::from)
                                .ok_or_else(|| format!("问题 {id} 的等级必须为字符串"))
                        })
                        .collect::<Result<Vec<String>, String>>()?;
                    Question::Score {
                        instructions,
                        criteria,
                    }
                }
                other => return Err(format!("问题 {id} 的 kind 未知:{other}")),
            };
            out.push((id, question));
        }
        Ok(out)
    }

    /// 答案 → 紧凑文本(模型消费;含分布与 confidence)
    fn format_answers(answers: &std::collections::BTreeMap<String, Answer>) -> String {
        let mut lines = Vec::new();
        for (id, answer) in answers {
            match answer {
                Answer::Noul { noul } => {
                    lines.push(format!("{id}: noul={noul:.3}"));
                }
                Answer::Choice {
                    choice,
                    probabilities,
                    confidence,
                } => {
                    let probs = probabilities
                        .iter()
                        .map(|(k, v)| format!("{k}={v:.3}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    lines.push(format!(
                        "{id}: choice={choice} confidence={confidence:.3} ({probs})"
                    ));
                }
                Answer::Score {
                    score,
                    legend,
                    probabilities,
                    confidence,
                } => {
                    let level = score.round().clamp(0.0, legend.len() as f64 - 1.0);
                    let label = legend
                        .get(&(level as u64).to_string())
                        .map(String::as_str)
                        .unwrap_or("?");
                    lines.push(format!(
                        "{id}: score={score:.2} ({label}) confidence={confidence:.3}"
                    ));
                    let _ = probabilities;
                }
            }
        }
        lines.join("\n")
    }
}

impl liuma_agent_loop::ToolPort for DecideTool {
    fn specs(&self) -> Vec<Value> {
        json!([{
            "type": "function",
            "function": {
                "name": DECIDE_TOOL_NAME,
                "description": "Ask the decision model a small set of bounded questions (yes/no probability, choice from a fixed option list, or rating on a scale) about the exact text you pass in `context`. Only `context` is sent to the configured decision endpoint — do not reference session contents indirectly. Prefer one call with several independent questions over several calls. Use it for quick judgments your code-like reasoning cannot settle: is this error transient, which of these options fits a rule, how severe is this warning.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "questions": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": { "type": "string", "description": "Short identifier; answers come back keyed by it." },
                                    "kind": { "type": "string", "enum": ["noul", "choice", "score"] },
                                    "instructions": { "type": "string", "description": "One specific, well-scoped question about `context`." },
                                    "options": { "type": "array", "items": { "type": "string" }, "description": "choice only: option names (2-255); include `other` when the list may not cover every input." },
                                    "levels": { "type": "array", "items": { "type": "string" }, "description": "score only: ordered level descriptions, lowest first (2+ levels)." }
                                },
                                "required": ["id", "kind", "instructions"]
                            }
                        },
                        "context": { "type": "string", "description": "The exact text to evaluate. This is the only content sent to the decision endpoint." }
                    },
                    "required": ["questions", "context"]
                }
            }
        }])
        .as_array()
        .cloned()
        .unwrap_or_default()
    }

    async fn execute(
        &mut self,
        call: &liuma_agent_loop::ToolCallRequest,
    ) -> liuma_agent_loop::ToolOutput {
        // wire 上 arguments 是 JSON 编码字符串,本地夹具是对象——两种形态
        // 都经 [`ToolCallRequest::parsed_arguments`] 归一(别的工具各自
        // 就地容忍,此处走统一入口)
        let args = match call.parsed_arguments() {
            Ok(v) => v,
            Err(reason) => {
                return liuma_agent_loop::ToolOutput {
                    output: format!("decide rejected: {reason}"),
                    success: false,
                    ..Default::default()
                };
            }
        };
        let context = args["context"].as_str().unwrap_or_default();
        let questions = match Self::parse_questions(&args["questions"]) {
            Ok(q) => q,
            Err(reason) => {
                return liuma_agent_loop::ToolOutput {
                    output: format!("decide rejected: {reason}"),
                    success: false,
                    ..Default::default()
                };
            }
        };
        // 上限守卫:超量截断并在输出注明(问题并行评估,小批优先)
        let truncated = questions.len() > MAX_QUESTIONS_PER_REQUEST;
        let questions = if truncated {
            questions
                .into_iter()
                .take(MAX_QUESTIONS_PER_REQUEST)
                .collect()
        } else {
            questions
        };
        let id = uuid::Uuid::now_v7().to_string();
        let mut request = DecisionRequest {
            state: json!(context),
            questions,
            model: self.model.clone(),
        };
        request.state = crate::client::fit_state(request.state, STATE_MAX_CHARS);
        let asked = json!({
            "id": id,
            "scenario": "tool",
            "model": self.model,
            "questions": request.questions.iter().map(|(qid, q)| {
                let kind = match q {
                    Question::Noul { .. } => "noul",
                    Question::Choice { .. } => "choice",
                    Question::Score { .. } => "score",
                };
                json!({ "id": qid, "kind": kind })
            }).collect::<Vec<_>>(),
            "stateDigest": request.state_digest(),
        });
        let started = std::time::Instant::now();
        let result = self.port.ask(request).await;
        let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        match result {
            Ok(answers) => {
                self.record(
                    asked,
                    json!({
                        "id": id,
                        "ok": true,
                        "answers": serde_json::to_value(&answers.answers).unwrap_or(Value::Null),
                        "durationMs": duration_ms,
                        "usage": {
                            "inputTokens": answers.usage.input_tokens,
                            "outputTokens": answers.usage.output_tokens,
                        },
                    }),
                );
                let mut output = Self::format_answers(&answers.answers);
                if truncated {
                    output =
                        format!("[questions truncated to {MAX_QUESTIONS_PER_REQUEST}]\n{output}");
                }
                liuma_agent_loop::ToolOutput {
                    output,
                    success: true,
                    ..Default::default()
                }
            }
            Err(err) => {
                self.record(
                    asked,
                    json!({
                        "id": id,
                        "ok": false,
                        "error": err.to_string(),
                        "durationMs": duration_ms,
                    }),
                );
                liuma_agent_loop::ToolOutput {
                    output: format!("decide unavailable: {}", describe(&err)),
                    success: false,
                    ..Default::default()
                }
            }
        }
    }

    fn take_state_events(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut self.pending.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

/// 失败原因的模型面措辞(Disabled 对模型无意义,统一为未配置)
fn describe(err: &DecisionError) -> String {
    match err {
        DecisionError::Disabled(_) => {
            "decision endpoint is not configured (missing API key)".into()
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use liuma_agent_loop::{ToolCallRequest, ToolPort};

    use crate::fake::FakeDecisionPort;
    use crate::types::DecisionAnswers;

    fn call(arguments: Value) -> ToolCallRequest {
        ToolCallRequest {
            name: DECIDE_TOOL_NAME.into(),
            arguments,
            id: String::new(),
        }
    }

    #[tokio::test]
    async fn happy_path_emits_paired_events_and_text() {
        let port = Arc::new(FakeDecisionPort::scripted(vec![Ok(
            FakeDecisionPort::noul_answers("is_transient", 0.93),
        )]));
        let mut tool = DecideTool::new(port.clone(), "jev-latest".into());
        let out = tool
            .execute(&call(json!({
                "context": "connection reset by peer",
                "questions": [
                    { "id": "is_transient", "kind": "noul", "instructions": "Is this error transient?" }
                ]
            })))
            .await;
        assert!(out.success, "{}", out.output);
        assert!(out.output.contains("is_transient"), "{}", out.output);
        assert!(out.output.contains("0.930"), "{}", out.output);
        // 事件对:asked 在前 answered 在后,scenario=tool,id 配对
        let events = tool.take_state_events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, "decision/asked");
        assert_eq!(events[1].0, "decision/answered");
        let asked = &events[0].1;
        let answered = &events[1].1;
        assert_eq!(asked["scenario"], "tool");
        assert_eq!(asked["model"], "jev-latest");
        assert_eq!(asked["id"], answered["id"]);
        assert_eq!(asked["questions"][0]["kind"], "noul");
        assert_eq!(answered["ok"], true);
        let noul = answered["answers"]["is_transient"]["noul"]
            .as_f64()
            .unwrap();
        assert!((noul - 0.93).abs() < 1e-9, "noul={noul}");
        // 收到的请求:state 即 context 原文(隐私最小面)
        let received = port.take_received();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].state, json!("connection reset by peer"));
    }

    /// 回归锁:真实 wire 上 `arguments` 是 **JSON 编码字符串**(流式增量
    /// 累积,引擎原样透传),不是对象。
    ///
    /// 实测:一个形状完全正确的 questions 数组被报成「questions 必须是
    /// 数组」——因为 `args["questions"]` 索引一段字符串得到 Null。
    /// 夹具用对象构造,所以这条线只有**按 wire 形态**写的测试才守得住。
    #[tokio::test]
    async fn wire_string_arguments_parse_like_objects() {
        let port = Arc::new(FakeDecisionPort::scripted(vec![Ok(
            FakeDecisionPort::noul_answers("is_transient", 0.93),
        )]));
        let mut tool = DecideTool::new(port.clone(), "jev-latest".into());
        let out = tool
            .execute(&call(Value::String(
                json!({
                    "context": "connection reset by peer",
                    "questions": [
                        { "id": "is_transient", "kind": "noul", "instructions": "Is this error transient?" }
                    ]
                })
                .to_string(),
            )))
            .await;
        assert!(out.success, "字符串形态参数必须等价于对象:{}", out.output);
        assert_eq!(
            port.take_received()[0].state,
            json!("connection reset by peer")
        );

        // 参数不是合法 JSON:报「不是 JSON」,不误报「缺字段」
        let mut tool = DecideTool::new(Arc::new(FakeDecisionPort::new()), "m".into());
        let out = tool
            .execute(&call(Value::String("{ not json".into())))
            .await;
        assert!(!out.success);
        assert!(
            out.output.contains("not valid JSON"),
            "应指出参数不是 JSON,实为:{}",
            out.output
        );
        assert!(tool.take_state_events().is_empty(), "未出网即无事件");
    }

    #[tokio::test]
    async fn port_failure_fails_softly_with_closed_pair() {
        let port = Arc::new(FakeDecisionPort::failing(DecisionError::Timeout));
        let mut tool = DecideTool::new(port, "m".into());
        let out = tool
            .execute(&call(json!({
                "context": "x",
                "questions": [{ "id": "q", "kind": "noul", "instructions": "?" }]
            })))
            .await;
        assert!(!out.success);
        assert!(out.output.contains("decide unavailable"), "{}", out.output);
        let events = tool.take_state_events();
        assert_eq!(events.len(), 2, "失败也收口");
        assert_eq!(events[1].1["ok"], false);
        assert!(events[1].1["error"].is_string());
    }

    #[tokio::test]
    async fn bad_arguments_rejected_without_events() {
        let port = Arc::new(FakeDecisionPort::new());
        let mut tool = DecideTool::new(port, "m".into());
        let out = tool
            .execute(&call(json!({
                "context": "x",
                "questions": [{ "id": "q", "kind": "nonsense", "instructions": "?" }]
            })))
            .await;
        assert!(!out.success);
        assert!(out.output.contains("kind 未知"), "{}", out.output);
        assert!(tool.take_state_events().is_empty(), "未出网即无事件");
    }

    #[tokio::test]
    async fn choice_and_score_kinds_build_criteria() {
        let port = Arc::new(FakeDecisionPort::scripted(vec![Ok(DecisionAnswers {
            model: "fake".into(),
            answers: {
                let mut m = std::collections::BTreeMap::new();
                m.insert(
                    "dept".into(),
                    Answer::Choice {
                        choice: "billing".into(),
                        probabilities: {
                            let mut p = std::collections::BTreeMap::new();
                            p.insert("billing".into(), 0.9);
                            p.insert("other".into(), 0.1);
                            p
                        },
                        confidence: 0.8,
                    },
                );
                m.insert(
                    "sev".into(),
                    Answer::Score {
                        score: 1.2,
                        legend: {
                            let mut l = std::collections::BTreeMap::new();
                            l.insert("0".into(), "low".into());
                            l.insert("1".into(), "mid".into());
                            l.insert("2".into(), "high".into());
                            l
                        },
                        probabilities: std::collections::BTreeMap::new(),
                        confidence: 0.7,
                    },
                );
                m
            },
            usage: Default::default(),
            request_id: None,
        })]));
        let mut tool = DecideTool::new(port.clone(), "m".into());
        let out = tool
            .execute(&call(json!({
                "context": "payout failing",
                "questions": [
                    { "id": "dept", "kind": "choice", "instructions": "Which team?", "options": ["billing", "other"] },
                    { "id": "sev", "kind": "score", "instructions": "Severity?", "levels": ["low", "mid", "high"] }
                ]
            })))
            .await;
        assert!(out.success, "{}", out.output);
        assert!(out.output.contains("choice=billing"), "{}", out.output);
        assert!(out.output.contains("score=1.20"), "{}", out.output);
        let received = port.take_received();
        let Question::Choice { criteria, .. } = &received[0].questions[0].1 else {
            panic!("choice");
        };
        assert!(criteria.contains_key("billing") && criteria.contains_key("other"));
        let Question::Score { criteria, .. } = &received[0].questions[1].1 else {
            panic!("score");
        };
        assert_eq!(criteria.len(), 3);
    }
}
