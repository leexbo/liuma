//! 折叠价值裁定:摘要前裁定折叠区间里哪些旧工具输出**不值得**带进
//! checkpoint。
//!
//! 与不变式的兼容边界:效果发生在派生纯函数内部(`liuma-session` 策略
//! ④——`decision/pruned` 引用的 `tool/result` 输出替换为常量占位符,
//! 1:1 内容替换、条目不删),日志本体不动,请求面与闸门期望面自动
//! 同源。本模块只做纯函数(请求构造、答案解释)与一次出网:候选选择
//! 在 `liuma-compaction`(纯函数),端口在 `liuma-agent-loop`。
//!
//! **极性警告**:本场景问「价值」(越高越该留),故 **低** noul 才裁;
//! [`crate::thresholds::FOLD_DROP_PROBABILITY`] 与上下文裁判的
//! `PRUNE_NO_VALUE_PROBABILITY` 分处概率区间两端,不是同一个方向。
//!
//! 本场景**只记录**(advisory;社区共识「shadow 先行」):落 receipt 供
//! 审计与「若生效会裁多少」的观察,派生面不动。fail-open:端口任何
//! `Err` 一律照常折叠。

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use liuma_agent_loop::value_judge::{FoldAdvice, ValueCandidate, ValueJudgePolicy};
use serde_json::{Value, json};

use crate::types::{Answer, DecisionRequest, Question};

/// 最近任务描述的字符上限(state 体积守卫)
const TASK_MAX_CHARS: usize = 400;

/// 折叠价值裁定器(engine 经 `Arc<dyn ValueJudge>` 持有)
pub struct FoldJudge {
    port: Arc<dyn crate::DecisionPort>,
    model: String,
    log: Arc<Mutex<liuma_session::EventLog>>,
    /// 决策 receipt 落档(宿主闭包)
    sink: crate::scenarios::ReceiptSink,
}

impl FoldJudge {
    /// 构造
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
        }
    }

    /// 最近任务描述:最后一条真实用户消息截断(判「对折叠后继续的
    /// 工作有多重要」得有工作本身;注入上下文不算一轮,判据同其余场景)
    fn recent_task(&self) -> String {
        let log = self.log.lock().unwrap_or_else(|p| p.into_inner());
        log.iter()
            .rev()
            .find(|ev| {
                ev.r#type == "user/message"
                    && ev.data["source"]["kind"].as_str().unwrap_or("user") == "user"
            })
            .and_then(|ev| ev.data["content"].as_str())
            .map(|c| c.chars().take(TASK_MAX_CHARS).collect())
            .unwrap_or_default()
    }

    /// receipt 对落档
    fn record(&self, asked: Value, answered: Value) {
        (self.sink)("decision/asked", asked);
        (self.sink)("decision/answered", answered);
    }
}

/// 裁定 state(候选预览数组 + 最近任务;问题内反引号路径
/// `candidates[N]` 指向对应槽位——协议约定 state 是路径根)
pub fn build_state(candidates: &[ValueCandidate], task: &str) -> Value {
    json!({
        "task": task,
        "candidates": candidates
            .iter()
            .map(|c| json!({ "seq": c.seq, "chars": c.chars, "output": c.preview }))
            .collect::<Vec<_>>(),
    })
}

/// 裁定请求构造:每候选一个 noul 问题(id = `s<seq>`,与上下文裁判
/// 同式——seq 即锚点)
pub fn build_request(candidates: &[ValueCandidate], task: &str, model: &str) -> DecisionRequest {
    let state = build_state(candidates, task);
    let questions = candidates
        .iter()
        .enumerate()
        .map(|(n, c)| {
            (
                format!("s{}", c.seq),
                Question::Noul {
                    instructions: json!({
                        "question": crate::thresholds::FOLD_VALUE_QUESTION.replace(
                            "`candidates[@]`",
                            &format!("`candidates[{n}]`"),
                        ),
                    }),
                    criteria: None,
                },
            )
        })
        .collect();
    DecisionRequest {
        state,
        questions,
        model: model.to_string(),
    }
}

