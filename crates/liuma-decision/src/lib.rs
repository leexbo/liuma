//! liuma-decision:决策模型接入(System One 协议)。
//!
//! System One 模型不生成文本:输入 state + 类型化问题
//! (noul/choice/score),返回约束在预声明选项集内的结构化答案
//! (概率 + 校准 confidence)。协议按开放品类接入——TypeSafe Jev
//! 与阿里百炼兼容同一形状,厂商差异经类型容忍(见 [`types`])。
//!
//! 边界与哲学:
//! - 传输在宿主(WIT world 即授权面;D5:wasi-http 不进组件)。
//! - fail-open 契约:端口任何 `Err` 一律退回原路径,消费方
//!   (审批评审员/Stop 哨兵/工具守卫/上下文裁判)绝不因决策端
//!   不可用而阻塞或改变默认行为。
//! - 每次询问留 receipt:`decision/asked` + `decision/answered`
//!   事件对(liuma-session),state 不落本体只记摘要。
//! - 阈值与问题文案集中 [`thresholds`](crate::thresholds) 单处,评审唯一入口。

#![deny(missing_docs)]

pub mod client;
pub mod fake;
pub mod scenarios;
pub mod thresholds;
pub mod tool;
pub mod types;

use std::future::Future;
use std::pin::Pin;

pub use client::{DecisionError, SystemOneClient, fit_state, retry_decision};
pub use fake::FakeDecisionPort;
pub use tool::{DECIDE_TOOL_NAME, DecideTool};
pub use types::{Answer, DecisionAnswers, DecisionRequest, NoulCriteria, Question, Usage};

/// 决策端口:一次询问,答案按请求问题 id 一一对应返回。
///
/// 对象安全形态(消费方经 `Arc<dyn DecisionPort>` 持有;装配层
/// 「port 缺 = 组件跳过」)。实现方负责超时/重试/截断;调用方负责
/// fail-open 消费与审计落档。
pub trait DecisionPort: Send + Sync {
    /// 发起一次询问(整体受硬截止约束;Err = fail-open 信号)
    fn ask(
        &self,
        req: DecisionRequest,
    ) -> Pin<Box<dyn Future<Output = Result<DecisionAnswers, DecisionError>> + Send + '_>>;
}
