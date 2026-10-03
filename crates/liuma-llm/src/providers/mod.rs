//! 厂商目录:每家 provider 一个文件,持有其方言声明与行为数据。
//!
//! - [`openai`] / [`deepseek`] / [`glm`] — 自有方言的厂商
//!   (方言 struct + 能力表 + 错误数据 + hosted 声明);
//! - [`anthropic`] — anthropic-messages 面(官方 + 全部 `/anthropic`
//!   兼容端点共用)的**面属主**:跨厂商并集表(思考矩阵/能力/hosted
//!   版本表)在此维护,兼容厂商(kimi/qwen/minimax 等)不另立文件;
//! - 方言注册表 [`DIALECT_NAMES`] 为全仓单一事实源(liuma-core
//!   upsert 校验与桌面下拉消费)。

pub mod anthropic;
pub mod deepseek;
pub mod glm;
pub mod openai;

pub use anthropic::AnthropicMessagesAdapter;
pub use deepseek::{DeepSeekChatAdapter, DeepSeekResponsesAdapter};
pub use glm::GlmResponsesAdapter;
pub use openai::{OpenAiCompletionsAdapter, OpenAiResponsesAdapter};

use crate::behaviors::ModelCaps;

/// 方言名全集(配置/CLI 的 `dialect` 字段取值;单一事实源)。
pub const DIALECT_NAMES: &[&str] = &[
    "deepseek-responses",
    "openai-responses",
    "glm-responses",
    "deepseek-chat",
    "openai-completions",
    "anthropic-messages",
];

/// 按方言名 + 模型名查输入能力(供 liuma-core/桌面等装配层消费;
/// 未知方言返回 None,未知模型返回该方言缺省)。
pub fn model_caps(dialect: &str, model: &str) -> Option<ModelCaps> {
    let adapter = crate::adapter_by_name(dialect)?;
    let b = adapter.behaviors();
    Some(crate::behaviors::resolve_caps(
        b.model_caps,
        b.default_caps,
        model,
    ))
}