/// 答案解释(纯函数;**反向极性**):`noul ≤ drop_probability` 的候选 =
/// 不值得带入 checkpoint → 裁掉清单。
///
/// 非有限值(协议异常/缺席答案)一律不裁——fail-open 在解释层同样成立。
pub fn disposable(
    candidates: &[ValueCandidate],
    answers: &BTreeMap<String, Answer>,
    drop_probability: f64,
) -> Vec<(u64, f64)> {
    candidates
        .iter()
        .filter_map(|c| {
            let key = format!("s{}", c.seq);
            match answers.get(&key) {
                Some(Answer::Noul { noul }) if noul.is_finite() && *noul <= drop_probability => {
                    Some((c.seq, *noul))
                }
                _ => None,
            }
        })
        .collect()
}

impl liuma_agent_loop::value_judge::ValueJudge for FoldJudge {
    fn policy(&self) -> ValueJudgePolicy {
        ValueJudgePolicy {
            // 与上下文裁判同门槛:太短的输出不值得问一次
            min_chars: crate::thresholds::PRUNE_CANDIDATE_MIN_CHARS,
            max_questions: crate::thresholds::MAX_QUESTIONS_PER_REQUEST,
            preview_chars: crate::scenarios::context::PREVIEW_MAX_CHARS,
        }
    }

