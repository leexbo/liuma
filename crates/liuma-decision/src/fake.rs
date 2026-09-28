//! 测试装备:可编程假决策端口与假折叠价值裁定器。
//!
//! 端口两种主用法:①`scripted` 预置应答序列(逐次弹出)——驱动
//! 「低置信 Continue / 高置信 Pass」等分支断言;②`failing` 恒错
//! ——fail-open 回归锁(消费方在恒错下行为与未装配一致)。
//! `received` 记录全部请求,断言问题构造与 state 形状。
//!
//! [`ScriptedFoldJudge`] 是价值裁定端口的装备:不出网,回执与计数
//! 可脚本化,并记下引擎送来的候选批次(候选窗口回归)。

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use crate::types::{DecisionAnswers, DecisionRequest};
use liuma_agent_loop::value_judge::{FoldAdvice, ValueCandidate, ValueJudgePolicy};

/// 假端口(测试与集成回归专用;生产装配不出现)
pub struct FakeDecisionPort {
    script: Mutex<VecDeque<Result<DecisionAnswers, crate::client::DecisionError>>>,
    fallback: Mutex<crate::client::DecisionError>,
    /// 收到的请求(顺序记录)
    pub received: Mutex<Vec<DecisionRequest>>,
}

impl FakeDecisionPort {
    /// 空端口:脚本耗尽后走 fallback(默认 Timeout)
    pub fn new() -> Self {
        Self {
            script: Mutex::new(VecDeque::new()),
            fallback: Mutex::new(crate::client::DecisionError::Timeout),
            received: Mutex::new(Vec::new()),
        }
    }

    /// 恒错端口(fail-open 回归锁:消费方在任何 Err 下退回原路径)
    pub fn failing(err: crate::client::DecisionError) -> Self {
        Self {
            fallback: Mutex::new(err),
            ..Self::new()
        }
    }

    /// 预置应答脚本(逐次弹出;耗尽后 fallback)
    pub fn scripted(steps: Vec<Result<DecisionAnswers, crate::client::DecisionError>>) -> Self {
        Self {
            script: Mutex::new(steps.into()),
            ..Self::new()
        }
    }

    /// 追加一步应答
    pub fn then(&self, step: Result<DecisionAnswers, crate::client::DecisionError>) {
        self.script
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push_back(step);
    }

    /// 取走全部收到的请求(断言用)
    pub fn take_received(&self) -> Vec<DecisionRequest> {
        std::mem::take(&mut self.received.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// 便捷构造:单键 noul 答案(`request_id` 携带调用标记,断言配对用)
    pub fn noul_answers(key: &str, value: f64) -> DecisionAnswers {
        let mut answers = std::collections::BTreeMap::new();
        answers.insert(key.to_string(), crate::types::Answer::Noul { noul: value });
        DecisionAnswers {
            model: "fake-1.0.0".into(),
            answers,
            usage: crate::types::Usage::default(),
            request_id: None,
        }
    }
}

impl Default for FakeDecisionPort {
    fn default() -> Self {
        Self::new()
    }
}

/// 假折叠价值裁定器(引擎侧装配回归:不出网,回执/计数与成败可脚本化)。
///
/// 记下每次收到的候选批次(断言候选窗口与截断),按脚本回应;
/// 脚本耗尽回 [`FoldAdvice::default`](不改动任何东西)。
pub struct ScriptedFoldJudge {
    /// 逐次裁定的结果(耗尽后回默认值)
    steps: Mutex<VecDeque<Result<FoldAdvice, String>>>,
    /// 收到的候选批次(顺序记录)
    pub seen: Mutex<Vec<Vec<ValueCandidate>>>,
    policy: ValueJudgePolicy,
}

impl ScriptedFoldJudge {
    /// 单步裁定器(首次咨询回该结果)
    pub fn returning(result: Result<FoldAdvice, String>) -> Self {
        Self {
            steps: Mutex::new(VecDeque::from([result])),
            seen: Mutex::new(Vec::new()),
            policy: ValueJudgePolicy {
                min_chars: 2_000,
                max_questions: crate::thresholds::MAX_QUESTIONS_PER_REQUEST,
                preview_chars: 600,
            },
        }
    }

    /// 取走候选批次记录(断言用)
    pub fn take_seen(&self) -> Vec<Vec<ValueCandidate>> {
        std::mem::take(&mut self.seen.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

impl liuma_agent_loop::value_judge::ValueJudge for ScriptedFoldJudge {
    fn policy(&self) -> ValueJudgePolicy {
        self.policy
    }

    fn judge<'a>(
        &'a self,
        candidates: &'a [ValueCandidate],
    ) -> Pin<Box<dyn Future<Output = Result<FoldAdvice, String>> + Send + 'a>> {
        self.seen
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(candidates.to_vec());
        Box::pin(async move {
            self.steps
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .pop_front()
                .unwrap_or(Ok(FoldAdvice::default()))
        })
    }
}

impl crate::DecisionPort for FakeDecisionPort {
    fn ask(
        &self,
        req: DecisionRequest,
    ) -> Pin<
        Box<dyn Future<Output = Result<DecisionAnswers, crate::client::DecisionError>> + Send + '_>,
    > {
        self.received
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(req);
        Box::pin(async move {
            let next = self
                .script
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .pop_front();
            match next {
                Some(step) => step,
                None => Err(self
                    .fallback
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone()),
            }
        })
    }
}
