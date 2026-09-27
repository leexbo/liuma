//! 测试装备:可编程假决策端口。
//!
//! 两种主用法:①`scripted` 预置应答序列(逐次弹出)——驱动
//! 「低置信 Continue / 高置信 Pass」等分支断言;②`failing` 恒错
//! ——fail-open 回归锁(消费方在恒错下行为与未装配一致)。
//! `received` 记录全部请求,断言问题构造与 state 形状。

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use crate::types::{DecisionAnswers, DecisionRequest};

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
