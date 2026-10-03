//! Provider 方言适配器:trait 契约 + rig 家族声明 + 注册 glue。
//!
//! 方言 struct 与行为数据在 [`crate::providers`](每厂商一个文件);
//! 本文件只留契约面与共享辅助。引擎内部为
//! rig-core 0.43(四评 + spike,见 docs/plans/dialect-first-class.md):
//! 请求编码/流解码经 rig wire 层,差异全部是 [`crate::behaviors`] 数据。
//!
//! 内部消息方言(日志派生)在 [`crate::translate`] 翻译为 rig 核心
//! Message——日志与不变式比对始终用内部形状(「模型可见 ⟺ 已记录」
//! 的比对面不随 provider 变化)。

use std::collections::HashMap;

use liuma_agent_loop::TransportError;
use serde_json::Value;

use liuma_agent_loop::RequestHeader;

use crate::behaviors::{
    HostedToolDecl, ProviderBehaviors, body_error_by_shape, refine_by_codes, resolve_caps,
    resolve_hosted, thinking_params,
};

// 方言 struct 再导出(稳定历史路径;实现移入厂商文件)
pub use crate::providers::anthropic::AnthropicMessagesAdapter;
pub use crate::providers::deepseek::{DeepSeekChatAdapter, DeepSeekResponsesAdapter};
pub use crate::providers::glm::GlmResponsesAdapter;
pub use crate::providers::openai::{OpenAiCompletionsAdapter, OpenAiResponsesAdapter};

/// rig provider 家族:OpenAI 兼容 wire(Responses/Chat 路由二选一)或
/// Anthropic Messages。
/// clippy:变体尺寸差 = Dialect 常量 vs 标记变体,两者皆 Copy 且小,
/// 装箱不划算
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RigFamily {
    /// OpenAI 兼容 wire:`dialect` 声明方言常量,`responses` 选
    /// `POST {base}/responses`(`provider.responses()`),否则
    /// `POST {base}/chat/completions`(`provider.chat()`)。
    OpenAi {
        /// rig 方言常量(OPENAI/DEEPSEEK 或 GLM 等自定义值)
        dialect: rig_core::providers::openai::wire::Dialect,
        /// Responses 路由
        responses: bool,
    },
    /// Anthropic Messages(x-api-key + version;`AnthropicConfig`)
    Anthropic,
}

/// provider 方言差异声明(连接语义之外的全部差异)。
///
/// 方法实现只读方言持有的 [`ProviderBehaviors`] 数据——新增差异种类
/// 时扩 behaviors 与解释器,不在各方言里堆逻辑。
pub trait ProviderAdapter: Send + Sync {
    /// 方言名(配置/CLI 选择用)
    fn name(&self) -> &'static str;
    /// rig provider 家族与路由
    fn family(&self) -> RigFamily;
    /// 行为组(思考矩阵/错误形态/业务码/能力目录/hosted 声明;
    /// 可经自定义方言实例替换)
    fn behaviors(&self) -> &ProviderBehaviors;
    /// 模型是否接受图片输入(模型级;未编目模型落行为组缺省)
    fn supports_images(&self, model: &str) -> bool {
        resolve_caps(
            self.behaviors().model_caps,
            self.behaviors().default_caps,
            model,
        )
        .image
    }
    /// anthropic 形态的必填 max_tokens(其余家族不发该键)
    fn max_tokens(&self) -> u32 {
        4096
    }
    /// 200 + 非 SSE 的 JSON 错误体归类(None = 非错误体,按空响应处理)
    fn body_error(&self, body: &str) -> Option<TransportError> {
        body_error_by_shape(self.behaviors().error_shape, body)
    }
    /// 请求面思考强度 → additional_params 注入(rig 编码后落 body
    /// 顶层;`None` = 该方言/模型不发思考强度)。默认走思考矩阵;
    /// 方言级表达(responses/chat 家族)由厂商文件覆写
    fn effort_params(&self, model: &str, effort: &str) -> Option<Value> {
        thinking_params(&self.behaviors().thinking_rules, model, effort)
    }
    /// 非 2xx 错误体精化:通用状态归类后按业务码表改判(默认直通)
    fn refine_error(&self, err: TransportError, body: &str) -> TransportError {
        refine_by_codes(&self.behaviors().business_codes, err, body)
    }
    /// hosted 工具声明解析(kind + 模型 → wire 形态;None = 该面/模型
    /// 不提供,调用方 fail-fast)
    fn hosted_tool(&self, kind: &str, model: &str) -> Option<HostedToolDecl> {
        resolve_hosted(self.behaviors().hosted_tools, kind, model)
    }
}

/// 按方言名构造 adapter(配置/CLI 的 `dialect` 字段入口;取值域见
/// [`crate::providers::DIALECT_NAMES`] 单一事实源)。
///
/// 未知名字返回 None(调用方 fail-fast,不静默回退默认方言)。
/// 自定义 provider(新 base_url/行为覆写)不走本表:直接构造方言
/// struct(字段 pub,行为组可换)经 [`crate::HttpTransport::with_adapter`]
/// 接入。
pub fn adapter_by_name(name: &str) -> Option<Box<dyn ProviderAdapter>> {
    match name {
        "deepseek-responses" => Some(Box::new(DeepSeekResponsesAdapter::default())),
        "openai-responses" => Some(Box::new(OpenAiResponsesAdapter::default())),
        "glm-responses" => Some(Box::new(GlmResponsesAdapter::default())),
        "deepseek-chat" => Some(Box::new(DeepSeekChatAdapter::default())),
        "openai-completions" => Some(Box::new(OpenAiCompletionsAdapter::default())),
        "anthropic-messages" => Some(Box::new(AnthropicMessagesAdapter::default())),
        _ => None,
    }
}

/// 工具调用 id → 名字的索引(内部 tool 消息不携带工具名,rig 的
/// ToolResult 需要 name;从同会话先前的 assistant tool_calls 反查)
pub(crate) fn index_call_names(messages: &Value) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for m in messages.as_array().into_iter().flatten() {
        for call in m["tool_calls"].as_array().into_iter().flatten() {
            if let (Some(id), Some(name)) = (
                call["id"].as_str().filter(|s| !s.is_empty()),
                call["name"].as_str().filter(|s| !s.is_empty()),
            ) {
                names.insert(id.to_string(), name.to_string());
            }
        }
    }
    names
}

/// 请求面工具声明分区:header.tools(OpenAI chat 嵌套形)中
/// function 条目 → rig 核心 `ToolDefinition`;其余条目原样返回
/// (hosted 工具条目,如 `{"type":"web_search",…}`,由 engine 按
/// 方言声明翻译)。修「非 function 条目被拍平成空名定义静默发出」
/// 的隐患(分区后 hosted 面不再进 function 通道)。
pub(crate) fn partition_tools(
    header: &RequestHeader,
) -> (Vec<rig_core::completion::ToolDefinition>, Vec<Value>) {
    let mut functions = Vec::new();
    let mut hosted = Vec::new();
    for t in &header.tools {
        if t["type"] == "function" {
            functions.push(rig_core::completion::ToolDefinition {
                name: t["function"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                description: t["function"]["description"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                parameters: t["function"]["parameters"].clone(),
            });
        } else {
            hosted.push(t.clone());
        }
    }
    (functions, hosted)
}
