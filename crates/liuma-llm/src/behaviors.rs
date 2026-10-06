//! Provider 行为差异数据层:思考表达规则、错误体形态、业务码分类、
//! 模型能力标记、hosted 工具声明。
//!
//! 设计原则(与本仓 D40 计费配置同款):**差异是数据,代码只有
//! 解释器**。数据由 [`crate::providers`] 各厂商文件构建
//! (一手源码转录);接入新 provider = 追加规则条目或换行为组,不改
//! 解释器。

use liuma_agent_loop::TransportError;
use serde_json::{Value, json};

// ============================================================
// 思考表达
// ============================================================

/// 思考字段形态(anthropic 兼容面各家不同;官方 Anthropic 的
/// budget_tokens 机制不在本表——未知字段有 400 风险,矩阵外不发)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThinkingType {
    /// 显式开启(GLM-5.3 / DeepSeek / Qwen3.8)
    Enabled,
    /// 自适应思考(claude-5 兼容形态)
    Adaptive,
}

/// 思考档位挂载点(矩阵内唯一形态;留枚组便于未来扩展)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffortField {
    /// `output_config: {effort: <档位>}`
    OutputConfigEffort,
}

/// 一条思考表达规则:模型名(小写子串)命中 → 发哪些字段。
/// 首条命中即用(特例在前,通配在后);无命中 = 不发。
#[derive(Clone, Debug)]
pub struct ThinkingRule {
    /// 模型名小写子串(如 "glm-5.3"、"deepseek")
    pub model_match: &'static str,
    /// `thinking: {type: …}` 字段;None = 不发该字段(kimi-k3 形态)
    pub thinking_type: Option<ThinkingType>,
    /// 档位挂载点;None = 不发档位(GLM 老模型形态)
    pub effort_at: Option<EffortField>,
}

impl ThinkingRule {
    /// 两字段形态(thinking:enabled + output_config:{effort})
    pub const ENABLED_AND_EFFORT: Self = Self {
        model_match: "",
        thinking_type: Some(ThinkingType::Enabled),
        effort_at: Some(EffortField::OutputConfigEffort),
    };
}

/// 按规则表解析思考注入(键经 rig `#[serde(flatten)]` additional_params
/// 落 body 顶层;`None` = 表内无命中,不发)
pub fn thinking_params(rules: &[ThinkingRule], model: &str, effort: &str) -> Option<Value> {
    let m = model.to_ascii_lowercase();
    let rule = rules.iter().find(|r| m.contains(r.model_match))?;
    let mut o = serde_json::Map::new();
    if let Some(t) = rule.thinking_type {
        let t = match t {
            ThinkingType::Enabled => "enabled",
            ThinkingType::Adaptive => "adaptive",
        };
        o.insert("thinking".into(), json!({ "type": t }));
    }
    if rule.effort_at.is_some() {
        o.insert("output_config".into(), json!({ "effort": effort }));
    }
    (!o.is_empty()).then_some(Value::Object(o))
}

// ============================================================
// 模型能力标记
// ============================================================

/// 模型输入能力(粒度 = 模型)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelCaps {
    /// 图片输入(user 消息与工具结果)
    pub image: bool,
    /// 视频输入
    pub video: bool,
    /// PDF 输入
    pub pdf: bool,
}

impl ModelCaps {
    /// 全文本(未编目模型的安全缺省)
    pub const TEXT_ONLY: Self = Self {
        image: false,
        video: false,
        pdf: false,
    };
}

/// 一条模型能力规则:模型名(小写子串)命中 → 能力。
/// 首条命中即用(特例在前,通配在后);未命中 → 行为组缺省。
#[derive(Clone, Copy, Debug)]
pub struct ModelCap {
    /// 模型名小写子串(如 "glm-5.3-flash";特例条目须排在通配前)
    pub model_match: &'static str,
    /// 命中模型的能力
    pub caps: ModelCaps,
}

/// 按能力表解析(首条命中;未命中 → default)
pub fn resolve_caps(table: &[ModelCap], default: ModelCaps, model: &str) -> ModelCaps {
    let m = model.to_ascii_lowercase();
    table
        .iter()
        .find(|c| m.contains(c.model_match))
        .map(|c| c.caps)
        .unwrap_or(default)
}

// ============================================================
// hosted 工具(厂商服务端执行,如 web_search)
// ============================================================

/// 一条 hosted 工具声明:liuma 工具面的通用 kind 在该厂商/模型上的
/// wire 形态。
#[derive(Clone, Copy, Debug)]
pub struct HostedToolDecl {
    /// liuma 工具面条目 type 值(如 "web_search";装配层用)
    pub kind: &'static str,
    /// 模型名小写子串(空 = 全模型;如官方 claude 仅 claude 系)
    pub model_match: &'static str,
    /// wire 上的 `type` 值(anthropic 面为版本化串,如
    /// "web_search_20260209";responses 面为 "web_search")
    pub wire_type: &'static str,
    /// wire 上是否携带 `name` 字段(anthropic 面要,responses 面不要)
    pub wire_name: Option<&'static str>,
}

