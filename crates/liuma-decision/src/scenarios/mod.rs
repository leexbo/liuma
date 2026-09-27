//! 场景适配:决策端口在四个运行时场景的消费逻辑。
//!
//! 共同契约(社区共识):advisory/shadow 先行、fail-open、
//! 确定性代码持最终裁决、问题文案与阈值集中 thresholds.rs。

use std::sync::Arc;

/// 决策 receipt 落档口(宿主闭包:锁内 append 赋 seq,持久化由日志的
/// durability sink 独占——与 hook/* 落档同一模式)
pub type ReceiptSink = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

pub mod approvals;
pub mod context;
pub mod guard;
pub mod stop;
