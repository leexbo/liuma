//! Stop 哨兵:turn 收尾前核对「声称完成是否有证据」(advisory)。
//!
//! jev-belay 模式:取最后 assistant 陈述 + 其后的工具结果为证据 state,
//! 问一次"该陈述是否缺乏本次会话观察到的证据支撑";缺证据概率高则
//! `StopVerdict::Continue`(reason 要求补证据)。引擎对 Continue 不做
//! loop guard(见 liuma-agent-loop engine.rs 注释),哨兵**自限**同 turn
//! 次数。fail-open:端口任何 Err/超时/disabled → Pass(与无哨兵一致)。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::types::{Answer, DecisionRequest, NoulCriteria, Question};

/// 单条证据的字符上限(证据 state 体积守卫)
const EVIDENCE_ITEM_MAX_CHARS: usize = 2_000;

/// 最多随附的证据条数(最后 assistant 之后的 tool/result)
const EVIDENCE_MAX_ITEMS: usize = 5;

/// Stop 哨兵(HookPort 四点;仅 on_stop 介入,其余直通)
pub struct StopSentinel {
    port: Arc<dyn crate::DecisionPort>,
    model: String,
    log: Arc<Mutex<liuma_session::EventLog>>,
    /// 决策 receipt 落档(宿主闭包;持久化归日志 durability sink 单写权威)
    sink: crate::scenarios::ReceiptSink,
    /// 同 turn Continue 计数(引擎不 loop guard,哨兵自限)
    continues: Mutex<HashMap<u64, u32>>,
}

impl StopSentinel {
    /// 构造(log 与 sink 由宿主装配面注入)
    pub fn new(
        port: Arc<dyn crate::DecisionPort>,
        model: String,
        log: Arc<Mutex<liuma_session::EventLog>>,
        sink: crate::scenarios::ReceiptSink,
    ) -> Self {
        Self {
            port,
            model,
            log,
            sink,
            continues: Mutex::new(HashMap::new()),
        }
    }

    /// 证据 state:最后 assistant/message 正文 + 其后的 tool/result 输出
    /// (截断守卫;无 assistant 陈述 = None,哨兵不介入)
    fn evidence(&self) -> Option<Value> {
        let log = self.log.lock().unwrap_or_else(|p| p.into_inner());
        let mut last_statement: Option<String> = None;
        let mut results: Vec<String> = Vec::new();
        for ev in log.iter() {
            match ev.r#type.as_str() {
                "assistant/message" => {
                    last_statement = ev.data["content"].as_str().map(String::from);
                    results.clear();
                }
                // 只收集最后陈述之后的工具结果(前面的证据已过期)
                "tool/result" if last_statement.is_some() => {
                    let output = ev.data["output"].as_str().unwrap_or_default();
                    let cut: String =
                        crate::client::fit_state(json!(output), EVIDENCE_ITEM_MAX_CHARS)
                            .as_str()
                            .unwrap_or_default()
                            .to_string();
                    results.push(cut);
                    if results.len() >= EVIDENCE_MAX_ITEMS {
                        results.remove(0);
                    }
                }
                _ => {}
            }
        }
        let statement = last_statement?;
        Some(json!({
            "final_statement": statement,
            "tool_results": results,
        }))
    }

    /// receipt 对落档(id 配对;失败也收口)
    fn record(&self, asked: Value, answered: Value) {
        (self.sink)("decision/asked", asked);
        (self.sink)("decision/answered", answered);
    }

    /// 同 turn 已 Continue 次数
    fn continues_of(&self, turn: u64) -> u32 {
        *self
            .continues
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&turn)
            .unwrap_or(&0)
    }

    fn bump_continue(&self, turn: u64) {
        *self
            .continues
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(turn)
            .or_default() += 1;
    }
}

impl liuma_agent_loop::hooks::HookPort for StopSentinel {
    async fn on_prompt_submit(
        &self,
        _prompt: &str,
        _turn: u64,
    ) -> liuma_agent_loop::hooks::PreStepVerdict {
        liuma_agent_loop::hooks::PreStepVerdict::Proceed
    }

