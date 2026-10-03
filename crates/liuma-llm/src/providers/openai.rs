//! OpenAI:Responses 与 Chat Completions 两方言 + 家族默认行为。

use crate::adapters::{ProviderAdapter, RigFamily};
use crate::behaviors::{HostedToolDecl, ProviderBehaviors};
use rig_core::providers::openai::wire::OPENAI;

/// OpenAI Responses 方言(hosted 工具声明:OpenAI 服务端 web_search,
/// rig `ResponsesToolDefinition::web_search()` 同款 type 值)。
#[derive(Debug)]
pub struct OpenAiResponsesAdapter {
    /// 行为组(可替换;默认 = OpenAI 家族 + responses 面 hosted 表)
    pub behaviors: ProviderBehaviors,
}

impl ProviderAdapter for OpenAiResponsesAdapter {
    fn name(&self) -> &'static str {
        "openai-responses"
    }
    fn family(&self) -> RigFamily {
        RigFamily::OpenAi {
            dialect: OPENAI,
            responses: true,
        }
    }
    fn behaviors(&self) -> &ProviderBehaviors {
        &self.behaviors
    }
    fn effort_params(&self, _model: &str, effort: &str) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "reasoning": { "effort": effort } }))
    }
}

/// OpenAI 兼容净版 chat 方言(无私有字段,不发思考强度)。
#[derive(Debug, Default)]
pub struct OpenAiCompletionsAdapter {
    /// 行为组(可替换;默认 = OpenAI 家族)
    pub behaviors: ProviderBehaviors,
}

impl ProviderAdapter for OpenAiCompletionsAdapter {
    fn name(&self) -> &'static str {
        "openai-completions"
    }
    fn family(&self) -> RigFamily {
        RigFamily::OpenAi {
            dialect: OPENAI,
            responses: false,
        }
    }
    fn behaviors(&self) -> &ProviderBehaviors {
        &self.behaviors
    }
    fn effort_params(&self, _model: &str, _effort: &str) -> Option<serde_json::Value> {
        None
    }
}

/// responses 面 hosted 工具声明(OpenAI 服务端 web_search;wire 形态
/// `{"type":"web_search"}`,无 name 字段)
pub const HOSTED_TOOLS: &[HostedToolDecl] = &[HostedToolDecl {
    kind: "web_search",
    model_match: "",
    wire_type: "web_search",
    wire_name: None,
}];

impl Default for OpenAiResponsesAdapter {
    /// 默认构造(openai 家族 + responses 面 hosted 表)
    fn default() -> Self {
        Self {
            behaviors: ProviderBehaviors {
                hosted_tools: HOSTED_TOOLS,
                ..ProviderBehaviors::openai_family()
            },
        }
    }
}
