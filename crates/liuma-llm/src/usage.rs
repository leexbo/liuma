//! 用量归一:rig 统一 `Usage`(7 键全 Option)→ 内部规范五键。
//!
//! [`liuma_agent_loop::LlmEvent::Usage`] 载荷契约 = 规范形:
//! `input_tokens` / `output_tokens` / `cached_tokens`(缓存读)/
//! `cache_write_tokens`(缓存写)/ `reasoning_tokens`。rig 的归一层
//! (0.43「one meaning for Usage on every provider」)已按家族方言把
//! 官方键形归一进 7 键;本层只做键名平移与裁剪(cached_input_tokens →
//! cached_tokens、cache_creation_input_tokens → cache_write_tokens;
//! total_tokens 可由 input+output 推导、tool_use_prompt_tokens 为
//! hosted-tools 专用,均不进规范形)。缺席指标不产键(不产 null 帧)。

use rig_core::completion::Usage as RigUsage;
use serde_json::{Value, json};

/// rig Usage → liuma 规范五键(缺席键不产)
pub fn canonical_from_rig(u: &RigUsage) -> Value {
    let mut o = serde_json::Map::new();
    for (k, v) in [
        ("input_tokens", u.input_tokens),
        ("output_tokens", u.output_tokens),
        ("cached_tokens", u.cached_input_tokens),
        ("cache_write_tokens", u.cache_creation_input_tokens),
        ("reasoning_tokens", u.reasoning_tokens),
    ] {
        if let Some(v) = v {
            o.insert(k.into(), json!(v));
        }
    }
    Value::Object(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全键平移(GLM 真机流的键形;真实流终裁夹具同源)
    #[test]
    fn full_key_translation() {
        let u = RigUsage {
            input_tokens: Some(22),
            output_tokens: Some(68),
            total_tokens: Some(90),
            cached_input_tokens: Some(0),
            cache_creation_input_tokens: Some(4),
            tool_use_prompt_tokens: Some(9),
            reasoning_tokens: Some(60),
        };
        assert_eq!(
            canonical_from_rig(&u),
            json!({
                "input_tokens": 22,
                "output_tokens": 68,
                "cached_tokens": 0,
                "cache_write_tokens": 4,
                "reasoning_tokens": 60,
            })
        );
    }

    /// 全缺席 → 空对象(不产 null;引擎侧以 is_reported 先行判空)
    #[test]
    fn absent_keys_yield_empty_object() {
        assert_eq!(canonical_from_rig(&RigUsage::default()), json!({}));
    }
}
