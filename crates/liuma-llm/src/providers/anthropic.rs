//! Anthropic Messages 面:官方与全部 `/anthropic` 兼容端点
//! (DeepSeek/GLM/Kimi/MiniMax/阿里/小米等)共用的方言。
//!
//! 本文件是该面的**属主**:跨厂商并集表(思考矩阵/能力/hosted 工具
//! 版本表)在此维护——兼容厂商不另立文件,模型名即键。
//!
//! 表内结论均经一手核实(2026-10-02)。

use crate::adapters::{ProviderAdapter, RigFamily};
use crate::behaviors::{
    EffortField, ErrorBodyShape, HostedToolDecl, ModelCap, ModelCaps, ProviderBehaviors,
    ThinkingRule, ThinkingType,
};

/// Anthropic Messages 方言。
#[derive(Debug)]
pub struct AnthropicMessagesAdapter {
    /// 行为组(可替换;默认 = 面属主并集表)
    pub behaviors: ProviderBehaviors,
}

/// 按厂商的 max_tokens 覆写(模型名小写子串,首条命中;厂商参数
/// 上限低于 [`DEFAULT_MAX_TOKENS`] 时发超限值会被参数校验拒绝。
/// 值贴着厂商上限留出余量,不顶格)
pub const MAX_TOKENS_OVERRIDES: &[(&str, u32)] = &[
    ("glm", 128_000),
    // 与默认同值,显式声明厂商口径
    ("deepseek", 256_000),
];

impl ProviderAdapter for AnthropicMessagesAdapter {
    fn name(&self) -> &'static str {
        "anthropic-messages"
    }
    fn family(&self) -> RigFamily {
        RigFamily::Anthropic
    }
    fn behaviors(&self) -> &ProviderBehaviors {
        &self.behaviors
    }
    fn max_tokens(&self, model: &str) -> u32 {
        let m = model.to_ascii_lowercase();
        MAX_TOKENS_OVERRIDES
            .iter()
            .find(|(needle, _)| m.contains(needle))
            .map(|(_, cap)| *cap)
            .unwrap_or(crate::adapters::DEFAULT_MAX_TOKENS)
    }
}

/// anthropic 兼容面思考矩阵:
/// glm-5.3/deepseek/qwen3.8 = thinking:enabled + output_config:{effort};
/// glm 老模型仅开关;kimi 仅档位。**矩阵外(官方 claude 等)不发**——
/// 官方思考表达是 budget_tokens 机制,未知字段有 400 风险。
pub const ANTHROPIC_THINKING_RULES: &[ThinkingRule] = &[
    ThinkingRule {
        model_match: "glm-5.3",
        ..ThinkingRule::ENABLED_AND_EFFORT
    },
    ThinkingRule {
        model_match: "deepseek",
        ..ThinkingRule::ENABLED_AND_EFFORT
    },
    ThinkingRule {
        model_match: "qwen3.8",
        ..ThinkingRule::ENABLED_AND_EFFORT
    },
    ThinkingRule {
        model_match: "glm",
        thinking_type: Some(ThinkingType::Enabled),
        effort_at: None,
    },
    ThinkingRule {
        model_match: "kimi",
        thinking_type: None,
        effort_at: Some(EffortField::OutputConfigEffort),
    },
];

