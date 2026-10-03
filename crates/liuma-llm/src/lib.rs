//! LLM 接入(liuma-llm):rig 0.43 provider 层 + 方言声明 + 传输桥 + 不变式闸门。
//!
//! llm 缝 + provider 实现分离:端口
//! [`liuma_agent_loop::LlmTransport`] 定义在组件侧(liuma-agent-loop,对应
//! WIT `liuma:host/llm-transport` import),本 crate 是宿主的实现——替换
//! provider 方言/实现 = 换 crate 依赖,不触碰宿主核心
//! (「一切皆插件」在 crate 层的落地)。
//!
//! 引擎内部为 rig-core 0.43(四评 + spike 后切换,见
//! docs/plans/dialect-first-class.md):请求编码/流解码/组帧归 rig wire
//! 层;liuma 侧只留四条边界——
//! - [`adapters`] — 方言差异声明(六方言 → rig 家族/路由、思考注入、
//!   图片支持、200+JSON 错误体归类;原生方言 + base_url 即接入
//!   全部 /anthropic 端点,不轻扩张);
//! - [`translate`] — 内部消息方言(日志形状)↔ rig Message;
//! - [`bridge`] — 自有 [`rig_http`] `HttpClientExt` 传输桥(连接语义:
//!   超时/错误体嗅探;重试在 engine,D41);
//! - [`engine`] — rig 驱动 + 事件映射(`LlmEvent` 约定/TTFT/错误归类);
//! - [`attachments`] — 图片附件请求期处理(字节来源 trait + offload);
//! - [`http`] — transport 组装([`LlmTransport`] 实现与 Summarizer);
//! - [`invariant`] — 「模型可见 ⟺ 已记录」derive-and-compare 校验器(E1);
//! - [`transport`] — 不变式闸门([`InvariantGate`])与假 provider(测试装备)。

#![deny(missing_docs)]

pub mod adapters;
pub mod attachments;
pub mod behaviors;
mod bridge;
mod engine;
pub mod http;
pub mod invariant;
pub mod providers;
pub mod translate;
pub mod transport;
pub mod usage;

pub use adapters::{ProviderAdapter, RigFamily, adapter_by_name};
pub use attachments::{
    AttachmentSource, MAX_REQUEST_IMAGE_BYTES, NoAttachments, OFFLOADED_IMAGE_TEXT,
    file_handle_text, offload_request_images, project_files_to_text, strip_images_for_summary,
};
pub use behaviors::{HostedToolDecl, ModelCap, ModelCaps, ProviderBehaviors, ThinkingRule};
pub use http::{HttpTransport, ProviderConfig};
pub use providers::DIALECT_NAMES;
pub use transport::{FakeProvider, GateError, InvariantGate};