    async fn pre_tool(
        &self,
        _call: &liuma_agent_loop::ToolCallRequest,
        _turn: u64,
    ) -> liuma_agent_loop::hooks::PreToolVerdict {
        liuma_agent_loop::hooks::PreToolVerdict::Proceed
    }

    async fn post_tool(
        &self,
        _call: &liuma_agent_loop::ToolCallRequest,
        _output: &liuma_agent_loop::ToolOutput,
        _turn: u64,
    ) -> liuma_agent_loop::hooks::PostToolVerdict {
        liuma_agent_loop::hooks::PostToolVerdict::Pass
    }

    async fn on_stop(&self, turn: u64) -> liuma_agent_loop::hooks::StopVerdict {
        use liuma_agent_loop::hooks::StopVerdict;
        // 自限优先:超限不再问(零调用零延迟)
        if self.continues_of(turn) >= crate::thresholds::STOP_MAX_CONTINUES_PER_TURN {
            return StopVerdict::Pass;
        }
        let Some(state) = self.evidence() else {
            return StopVerdict::Pass;
        };
        let mut request = DecisionRequest {
            state,
            questions: vec![(
                "lacks_evidence".to_string(),
                Question::Noul {
                    instructions: json!(crate::thresholds::STOP_EVIDENCE_QUESTION),
                    criteria: Some(NoulCriteria {
                        yes: crate::thresholds::STOP_YES_MEANING.into(),
                        no: crate::thresholds::STOP_NO_MEANING.into(),
                    }),
                },
            )],
            model: self.model.clone(),
        };
        request.state = crate::client::fit_state(request.state, crate::thresholds::STATE_MAX_CHARS);
        let id = uuid::Uuid::now_v7().to_string();
        let asked = json!({
            "id": id,
            "scenario": "stop",
            "model": self.model,
            "questions": [{ "id": "lacks_evidence", "kind": "noul" }],
            "stateDigest": request.state_digest(),
        });
        let started = std::time::Instant::now();
        match self.port.ask(request).await {
            Ok(answers) => {
                let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
                let probability = match answers.answers.get("lacks_evidence") {
                    Some(Answer::Noul { noul }) if noul.is_finite() => *noul,
                    _ => {
                        self.record(
                            asked,
                            json!({
                                "id": id, "ok": false,
                                "error": "answer shape mismatch",
                                "durationMs": duration_ms,
                            }),
                        );
                        return StopVerdict::Pass;
                    }
                };
                self.record(
                    asked,
                    json!({
                        "id": id, "ok": true,
                        "answers": serde_json::to_value(&answers.answers).unwrap_or(Value::Null),
                        "durationMs": duration_ms,
                        "usage": {
                            "inputTokens": answers.usage.input_tokens,
                            "outputTokens": answers.usage.output_tokens,
                        },
                    }),
                );
                if probability >= crate::thresholds::STOP_CONTINUE_PROBABILITY {
                    self.bump_continue(turn);
                    StopVerdict::Continue {
                        reason: format!(
                            "evidence check (decision model {probability:.2}): the final \
                             statement lacks observable support — provide the missing \
                             evidence (tool output you actually observed) or revise the claim."
                        ),
                    }
                } else {
                    StopVerdict::Pass
                }
            }
            Err(err) => {
                // fail-open:任何错误放行(与无哨兵一致)
                let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
                self.record(
                    asked,
                    json!({
                        "id": id, "ok": false,
                        "error": err.to_string(),
                        "durationMs": duration_ms,
                    }),
                );
                StopVerdict::Pass
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::DecisionError;
    use crate::fake::FakeDecisionPort;
    use liuma_agent_loop::hooks::{HookPort, StopVerdict};
    use liuma_session::{EventEnvelope, EventLog};

    /// 收集到的 receipt 事件(ty, data)
    type ReceiptLog = Arc<Mutex<Vec<(String, Value)>>>;

    /// 记录型 sink(断言 receipt 对)
    fn recording_sink() -> (crate::scenarios::ReceiptSink, ReceiptLog) {
        let seen: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let clone = Arc::clone(&seen);
        (
            Arc::new(move |ty: &str, data: Value| {
                clone.lock().unwrap().push((ty.to_string(), data));
            }),
            seen,
        )
    }

    fn log_with_statement(statement: &str, evidence: &[&str]) -> Arc<Mutex<EventLog>> {
        let log = EventLog::new();
        let append = |log: &mut EventLog, ty: &str, data: Value| {
            log.append(EventEnvelope::new(ty, 0, data)).unwrap();
        };
        let mut log = log;
        append(&mut log, "user/message", json!({ "content": "do it" }));
        for item in evidence {
            append(
                &mut log,
                "tool/result",
                json!({ "call": 1, "output": item }),
            );
        }
        append(
            &mut log,
            "assistant/message",
            json!({ "content": statement }),
        );
        Arc::new(Mutex::new(log))
    }

    #[tokio::test]
    async fn low_evidence_probability_continues_twice_then_limits() {
        let (sink, seen) = recording_sink();
        // 三步都是「缺证据」高概率:#1/#2 Continue(自限上限 2),#3 零调用直通
        let port = Arc::new(FakeDecisionPort::scripted(vec![
            Ok(FakeDecisionPort::noul_answers("lacks_evidence", 0.95)),
            Ok(FakeDecisionPort::noul_answers("lacks_evidence", 0.95)),
            Ok(FakeDecisionPort::noul_answers("lacks_evidence", 0.95)),
        ]));
        let sentinel = StopSentinel::new(
            port,
            "m".into(),
            log_with_statement("all done and verified", &["build ok"]),
            sink,
        );
        assert!(matches!(
            HookPort::on_stop(&sentinel, 1).await,
            StopVerdict::Continue { .. }
        ));
        assert!(matches!(
            HookPort::on_stop(&sentinel, 1).await,
            StopVerdict::Continue { .. }
        ));
        // 同 turn 第三次:达 STOP_MAX_CONTINUES_PER_TURN,零调用直通
        assert_eq!(HookPort::on_stop(&sentinel, 1).await, StopVerdict::Pass);
        // 新 turn 计数独立
        assert!(matches!(
            HookPort::on_stop(&sentinel, 2).await,
            StopVerdict::Continue { .. }
        ));
        // receipt:三次介入产生三对(自限那次零调用零事件)
        let events = seen.lock().unwrap();
        assert_eq!(events.len(), 6, "三对 asked/answered:{events:?}");
        assert_eq!(events[0].0, "decision/asked");
        assert_eq!(events[1].0, "decision/answered");
        assert_eq!(events[0].1["scenario"], "stop");
        assert_eq!(events[1].1["ok"], true);
    }

    #[tokio::test]
    async fn evidence_backed_statement_passes() {
        let (sink, seen) = recording_sink();
        let port = Arc::new(FakeDecisionPort::scripted(vec![Ok(
            FakeDecisionPort::noul_answers("lacks_evidence", 0.05),
        )]));
        let sentinel = StopSentinel::new(
            port,
            "m".into(),
            log_with_statement("the build printed ok", &["build ok\nexit 0"]),
            sink,
        );
        assert_eq!(HookPort::on_stop(&sentinel, 1).await, StopVerdict::Pass);
        // receipt 照落(放行也留档)
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn port_failure_fails_open() {
        let (sink, seen) = recording_sink();
        let port = Arc::new(FakeDecisionPort::failing(DecisionError::Timeout));
        let sentinel = StopSentinel::new(port, "m".into(), log_with_statement("done", &[]), sink);
        assert_eq!(HookPort::on_stop(&sentinel, 1).await, StopVerdict::Pass);
        let events = seen.lock().unwrap();
        assert_eq!(events.len(), 2, "失败也收口");
        assert_eq!(events[1].1["ok"], false);
    }

    #[tokio::test]
    async fn no_assistant_statement_skips_entirely() {
        let (sink, seen) = recording_sink();
        let port = Arc::new(FakeDecisionPort::new());
        let log = Arc::new(Mutex::new(EventLog::new()));
        let sentinel = StopSentinel::new(port, "m".into(), log, sink);
        assert_eq!(HookPort::on_stop(&sentinel, 1).await, StopVerdict::Pass);
        assert!(seen.lock().unwrap().is_empty(), "无陈述零调用");
    }
}
