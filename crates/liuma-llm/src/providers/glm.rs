//! GLM(智谱 bigmodel/z.ai):Responses 方言 + 能力目录 + 错误契约。
//!
//! 注意 anthropic 面的站点级覆盖(bigmodel/z.ai 的 `/api/anthropic`
//! 全模型开图/视频)在面属主 [`crate::providers::anthropic`] 的并集表
//! 维护,不在此。

use crate::adapters::{ProviderAdapter, RigFamily};
use crate::behaviors::{BusinessCodes, ErrorBodyShape, ModelCap, ModelCaps, ProviderBehaviors};
use rig_core::providers::openai::wire::Dialect;

/// GLM(bigmodel)的自定义方言值:OpenAI wire 基线 + Responses 路由
/// (路由由 [`RigFamily::OpenAi`] 的 `responses` 位选择,不经方言 quirks)。
const fn glm_dialect() -> Dialect {
    Dialect::gateway("glm", "https://open.bigmodel.cn/api/v1", "BIGMODEL_API_KEY")
}

/// GLM Responses 方言。
///
/// 与 OpenAI Responses 请求面同形;差异在错误面——端点对坏 key 等以
/// **200 + application/json 顶层错误体**应答(2026-09-12 实测),官方
/// 契约另有嵌套形态,两形态并收;业务码表见 [`GLM_BUSINESS_CODES`]。
#[derive(Debug)]
pub struct GlmResponsesAdapter {
    /// 行为组(可替换;默认 = GLM 双形态错误 + 官方业务码 + 能力目录)
    pub behaviors: ProviderBehaviors,
}

impl ProviderAdapter for GlmResponsesAdapter {
    fn name(&self) -> &'static str {
        "glm-responses"
    }
    fn family(&self) -> RigFamily {
        RigFamily::OpenAi {
            dialect: glm_dialect(),
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

/// GLM(bigmodel)官方错误码分类(2026-10 官方错误码表)
pub const GLM_BUSINESS_CODES: BusinessCodes = BusinessCodes {
    overflow: &[1261],
    fatal: &[1113, 1309, 1311, 1314, 1315],
};

/// 模型能力目录(Responses/chat 面):
/// glm-5.3-flash 有图/视频/pdf(998 行覆盖 978 的 glm-5.3 纯文本),
/// 5v-turbo 与 4.6v 系有图,其余纯文本。
pub const MODEL_CAPS: &[ModelCap] = &[
    ModelCap {
        model_match: "glm-5.3-flash",
        caps: ModelCaps {
            image: true,
            video: true,
            pdf: true,
        },
    },
    ModelCap {
        model_match: "glm-5v-turbo",
        caps: ModelCaps {
            image: true,
            video: true,
            pdf: false,
        },
    },
    ModelCap {
        model_match: "glm-4.6v",
        caps: ModelCaps {
            image: true,
            video: false,
            pdf: false,
        },
    },
];

/// GLM 行为组默认(双形态错误 + 官方业务码 + 能力目录 + text-only 缺省)
pub fn behaviors() -> ProviderBehaviors {
    ProviderBehaviors {
        error_shape: ErrorBodyShape::TopLevelThenNested,
        business_codes: GLM_BUSINESS_CODES,
        model_caps: MODEL_CAPS,
        default_caps: ModelCaps::TEXT_ONLY,
        ..ProviderBehaviors::openai_family()
    }
}

impl Default for GlmResponsesAdapter {
    fn default() -> Self {
        Self {
            behaviors: behaviors(),
        }
    }
}