/// 解析 hosted 工具声明(工具条目 kind + 模型名 → 声明;
/// None = 该面/该模型不提供此工具,调用方 fail-fast)
pub fn resolve_hosted(table: &[HostedToolDecl], kind: &str, model: &str) -> Option<HostedToolDecl> {
    let m = model.to_ascii_lowercase();
    table
        .iter()
        .find(|d| d.kind == kind && (d.model_match.is_empty() || m.contains(d.model_match)))
        .copied()
}

// ============================================================
// 错误体形态与业务码
// ============================================================

/// 200 + 非 SSE 错误体的 JSON 形态(端点以 200 应答错误的坏习惯面)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorBodyShape {
    /// 嵌套形态 `{"error":{"code":…,"message":…}}`(OpenAI 兼容家族契约)
    Nested,
    /// 顶层形态 `{"code":…,"msg":…}`(GLM 200 实测形态)
    TopLevel,
    /// 顶层优先、嵌套兜底(GLM 官方契约两形态并收)
    TopLevelThenNested,
}

/// 业务码语义分类(官方错误码表的精炼面;解释器见
/// [`refine_by_codes`])
#[derive(Clone, Copy, Debug, Default)]
pub struct BusinessCodes {
    /// 上下文超长类(如 GLM 1261「Prompt 超长」)→ CONTEXT_OVERFLOW,
    /// engine 走强制压缩而非盲目重发
    pub overflow: &'static [u64],
    /// 订阅/账户致命类(欠费/套餐到期/无权限;如 GLM 1113/1309/1311/
    /// 1314/1315)→ 不可重试 Other——顶着 429 状态也不进限流重试
    pub fatal: &'static [u64],
}

/// provider 行为组:方言声明持有,可整体替换/逐项追加。
#[derive(Clone, Debug)]
pub struct ProviderBehaviors {
    /// 思考表达规则表(openai 系为空 = 方言级表达在 adapter 覆写)
    pub thinking_rules: Vec<ThinkingRule>,
    /// 200 + JSON 错误体形态
    pub error_shape: ErrorBodyShape,
    /// 业务码分类(空 = 无业务码精化)
    pub business_codes: BusinessCodes,
    /// 模型能力表(首条命中;未命中 → default_caps)
    pub model_caps: &'static [ModelCap],
    /// 未编目模型的能力缺省
    pub default_caps: ModelCaps,
    /// hosted 工具声明(按模型/面解析)
    pub hosted_tools: &'static [HostedToolDecl],
}

impl Default for ProviderBehaviors {
    fn default() -> Self {
        Self::openai_family()
    }
}

impl ProviderBehaviors {
    /// OpenAI 兼容家族默认:嵌套错误体、无业务码表、图片开(现代
    /// OpenAI 兼容端点普遍收 input_image,发错模型可见 400 而非误降级)
    pub fn openai_family() -> Self {
        Self {
            thinking_rules: Vec::new(),
            error_shape: ErrorBodyShape::Nested,
            business_codes: BusinessCodes::default(),
            model_caps: &[],
            default_caps: ModelCaps {
                image: true,
                video: false,
                pdf: false,
            },
            hosted_tools: &[],
        }
    }
}

// ------------------------------------------------------------
// 错误解释器
// ------------------------------------------------------------

/// 200 + 非 SSE 错误体归类(按形态;code 落 HTTP 范围 400-599 →
/// 按状态归类(401/403 → AUTH),平台自有码 → Other 带原始消息;
/// 识别不出错误结构 → None,交由引擎按空响应处理)
pub fn body_error_by_shape(shape: ErrorBodyShape, body: &str) -> Option<TransportError> {
    match shape {
        ErrorBodyShape::Nested => body_error_nested(body),
        ErrorBodyShape::TopLevel => body_error_top_level(body),
        ErrorBodyShape::TopLevelThenNested => {
            body_error_top_level(body).or_else(|| body_error_nested(body))
        }
    }
}

/// 非 2xx 错误体精化:业务码命中表 → 改判;未命中 → 原分类透传
pub fn refine_by_codes(codes: &BusinessCodes, err: TransportError, body: &str) -> TransportError {
    let Some(code) = business_code(body) else {
        return err;
    };
    if codes.overflow.contains(&code) {
        return TransportError::ContextOverflow {
            status: 400,
            body: body.to_string(),
        };
    }
    if codes.fatal.contains(&code) {
        return TransportError::Other(format!("provider 订阅/账户错误(业务码 {code}):{body}"));
    }
    err
}

