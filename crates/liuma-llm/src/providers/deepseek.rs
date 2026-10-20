//! DeepSeek:Responses 与 Chat 两方言 + 模型能力目录。
//!
//! 能力结论(2026-10-02 核实):flash 系(V41 代)有图,
//! v4-pro/v4-flash 纯文本。

use crate::adapters::{ProviderAdapter, RigFamily};
use crate::behaviors::{ModelCap, ModelCaps, ProviderBehaviors};
use rig_core::providers::openai::wire::DEEPSEEK;

/// DeepSeek Responses 方言(OpenAI Responses 家族契约,effort 同形)。
#[derive(Debug)]
pub struct DeepSeekResponsesAdapter {
    /// 行为组(可替换;默认 = DeepSeek 能力目录)
    pub behaviors: ProviderBehaviors,
}

impl ProviderAdapter for DeepSeekResponsesAdapter {
    fn name(&self) -> &'static str {
        "deepseek-responses"
    }
    fn family(&self) -> RigFamily {
        RigFamily::OpenAi {
            dialect: DEEPSEEK,
            responses: true,
        }
    }
    fn behaviors(&self) -> &ProviderBehaviors {
        &self.behaviors
    }
    /// OpenAI Responses 家族契约:`reasoning:{effort}`(DeepSeek 与
    /// OpenAI 同形;方言级表达,不经模型矩阵)。`off` 无可表达形态
    /// (reasoning 只收档位)→ None,回落服务端默认
    fn effort_params(&self, _model: &str, effort: &str) -> Option<serde_json::Value> {
        if effort == "off" {
            return None;
        }
        Some(serde_json::json!({ "reasoning": { "effort": effort } }))
    }
}

/// DeepSeek chat 方言(V4 系思考控制)。
///
/// 思考控制 wire 形态(8d1e722 事故锁):`thinking` 与 `reasoning_effort`
/// 是**两个顶层字段**——嵌进 thinking 对象不合服务端 schema、被丢弃,
/// 模型不思考。
#[derive(Debug)]
pub struct DeepSeekChatAdapter {
    /// 行为组(可替换;默认 = DeepSeek 能力目录)
    pub behaviors: ProviderBehaviors,
}

impl ProviderAdapter for DeepSeekChatAdapter {
    fn name(&self) -> &'static str {
        "deepseek-chat"
    }
    fn family(&self) -> RigFamily {
        RigFamily::OpenAi {
            dialect: DEEPSEEK,
            responses: false,
        }
    }
    fn behaviors(&self) -> &ProviderBehaviors {
        &self.behaviors
    }
    fn effort_params(&self, _model: &str, effort: &str) -> Option<serde_json::Value> {
        // off = 显式关思考:只发开关字段(档位值域 low/high/max,off
        // 非法 → 不带 reasoning_effort;8d1e722 两字段形态保持)
        if effort == "off" {
            return Some(serde_json::json!({ "thinking": { "type": "disabled" } }));
        }
        Some(serde_json::json!({
            "thinking": { "type": "enabled" },
            "reasoning_effort": effort,
        }))
    }
}

/// 模型能力目录(特例在前,未编目 → text-only):
/// - `deepseek-flash` / `v4.1-flash` / `v4-1-flash` → 图;
/// - `v4-pro` / `v4-flash` → 纯文本(缺省即 text-only,显式列出
///   便于读者);
/// - 旧 deepseek-chat/reasoner 落缺省 text-only。
pub const MODEL_CAPS: &[ModelCap] = &[
    ModelCap {
        model_match: "deepseek-v4.1-flash",
        caps: ModelCaps {
            image: true,
            video: false,
            pdf: false,
        },
    },
    ModelCap {
        model_match: "deepseek-v4-1-flash",
        caps: ModelCaps {
            image: true,
            video: false,
            pdf: false,
        },
    },
    ModelCap {
        model_match: "deepseek-flash",
        caps: ModelCaps {
            image: true,
            video: false,
            pdf: false,
        },
    },
];

/// DeepSeek 行为组默认(openai 家族 + 本厂能力目录 + text-only 缺省)
pub fn behaviors() -> ProviderBehaviors {
    ProviderBehaviors {
        model_caps: MODEL_CAPS,
        default_caps: ModelCaps::TEXT_ONLY,
        ..ProviderBehaviors::openai_family()
    }
}

impl Default for DeepSeekResponsesAdapter {
    fn default() -> Self {
        Self {
            behaviors: behaviors(),
        }
    }
}

impl Default for DeepSeekChatAdapter {
    fn default() -> Self {
        Self {
            behaviors: behaviors(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::adapters::adapter_by_name;

    /// off 档按面分叉:chat 面只发开关字段不带档位(8d1e722 形态的
    /// 第三态);responses 面 reasoning 只收档位 → None 回落默认
    #[test]
    fn off_effort_shapes_per_family() {
        let chat = adapter_by_name("deepseek-chat").unwrap();
        let p = chat.effort_params("deepseek-flash", "off").unwrap();
        assert_eq!(p, serde_json::json!({ "thinking": { "type": "disabled" } }));
        let p = chat.effort_params("deepseek-flash", "high").unwrap();
        assert_eq!(
            p,
            serde_json::json!({
                "thinking": { "type": "enabled" },
                "reasoning_effort": "high",
            })
        );
        let responses = adapter_by_name("deepseek-responses").unwrap();
        assert!(responses.effort_params("deepseek-flash", "off").is_none());
        let p = responses.effort_params("deepseek-flash", "low").unwrap();
        assert_eq!(p, serde_json::json!({ "reasoning": { "effort": "low" } }));
    }
}