    fn judge<'a>(
        &'a self,
        candidates: &'a [ValueCandidate],
    ) -> Pin<Box<dyn Future<Output = Result<FoldAdvice, String>> + Send + 'a>> {
        Box::pin(async move {
            let total = candidates.len();
            let picked = &candidates[..total.min(self.policy().max_questions)];
            if picked.is_empty() {
                return Ok(FoldAdvice {
                    total,
                    ..FoldAdvice::default()
                });
            }
            let id = uuid::Uuid::now_v7().to_string();
            let request = build_request(picked, &self.recent_task(), &self.model);
            let asked = json!({
                "id": id,
                "scenario": "fold",
                "model": self.model,
                "questions": request
                    .questions
                    .iter()
                    .map(|(qid, _)| json!({ "id": qid, "kind": "noul" }))
                    .collect::<Vec<_>>(),
                "stateDigest": request.state_digest(),
            });
            let started = std::time::Instant::now();
            match self.port.ask(request).await {
                Ok(answers) => {
                    let duration_ms =
                        i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
                    let lost = disposable(
                        picked,
                        &answers.answers,
                        crate::thresholds::FOLD_DROP_PROBABILITY,
                    );
                    self.record(
                        asked,
                        json!({
                            "id": id, "ok": true,
                            "answers": serde_json::to_value(&answers.answers)
                                .unwrap_or(Value::Null),
                            "durationMs": duration_ms,
                            "usage": {
                                "inputTokens": answers.usage.input_tokens,
                                "outputTokens": answers.usage.output_tokens,
                            },
                        }),
                    );
                    // 只记录:裁掉条数在 answered 的答案值里可观察
                    // (「若生效会裁多少」),派生面不动
                    Ok(FoldAdvice {
                        no_value: lost.len(),
                        applied: false,
                        judged: picked.len(),
                        total,
                    })
                }
                Err(err) => {
                    // fail-open:裁定失败不落 pruned,receipt 收口留痕
                    let duration_ms =
                        i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
                    self.record(
                        asked,
                        json!({
                            "id": id, "ok": false,
                            "error": err.to_string(),
                            "durationMs": duration_ms,
                        }),
                    );
                    Err(err.to_string())
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeDecisionPort;
    use crate::types::DecisionAnswers;
    use liuma_agent_loop::value_judge::ValueJudge as _;
    use liuma_session::{EventEnvelope, EventLog};
    use std::sync::Mutex;

    fn candidate(seq: u64, chars: usize) -> ValueCandidate {
        ValueCandidate {
            seq,
            chars,
            preview: "x".repeat(20),
        }
    }

    fn answers(pairs: &[(u64, f64)]) -> BTreeMap<String, Answer> {
        pairs
            .iter()
            .map(|(seq, p)| (format!("s{seq}"), Answer::Noul { noul: *p }))
            .collect()
    }

    /// 按 `(seq, noul)` 作答的假端口(裁定场景的批量形态)
    fn port_answering(pairs: &[(u64, f64)]) -> FakeDecisionPort {
        FakeDecisionPort::scripted(vec![Ok(DecisionAnswers {
            model: "fake-1.0.0".into(),
            answers: answers(pairs),
            usage: crate::types::Usage::default(),
            request_id: None,
        })])
    }

    /// 只收集 receipt 类型的 sink
    fn type_sink(into: &Arc<Mutex<Vec<String>>>) -> crate::scenarios::ReceiptSink {
        let into = Arc::clone(into);
        Arc::new(move |ty: &str, _d: Value| {
            into.lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(ty.to_string());
        })
    }

    /// **反向极性锁**:低 noul(「不值得带走」)= 裁,高 noul = 留。
    /// 与上下文裁判的 `losing_seqs` 方向相反——照抄那份实现即翻车
    /// (同一份概率值在两个场景里给出相反的裁决)
    #[test]
    fn disposable_has_reversed_polarity_vs_context_judge() {
        let cands = vec![candidate(7, 5_000), candidate(9, 8_000)];
        let a = answers(&[(7, 0.05), (9, 0.95)]);
        // 折叠价值:0.05 那条裁掉,0.95 那条留下
        assert_eq!(
            disposable(&cands, &a, crate::thresholds::FOLD_DROP_PROBABILITY),
            vec![(7, 0.05)]
        );
        // 对照:同一份答案送上下文裁判 → 裁的是另一条(no_value ≥ 0.85)
        let ctx: Vec<crate::scenarios::context::PruneCandidate> = cands
            .iter()
            .map(|c| crate::scenarios::context::PruneCandidate {
                seq: c.seq,
                chars: c.chars,
                preview: c.preview.clone(),
            })
            .collect();
        assert_eq!(
            crate::scenarios::context::losing_seqs(
                &ctx,
                &a,
                crate::thresholds::PRUNE_NO_VALUE_PROBABILITY
            ),
            vec![(9, 0.95)]
        );
        // 阈值含端点(≤):恰在线上即裁
        let edge = answers(&[(7, crate::thresholds::FOLD_DROP_PROBABILITY)]);
        assert_eq!(
            disposable(&cands, &edge, crate::thresholds::FOLD_DROP_PROBABILITY).len(),
            1
        );
        // 非有限值/缺席答案不裁(fail-open 到解释层)
        let nan = answers(&[(7, f64::NAN)]);
        assert!(disposable(&cands, &nan, crate::thresholds::FOLD_DROP_PROBABILITY).is_empty());
        assert!(disposable(&cands, &BTreeMap::new(), 0.15).is_empty());
    }

    /// 请求构造:state 携带 task + 预览数组,每候选一问,问题内路径
    /// 指向自己的槽位(id 保序与 seq 对应)
    #[test]
    fn request_shape_binds_each_question_to_its_candidate() {
        let cands = vec![candidate(3, 2_000), candidate(5, 9_000)];
        let req = build_request(&cands, "fix the parser", "m");
        assert_eq!(req.questions.len(), 2);
        assert_eq!(req.questions[0].0, "s3");
        assert_eq!(req.questions[1].0, "s5");
        assert_eq!(req.state["task"], "fix the parser");
        assert_eq!(req.state["candidates"][1]["seq"], 5);
        assert_eq!(req.state["candidates"][1]["chars"], 9_000);
        let Question::Noul { instructions, .. } = &req.questions[1].1 else {
            panic!("noul");
        };
        let q = instructions["question"].as_str().expect("问题文本");
        assert!(q.contains("`candidates[1]`"), "{q}");
        assert!(!q.contains("@"), "槽位占位符已替换:{q}");
    }

    /// 只记录:落 receipt 对,**不落 pruned**(派生面不动),条数仍如实
    /// 回传给引擎(「若生效会裁多少」= 观察面)
    #[tokio::test]
    async fn advisory_records_receipt_without_pruning() {
        let cands = vec![candidate(3, 2_000)];
        let log = Arc::new(Mutex::new(EventLog::new()));
        let receipts = Arc::new(Mutex::new(Vec::<String>::new()));
        // 最近任务取自日志(真实用户消息)
        log.lock()
            .unwrap_or_else(|p| p.into_inner())
            .append(EventEnvelope::new(
                "user/message",
                0,
                json!({ "content": "fix the parser" }),
            ))
            .expect("append");
        let port = Arc::new(port_answering(&[(3, 0.02)]));
        let judge = FoldJudge::new(
            Arc::clone(&port) as Arc<dyn crate::DecisionPort>,
            "m".into(),
            Arc::clone(&log),
            type_sink(&receipts),
        );
        let advice = judge.judge(&cands).await.expect("裁定成功");
        assert_eq!(advice.no_value, 1);
        assert!(!advice.applied);
        assert_eq!((advice.judged, advice.total), (1, 1));
        let kinds = receipts.lock().unwrap_or_else(|p| p.into_inner()).clone();
        assert_eq!(kinds, vec!["decision/asked", "decision/answered"]);
        // 问题里带上了任务描述(state 路径根)
        let received = port.received.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(received[0].state["task"], "fix the parser");
    }

    /// fail-open:端口 Err 上抛给引擎(照常折叠),receipt 仍收口
    /// (ok:false)——不落 pruned
    #[tokio::test]
    async fn port_error_fails_open_with_closed_receipt() {
        let log = Arc::new(Mutex::new(EventLog::new()));
        let receipts = Arc::new(Mutex::new(Vec::<String>::new()));
        let judge = FoldJudge::new(
            Arc::new(FakeDecisionPort::failing(
                crate::client::DecisionError::Timeout,
            )) as Arc<dyn crate::DecisionPort>,
            "m".into(),
            Arc::clone(&log),
            type_sink(&receipts),
        );
        let err = judge
            .judge(&[candidate(3, 2_000)])
            .await
            .expect_err("端口失败上抛");
        assert!(err.contains("timeout"), "{err}");
        let kinds = receipts.lock().unwrap_or_else(|p| p.into_inner()).clone();
        assert_eq!(kinds, vec!["decision/asked", "decision/answered"]);
    }

    /// 上限:候选多于 max_questions 时只评估前 N 条(大输出优先序来自
    /// 引擎侧的候选排序),judged < total 诚实回传
    #[tokio::test]
    async fn candidate_cap_keeps_the_denominator_honest() {
        let cap = crate::thresholds::MAX_QUESTIONS_PER_REQUEST;
        let cands: Vec<ValueCandidate> =
            (1..=cap as u64 + 2).map(|n| candidate(n, 3_000)).collect();
        let keep: Vec<(u64, f64)> = (1..=cap as u64 + 2).map(|n| (n, 0.9)).collect();
        let log = Arc::new(Mutex::new(EventLog::new()));
        let receipts = Arc::new(Mutex::new(Vec::<String>::new()));
        let port = Arc::new(port_answering(&keep));
        let judge = FoldJudge::new(
            Arc::clone(&port) as Arc<dyn crate::DecisionPort>,
            "m".into(),
            Arc::clone(&log),
            type_sink(&receipts),
        );
        let advice = judge.judge(&cands).await.expect("裁定成功");
        assert_eq!(advice.judged, cap);
        assert_eq!(advice.total, cands.len());
        assert_eq!(advice.no_value, 0, "全留:高 noul 不裁");
        let received = port.received.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(received[0].questions.len(), cap);
    }
}