/// 错误体的业务码(嵌套 error.code 与顶层 code 两形态,串数字皆收)
fn business_code(body: &str) -> Option<u64> {
    let v: Value = serde_json::from_str(body).ok()?;
    let raw = v["error"]["code"]
        .as_str()
        .or_else(|| v["code"].as_str())
        .map(String::from)
        .or_else(|| {
            v["error"]["code"]
                .as_u64()
                .or(v["code"].as_u64())
                .map(|c| c.to_string())
        })?;
    raw.parse().ok()
}

fn code_to_error(code: u64, message: &str, body: &str) -> TransportError {
    use crate::http::classify_status;
    let status = u16::try_from(code)
        .ok()
        .and_then(|c| reqwest::StatusCode::from_u16(c).ok())
        .filter(|s| (400..=599).contains(&s.as_u16()));
    match status {
        Some(status) => classify_status(status, None, body.to_string()),
        None => TransportError::Other(format!("provider 错误 {code}: {message}")),
    }
}

fn body_error_nested(body: &str) -> Option<TransportError> {
    let v: Value = serde_json::from_str(body).ok()?;
    let error = v["error"].as_object()?;
    let code = error["code"]
        .as_u64()
        .or_else(|| error["code"].as_str()?.parse::<u64>().ok())?;
    let message = error["message"].as_str().unwrap_or_default();
    Some(code_to_error(code, message, body))
}

fn body_error_top_level(body: &str) -> Option<TransportError> {
    let v: Value = serde_json::from_str(body).ok()?;
    let code = v["code"]
        .as_u64()
        .or_else(|| v["code"].as_str()?.parse::<u64>().ok())?;
    let msg = v["msg"]
        .as_str()
        .or(v["message"].as_str())
        .unwrap_or_default();
    Some(code_to_error(code, msg, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 能力解析:首条命中 / 未命中缺省(dsh 策略 text-only)
    #[test]
    fn caps_first_match_then_default() {
        let table = [
            ModelCap {
                model_match: "flash",
                caps: ModelCaps {
                    image: true,
                    video: false,
                    pdf: false,
                },
            },
            ModelCap {
                model_match: "pro",
                caps: ModelCaps::TEXT_ONLY,
            },
        ];
        let default = ModelCaps::TEXT_ONLY;
        let img = resolve_caps(&table, default, "deepseek-flash");
        assert!(img.image && !img.video);
        let text = resolve_caps(&table, default, "deepseek-v4-pro");
        assert!(!text.image);
        assert_eq!(resolve_caps(&table, default, "unknown-model"), default);
    }

    /// hosted 声明解析:kind + 模型门控(带门控条目对门内模型优先)
    #[test]
    fn hosted_resolution_by_kind_and_model() {
        let table = [
            HostedToolDecl {
                kind: "web_search",
                model_match: "",
                wire_type: "web_search_20260209",
                wire_name: Some("web_search"),
            },
            HostedToolDecl {
                kind: "web_search",
                model_match: "deepseek",
                wire_type: "web_search_20250305",
                wire_name: Some("web_search"),
            },
        ];
        // find 首条命中:通配在前会挡住 deepseek 条目——门控条目必须
        // 排在通配前(数据表的排序契约,见 providers/anthropic.rs)
        let table_sorted = [table[1], table[0]];
        let d = resolve_hosted(&table_sorted, "web_search", "deepseek-flash").unwrap();
        assert_eq!(d.wire_type, "web_search_20250305");
        let g = resolve_hosted(&table_sorted, "web_search", "glm-5.3").unwrap();
        assert_eq!(g.wire_type, "web_search_20260209");
        assert!(resolve_hosted(&table_sorted, "file_search", "glm-5.3").is_none());
    }

    /// 业务码精化:overflow/fatal/未命中三路(解释器行为锁;
    /// 数据表在 providers/glm.rs)
    #[test]
    fn refine_by_codes_three_paths() {
        let codes = BusinessCodes {
            overflow: &[1261],
            fatal: &[1309],
        };
        let overflow = refine_by_codes(
            &codes,
            TransportError::InvalidRequest {
                status: 400,
                body: String::new(),
            },
            r#"{"error":{"code":"1261","message":"Prompt 超长"}}"#,
        );
        assert!(overflow.is_context_overflow());
        let fatal = refine_by_codes(
            &codes,
            TransportError::RateLimit {
                retry_after_ms: None,
                body: String::new(),
            },
            r#"{"code":1309,"msg":"套餐已到期"}"#,
        );
        assert!(!fatal.retryable());
        let kept = refine_by_codes(
            &codes,
            TransportError::RateLimit {
                retry_after_ms: None,
                body: String::new(),
            },
            r#"{"code":1308,"msg":"配额窗口"}"#,
        );
        assert!(kept.retryable(), "未命中码原分类透传");
    }
}