/// anthropic 面能力并集(特例在前;缺省 = 图开——官方 claude 与多数
/// 兼容端点的现代模型收图,GLM 站点级覆盖亦全开;误发纯文本模型
/// 可见 400 而非误降级):
/// - GLM:bigmodel/z.ai 的 `/api/anthropic` 站点级全模型开图/视频
///   (含目录里 img=false 的 glm-5.3);
/// - kimi-k3/k3:图+视频(k3-256k 仅图,特例条目在前);
/// - minimax-m3/m3:图+视频,m2.x 纯文本;
/// - qwen3.8:图+视频,qwen 其余纯文本;
/// - deepseek:flash 系图,其余纯文本。
pub const ANTHROPIC_FACE_CAPS: &[ModelCap] = &[
    // GLM 站点覆盖(anthropic 面专属,优先于厂商目录的同名结论)
    ModelCap {
        model_match: "glm",
        caps: ModelCaps {
            image: true,
            video: true,
            pdf: false,
        },
    },
    // kimi(k3-256k 仅图,先于 k3 通配)
    ModelCap {
        model_match: "k3-256k",
        caps: ModelCaps {
            image: true,
            video: false,
            pdf: false,
        },
    },
    ModelCap {
        model_match: "k3",
        caps: ModelCaps {
            image: true,
            video: true,
            pdf: false,
        },
    },
    // kimi-k2.x:图(视频 k2.5 无)
    ModelCap {
        model_match: "kimi-k2.7",
        caps: ModelCaps {
            image: true,
            video: true,
            pdf: false,
        },
    },
    ModelCap {
        model_match: "kimi-k2.6",
        caps: ModelCaps {
            image: true,
            video: true,
            pdf: false,
        },
    },
    ModelCap {
        model_match: "kimi-k2.5",
        caps: ModelCaps {
            image: true,
            video: false,
            pdf: false,
        },
    },
    // minimax
    ModelCap {
        model_match: "minimax-m3",
        caps: ModelCaps {
            image: true,
            video: true,
            pdf: false,
        },
    },
    ModelCap {
        model_match: "m3",
        caps: ModelCaps {
            image: true,
            video: true,
            pdf: false,
        },
    },
    ModelCap {
        model_match: "minimax-m2",
        caps: ModelCaps::TEXT_ONLY,
    },
    ModelCap {
        model_match: "minimax-m2.",
        caps: ModelCaps::TEXT_ONLY,
    },
    // qwen(3.8 先于通配)
    ModelCap {
        model_match: "qwen3.8",
        caps: ModelCaps {
            image: true,
            video: true,
            pdf: false,
        },
    },
    ModelCap {
        model_match: "qwen",
        caps: ModelCaps::TEXT_ONLY,
    },
    // deepseek(与厂商目录一致;anthropic 面独立收录)
    ModelCap {
        model_match: "deepseek-v4.1-flash",
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
    ModelCap {
        model_match: "deepseek",
        caps: ModelCaps::TEXT_ONLY,
    },
];

/// anthropic 面 hosted 工具版本表(带模型门控的条目在前):
/// - deepseek `/anthropic` 与官方 claude:`web_search_20250305`(官方
///   文档唯一版本;GLM 兼容面同样收);
/// - GLM(bigmodel/z.ai `/api/anthropic`):`web_search_20260209`
///   (服务端反序列化只认此与 20250305——日期段不带下划线,实测
///   `web_search_2026_02_09` 被 400 拒);
/// - 兼容矩阵外模型不声明(fail-fast,不盲发未知 type)。
pub const ANTHROPIC_FACE_HOSTED: &[HostedToolDecl] = &[
    HostedToolDecl {
        kind: "web_search",
        model_match: "deepseek",
        wire_type: "web_search_20250305",
        wire_name: Some("web_search"),
    },
    HostedToolDecl {
        kind: "web_search",
        model_match: "glm",
        wire_type: "web_search_20260209",
        wire_name: Some("web_search"),
    },
    HostedToolDecl {
        kind: "web_search",
        model_match: "claude",
        wire_type: "web_search_20250305",
        wire_name: Some("web_search"),
    },
];

/// 面属主默认行为组(并集表 + 嵌套错误体 + 图开缺省)
pub fn behaviors() -> ProviderBehaviors {
    ProviderBehaviors {
        thinking_rules: ANTHROPIC_THINKING_RULES.to_vec(),
        error_shape: ErrorBodyShape::Nested,
        business_codes: Default::default(),
        model_caps: ANTHROPIC_FACE_CAPS,
        default_caps: ModelCaps {
            image: true,
            video: false,
            pdf: true,
        },
        hosted_tools: ANTHROPIC_FACE_HOSTED,
    }
}

impl Default for AnthropicMessagesAdapter {
    fn default() -> Self {
        Self {
            behaviors: behaviors(),
        }
    }
}
