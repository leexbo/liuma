//! 工具守卫:pre_tool 的高危拦截(jev-guard/pi-warden 模式)。
//!
//! 双问一次请求:choice{proceed, block} 主裁 + risk 评分。**shadow 恒
//! Proceed**(只落 receipt 观察);`enforce` 仅当 choice=block 且
//! confidence 与选中概率都过高阈值才 `Deny`(确定性代码持终审,决策
//! 模型只有建议权)。fail-open:端口任何 Err/超时/disabled → Proceed。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::types::{Answer, DecisionRequest, Question};

/// 单条参数摘要的字符上限(state 体积守卫)
const ARGUMENTS_MAX_CHARS: usize = 2_000;

/// 工具守卫(HookPort 四点;仅 pre_tool 介入,其余直通)
pub struct ToolGuard {
    port: Arc<dyn crate::DecisionPort>,
    model: String,
    log: Arc<Mutex<liuma_session::EventLog>>,
    /// 决策 receipt 落档(宿主闭包)
    sink: crate::scenarios::ReceiptSink,
    /// shadow(只记录)/ enforce(高置信才拦)
    enforce: bool,
    /// enforce 拦截置信下限(装配层注入;None 在 liuma-app 展开为
    /// thresholds::GUARD_ENFORCE_CONFIDENCE)。只放开这一旋钮——选中
    /// 概率的防平票线(`GUARD_ENFORCE_CHOICE_PROBABILITY`)保持内置,
    /// 双阈值语义不因配置拆散
    enforce_confidence: f64,
}

impl ToolGuard {
    /// 构造(`enforce = false` 即 shadow)
    pub fn new(
        port: Arc<dyn crate::DecisionPort>,
        model: String,
        log: Arc<Mutex<liuma_session::EventLog>>,
        sink: crate::scenarios::ReceiptSink,
        enforce: bool,
        enforce_confidence: f64,
    ) -> Self {
        Self {
            port,
            model,
            log,
            sink,
            enforce,
            enforce_confidence,
        }
    }

    /// 最近任务描述:最后一条真实用户消息(source.kind=user)内容截断
    fn recent_task(&self) -> String {
        let log = self.log.lock().unwrap_or_else(|p| p.into_inner());
        log.last_matching("user/message", |ev| {
            ev.data["source"]["kind"].as_str().unwrap_or("user") == "user"
        })
        .and_then(|ev| ev.data["content"].as_str().map(String::from))
        .map(|c| c.chars().take(400).collect())
        .unwrap_or_default()
    }

    /// receipt 对落档
    fn record(&self, asked: Value, answered: Value) {
        (self.sink)("decision/asked", asked);
        (self.sink)("decision/answered", answered);
    }

    /// 双问构造(主裁 choice + 风险 score;同 state 并行评估)
    fn questions() -> Vec<(String, Question)> {
        let mut verdict_criteria = BTreeMap::new();
        verdict_criteria.insert(
            "proceed".to_string(),
            crate::thresholds::GUARD_PROCEED_MEANING.into(),
        );
        verdict_criteria.insert(
            "block".to_string(),
            crate::thresholds::GUARD_BLOCK_MEANING.into(),
        );
        vec![
            (
                "verdict".to_string(),
                Question::Choice {
                    instructions: json!(crate::thresholds::GUARD_QUESTION),
                    criteria: verdict_criteria,
                },
            ),
            (
                "risk".to_string(),
                Question::Score {
                    instructions: json!("How risky is this tool call?"),
                    criteria: vec![
                        "Harmless".into(),
                        "Low risk".into(),
                        "Risky".into(),
                        "Severe".into(),
                    ],
                },
            ),
        ]
    }
}

impl liuma_agent_loop::hooks::HookPort for ToolGuard {
    async fn on_prompt_submit(
        &self,
        _prompt: &str,
        _turn: u64,
    ) -> liuma_agent_loop::hooks::PreStepVerdict {
        liuma_agent_loop::hooks::PreStepVerdict::Proceed
    }

    async fn pre_tool(
        &self,
        call: &liuma_agent_loop::ToolCallRequest,
        _turn: u64,
    ) -> liuma_agent_loop::hooks::PreToolVerdict {
        use liuma_agent_loop::hooks::PreToolVerdict;
        // wire 上 arguments 是 JSON 编码字符串(见 ToolCallRequest):不
        // 归一的话 state 里躺的是一段二次转义的字符串,模型读得到但形状
        // 是错的。解析失败**原样透传**(不降级成空对象)——守卫是建议面,
        // 宁可把原始形态交给模型,也不要凭空造一个空参数骗它
        let arguments = crate::client::fit_state(
            call.parsed_arguments()
                .unwrap_or_else(|_| call.arguments.clone()),
            ARGUMENTS_MAX_CHARS,
        );
        let mut request = DecisionRequest {
            state: json!({
                "tool": call.name,
                "arguments": arguments,
                "recentTask": self.recent_task(),
            }),
            questions: Self::questions(),
            model: self.model.clone(),
        };
        request.state = crate::client::fit_state(request.state, crate::thresholds::STATE_MAX_CHARS);
        let id = uuid::Uuid::now_v7().to_string();
        let asked = json!({
            "id": id,
            "scenario": "guard",
            "model": self.model,
            // 被裁决的工具名:前端据此把本节点认领到那次调用上
            // (只记名字,不记参数——参数已在 state 里出境,日志不重复落)
            "tool": call.name,
            "questions": [
                { "id": "verdict", "kind": "choice" },
                { "id": "risk", "kind": "score" }
            ],
            "stateDigest": request.state_digest(),
        });
        let started = std::time::Instant::now();
        match self.port.ask(request).await {
            Ok(answers) => {
                let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
                let (verdict, confidence, chosen_prob) = match answers.answers.get("verdict") {
                    Some(Answer::Choice {
                        choice,
                        probabilities,
                        confidence,
                    }) => (
                        choice.clone(),
                        *confidence,
                        probabilities.get(choice).copied().unwrap_or(0.0),
                    ),
                    _ => {
                        self.record(
                            asked,
                            json!({
                                "id": id, "ok": false,
                                "error": "answer shape mismatch",
                                "durationMs": duration_ms,
                            }),
                        );
                        return PreToolVerdict::Proceed;
                    }
                };
                let risk = match answers.answers.get("risk") {
                    Some(Answer::Score { score, .. }) => *score,
                    _ => f64::NAN,
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
                // enforce 门:choice=block 且 confidence 与选中概率双阈值
                // 过线才拦(防平票误拦);shadow/未过线恒放行
                let should_deny = self.enforce
                    && verdict == "block"
                    && confidence >= self.enforce_confidence
                    && chosen_prob >= crate::thresholds::GUARD_ENFORCE_CHOICE_PROBABILITY;
                if should_deny {
                    PreToolVerdict::Deny {
                        reason: format!(
                            "decision-guard (enforce): tool call blocked by the decision \
                             model (confidence {confidence:.2}); revise the approach or \
                             ask the user to disable decision.guard.enforce."
                        ),
                    }
                } else {
                    let _ = risk;
                    PreToolVerdict::Proceed
                }
            }
            Err(err) => {
                // fail-open:任何错误放行
                let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
                self.record(
                    asked,
                    json!({
                        "id": id, "ok": false,
                        "error": err.to_string(),
                        "durationMs": duration_ms,
                    }),
                );
                PreToolVerdict::Proceed
            }
        }
    }

    async fn post_tool(
        &self,
        _call: &liuma_agent_loop::ToolCallRequest,
        _output: &liuma_agent_loop::ToolOutput,
        _turn: u64,
    ) -> liuma_agent_loop::hooks::PostToolVerdict {
        liuma_agent_loop::hooks::PostToolVerdict::Pass
    }

    async fn on_stop(&self, _turn: u64) -> liuma_agent_loop::hooks::StopVerdict {
        liuma_agent_loop::hooks::StopVerdict::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::DecisionError;
    use crate::fake::FakeDecisionPort;
    use liuma_agent_loop::hooks::{HookPort, PreToolVerdict};
    use liuma_session::{EventEnvelope, EventLog};

    /// choice 答案构造(verdict + risk)
    fn guard_answers(
        choice: &str,
        confidence: f64,
        prob_block: f64,
    ) -> crate::types::DecisionAnswers {
        let mut probabilities = BTreeMap::new();
        probabilities.insert("proceed".to_string(), 1.0 - prob_block);
        probabilities.insert("block".to_string(), prob_block);
        let mut answers = BTreeMap::new();
        answers.insert(
            "verdict".to_string(),
            Answer::Choice {
                choice: choice.into(),
                probabilities,
                confidence,
            },
        );
        answers.insert(
            "risk".to_string(),
            Answer::Score {
                score: 2.5,
                legend: {
                    let mut l = BTreeMap::new();
                    l.insert("0".into(), "Harmless".into());
                    l.insert("1".into(), "Low risk".into());
                    l.insert("2".into(), "Risky".into());
                    l.insert("3".into(), "Severe".into());
                    l
                },
                probabilities: BTreeMap::new(),
                confidence: 0.8,
            },
        );
        crate::types::DecisionAnswers {
            model: "m".into(),
            answers,
            usage: Default::default(),
            request_id: None,
        }
    }

    fn call() -> liuma_agent_loop::ToolCallRequest {
        liuma_agent_loop::ToolCallRequest {
            name: "bash".into(),
            arguments: json!({ "command": "rm -rf /" }),
        }
    }

    fn empty_log() -> Arc<Mutex<EventLog>> {
        let mut log = EventLog::new();
        log.append(EventEnvelope::new(
            "user/message",
            0,
            json!({ "content": "clean up", "source": { "kind": "user" } }),
        ))
        .unwrap();
        Arc::new(Mutex::new(log))
    }

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

    #[tokio::test]
    async fn shadow_never_denies_but_records() {
        let (sink, seen) = recording_sink();
        // 高置信 block:shadow 也放行
        let port = Arc::new(FakeDecisionPort::scripted(vec![Ok(guard_answers(
            "block", 0.99, 0.99,
        ))]));
        let guard = ToolGuard::new(
            port,
            "m".into(),
            empty_log(),
            sink,
            false,
            crate::thresholds::GUARD_ENFORCE_CONFIDENCE,
        );
        assert_eq!(
            HookPort::pre_tool(&guard, &call(), 1).await,
            PreToolVerdict::Proceed
        );
        let events = seen.lock().unwrap();
        assert_eq!(events.len(), 2, "shadow 也留 receipt 对");
        assert_eq!(events[0].1["scenario"], "guard");
        // 被裁决的工具名随 receipt 出境:前端据此把节点认领到那次调用上
        // (折进工具组后,行内不再有组头交代「针对哪次调用」)
        assert_eq!(events[0].1["tool"], "bash");
        assert_eq!(events[1].1["answers"]["verdict"]["choice"], "block");
    }

    #[tokio::test]
    async fn enforce_denies_only_above_both_thresholds() {
        let (sink, _seen) = recording_sink();
        // 双阈值过线:block + confidence 0.99 + 概率 0.99
        let port = Arc::new(FakeDecisionPort::scripted(vec![Ok(guard_answers(
            "block", 0.99, 0.99,
        ))]));
        let guard = ToolGuard::new(
            port,
            "m".into(),
            empty_log(),
            sink,
            true,
            crate::thresholds::GUARD_ENFORCE_CONFIDENCE,
        );
        assert!(matches!(
            HookPort::pre_tool(&guard, &call(), 1).await,
            PreToolVerdict::Deny { .. }
        ));

        // confidence 不足:放行
        let (sink, _) = recording_sink();
        let port = Arc::new(FakeDecisionPort::scripted(vec![Ok(guard_answers(
            "block", 0.7, 0.99,
        ))]));
        let guard = ToolGuard::new(
            port,
            "m".into(),
            empty_log(),
            sink,
            true,
            crate::thresholds::GUARD_ENFORCE_CONFIDENCE,
        );
        assert_eq!(
            HookPort::pre_tool(&guard, &call(), 1).await,
            PreToolVerdict::Proceed
        );

        // 选中概率不足(平票):放行
        let (sink, _) = recording_sink();
        let port = Arc::new(FakeDecisionPort::scripted(vec![Ok(guard_answers(
            "block", 0.95, 0.6,
        ))]));
        let guard = ToolGuard::new(
            port,
            "m".into(),
            empty_log(),
            sink,
            true,
            crate::thresholds::GUARD_ENFORCE_CONFIDENCE,
        );
        assert_eq!(
            HookPort::pre_tool(&guard, &call(), 1).await,
            PreToolVerdict::Proceed
        );
    }

    /// 拦截线可注入:同答案在默认 0.9 下放行、注入 0.5 后拦截——
    /// 钉住「参数真的生效」,不是摆设
    #[tokio::test]
    async fn enforce_threshold_is_injectable() {
        // block@confidence 0.7:默认线(0.9)下放行
        let (sink, _) = recording_sink();
        let port = Arc::new(FakeDecisionPort::scripted(vec![Ok(guard_answers(
            "block", 0.7, 0.99,
        ))]));
        let guard = ToolGuard::new(
            port,
            "m".into(),
            empty_log(),
            sink,
            true,
            crate::thresholds::GUARD_ENFORCE_CONFIDENCE,
        );
        assert_eq!(
            HookPort::pre_tool(&guard, &call(), 1).await,
            PreToolVerdict::Proceed
        );

        // 注入 0.5:同答案被拦
        let (sink, _) = recording_sink();
        let port = Arc::new(FakeDecisionPort::scripted(vec![Ok(guard_answers(
            "block", 0.7, 0.99,
        ))]));
        let guard = ToolGuard::new(port, "m".into(), empty_log(), sink, true, 0.5);
        assert!(matches!(
            HookPort::pre_tool(&guard, &call(), 1).await,
            PreToolVerdict::Deny { .. }
        ));
    }

    #[tokio::test]
    async fn port_failure_fails_open() {
        let (sink, seen) = recording_sink();
        let port = Arc::new(FakeDecisionPort::failing(DecisionError::Timeout));
        let guard = ToolGuard::new(
            port,
            "m".into(),
            empty_log(),
            sink,
            true,
            crate::thresholds::GUARD_ENFORCE_CONFIDENCE,
        );
        assert_eq!(
            HookPort::pre_tool(&guard, &call(), 1).await,
            PreToolVerdict::Proceed
        );
        assert_eq!(seen.lock().unwrap()[1].1["ok"], false);
    }

    /// 回归锁:守卫送出的 state 里 `arguments` 必须是**解析后的对象**。
    ///
    /// wire 上引擎原样透传的是 JSON 编码字符串;不归一的话决策模型收到
    /// 一段二次转义的字符串——读得到但形状是错的,判断质量跟着打折。
    #[tokio::test]
    async fn state_carries_parsed_arguments_not_the_wire_string() {
        let (sink, _seen) = recording_sink();
        let port = Arc::new(FakeDecisionPort::scripted(vec![Ok(guard_answers(
            "proceed", 0.9, 0.1,
        ))]));
        let guard = ToolGuard::new(
            port.clone(),
            "m".into(),
            empty_log(),
            sink,
            false,
            crate::thresholds::GUARD_ENFORCE_CONFIDENCE,
        );
        let wire = liuma_agent_loop::ToolCallRequest {
            name: "bash".into(),
            arguments: Value::String(json!({ "command": "rm -rf /tmp/x" }).to_string()),
        };
        assert_eq!(
            HookPort::pre_tool(&guard, &wire, 1).await,
            PreToolVerdict::Proceed
        );
        let received = port.take_received();
        assert_eq!(received.len(), 1);
        let args = &received[0].state["arguments"];
        assert!(
            args.is_object(),
            "state.arguments 应为对象而非 wire 字符串:{args}"
        );
        assert_eq!(args["command"], "rm -rf /tmp/x");
    }

    /// recentTask:取最后一条真实用户消息(注入上下文不算)
    #[tokio::test]
    async fn recent_task_reads_last_user_message() {
        let (sink, _seen) = recording_sink();
        let port = Arc::new(FakeDecisionPort::scripted(vec![Ok(guard_answers(
            "proceed", 0.9, 0.1,
        ))]));
        let log = empty_log();
        log.lock()
            .unwrap()
            .append(EventEnvelope::new(
                "user/message",
                0,
                json!({ "content": "[injected context]", "source": { "kind": "plugin" } }),
            ))
            .unwrap();
        let guard = ToolGuard::new(
            port,
            "m".into(),
            log,
            sink,
            false,
            crate::thresholds::GUARD_ENFORCE_CONFIDENCE,
        );
        assert_eq!(guard.recent_task(), "clean up");
    }
}
