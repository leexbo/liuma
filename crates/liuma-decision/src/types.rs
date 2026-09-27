//! System One 协议类型:问题/答案的强类型形状。
//!
//! 与线上 JSON 一一对应(`serde(tag = "type")`,值 `noul`/`choice`/`score`)。
//! 两家 provider 的字段差异以 Option/`#[serde(default)]` 容忍:
//! 阿里百炼无 `usage.output_tokens`、附加 `request_id`;双方真实样例在
//! `tests/client_wire.rs` 回归。答案永远约束在请求给定的选项集内——
//! 类型安全 by construction,消费方按 id 手写 match 穷尽解包。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// 问题类型(协议 `type` 字段判别)。
///
/// `instructions` 可为字符串或结构化对象(问题一字段、数据其余字段,
/// 反引号路径引用 state,如 `` `ticket.messages[0].text` ``)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Question {
    /// 是/非判断(答案 = P(yes),无独立 confidence)
    Noul {
        /// 问题文本或结构化对象
        instructions: Value,
        /// 可选:yes / no 各自含义描述
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<NoulCriteria>,
    },
    /// 多选一(≤255 选项;答案 = 选项 + 全量分布 + confidence)
    Choice {
        /// 问题文本或结构化对象
        instructions: Value,
        /// 选项名 → 选项描述(建议含 `other` 兜底项)
        criteria: BTreeMap<String, String>,
    },
    /// 有序量表(2–255 级;答案 = 概率加权分,可落两级之间)
    Score {
        /// 问题文本或结构化对象
        instructions: Value,
        /// 从低到高的等级描述
        criteria: Vec<String>,
    },
}

/// noul 的 yes/no 含义描述(协议 `criteria: {"true":…, "false":…}`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoulCriteria {
    /// "是"(值接近 1)的含义
    #[serde(rename = "true")]
    pub yes: String,
    /// "否"(值接近 0)的含义
    #[serde(rename = "false")]
    pub no: String,
}

/// 答案类型(与问题 `type` 一致;Choice/Score 附带 confidence)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Answer {
    /// P(yes):0 = 否,1 = 是,0.5 = 不确定
    Noul {
        /// 是的概率
        noul: f64,
    },
    /// 选中的选项 + 全量分布 + confidence
    Choice {
        /// 最高概率选项
        choice: String,
        /// 每个选项的概率(和为 1)
        probabilities: BTreeMap<String, f64>,
        /// 分布形状导出的置信度(0..1)
        confidence: f64,
    },
    /// 加权分 + 等级表 + 分布 + confidence
    Score {
        /// 等级索引的概率加权期望(可落两级之间,如 1.43)
        score: f64,
        /// 等级索引(字符串数字)→ 等级描述
        legend: BTreeMap<String, String>,
        /// 每级概率(和为 1)
        probabilities: BTreeMap<String, f64>,
        /// 分布形状导出的置信度(0..1)
        confidence: f64,
    },
}

/// token 用量(百炼不返回 `output_tokens` → 缺席容忍)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Usage {
    /// 输入 token 数
    #[serde(default)]
    pub input_tokens: u64,
    /// 输出 token 数(TypeSafe 免费计量返回;百炼缺席)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}

/// 一次询问:状态 + 保序问题表。
///
/// 问题 id 不发给模型、不参与推理,仅用于答案配对(协议约定)。
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionRequest {
    /// 待评判的状态(字符串 / JSON 对象 / 数组,原样透传)
    pub state: Value,
    /// 问题表(id 保序;同请求内并行独立评估同一 state)
    pub questions: Vec<(String, Question)>,
    /// 模型名(请求 `model` 字段;装配配置注入,如 `jev-latest` /
    /// `decision-model-preview`)
    pub model: String,
}

/// 一次应答:答案按请求 id 一一对应返回。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionAnswers {
    /// 应答的模型(版本化 id;别名请求会回显解析结果)
    pub model: String,
    /// 答案表(键 = 请求问题的 id)
    pub answers: BTreeMap<String, Answer>,
    /// token 用量
    #[serde(default)]
    pub usage: Usage,
    /// provider 请求 id(百炼附加;官方无)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl DecisionRequest {
    /// 序列化为协议请求体(`questions` 以 map 形态发;id 顺序无协议意义)
    pub fn to_body(&self) -> Value {
        let questions: serde_json::Map<String, Value> = self
            .questions
            .iter()
            .map(|(id, q)| (id.clone(), serde_json::to_value(q).unwrap_or(Value::Null)))
            .collect();
        serde_json::json!({
            "state": self.state,
            "model": self.model,
            "questions": questions,
        })
    }

    /// state 摘要(sha256 前 16 hex):审计落档用,原文不进日志
    pub fn state_digest(&self) -> String {
        let canonical = if self.state.is_string() {
            self.state.as_str().unwrap_or_default().as_bytes().to_vec()
        } else {
            serde_json::to_vec(&self.state).unwrap_or_default()
        };
        let digest = Sha256::digest(&canonical);
        hex::encode(&digest[..8])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// TypeSafe 官方文档样例:choice + score + noul 混用请求体的逐字对齐
    #[test]
    fn request_body_matches_protocol_shape() {
        let mut criteria = BTreeMap::new();
        criteria.insert(
            "billing".to_string(),
            "Payments, invoicing, refunds".to_string(),
        );
        criteria.insert(
            "technical".to_string(),
            "Bugs, outages, integrations".to_string(),
        );
        let req = DecisionRequest {
            state: json!("Help! My payouts have been failing for 3 days."),
            questions: vec![
                (
                    "is_urgent".to_string(),
                    Question::Noul {
                        instructions: json!("Does this convey urgency?"),
                        criteria: Some(NoulCriteria {
                            yes: "Explicitly time-sensitive".into(),
                            no: "No urgency expressed".into(),
                        }),
                    },
                ),
                (
                    "department".to_string(),
                    Question::Choice {
                        instructions: json!("Which team should handle this?"),
                        criteria,
                    },
                ),
                (
                    "frustration".to_string(),
                    Question::Score {
                        instructions: json!("How frustrated is the customer?"),
                        criteria: vec!["Calm".into(), "Frustrated".into(), "Very angry".into()],
                    },
                ),
            ],
            model: "jev-latest".into(),
        };
        let body = req.to_body();
        assert_eq!(body["model"], "jev-latest");
        assert_eq!(body["questions"]["is_urgent"]["type"], "noul");
        assert_eq!(
            body["questions"]["is_urgent"]["criteria"]["true"],
            "Explicitly time-sensitive"
        );
        assert_eq!(body["questions"]["department"]["type"], "choice");
        assert_eq!(
            body["questions"]["department"]["criteria"]["billing"],
            "Payments, invoicing, refunds"
        );
        assert_eq!(
            body["questions"]["frustration"]["criteria"][2],
            "Very angry"
        );
    }

    /// TypeSafe 官方响应样例:三型答案解码 + noul 无 confidence 的容忍
    #[test]
    fn official_response_sample_decodes() {
        let raw = json!({
            "model": "jev-1.13.0",
            "answers": {
                "is_urgent": { "type": "noul", "noul": 0.95 },
                "department": {
                    "type": "choice", "choice": "billing", "confidence": 0.81,
                    "probabilities": { "billing": 0.88, "technical": 0.12, "sales": 0.0 }
                },
                "frustration": {
                    "type": "score", "score": 1.05, "confidence": 0.92,
                    "legend": { "0": "Calm", "1": "Frustrated", "2": "Very angry" },
                    "probabilities": { "0": 0.0, "1": 0.95, "2": 0.05 }
                }
            },
            "usage": { "input_tokens": 318, "output_tokens": 34 }
        });
        let answers: DecisionAnswers = serde_json::from_value(raw).expect("decode");
        assert_eq!(answers.model, "jev-1.13.0");
        assert_eq!(answers.usage.output_tokens, Some(34));
        let Answer::Noul { noul } = answers.answers["is_urgent"] else {
            panic!("noul 变体");
        };
        assert!((noul - 0.95).abs() < f64::EPSILON);
        let Answer::Choice {
            choice, confidence, ..
        } = &answers.answers["department"]
        else {
            panic!("choice 变体");
        };
        assert_eq!(choice, "billing");
        assert!((confidence - 0.81).abs() < f64::EPSILON);
        let Answer::Score { score, legend, .. } = &answers.answers["frustration"] else {
            panic!("score 变体");
        };
        assert!((score - 1.05).abs() < f64::EPSILON);
        assert_eq!(legend["1"], "Frustrated");
    }

    /// 阿里百炼响应样例:usage 无 output_tokens、附加 request_id/latency_ms
    /// (未登记字段容忍)、score 落两级之间
    #[test]
    fn aliyun_response_sample_decodes() {
        let raw = json!({
            "model": "decision-model-preview",
            "request_id": "7b986c65-b223-9341-b5f0-b988e27ecaac",
            "answers": {
                "department": {
                    "type": "choice", "choice": "billing", "confidence": 0.88,
                    "probabilities": { "billing": 0.94, "technical": 0.06 }
                },
                "severity": {
                    "type": "score", "score": 2.25, "confidence": 0.91,
                    "legend": { "0": "轻微", "1": "部分", "2": "核心", "3": "严重" },
                    "probabilities": { "0": 0.0, "1": 0.01, "2": 0.73, "3": 0.26 }
                }
            },
            "usage": { "input_tokens": 125 },
            "latency_ms": 52.9
        });
        let answers: DecisionAnswers = serde_json::from_value(raw).expect("decode");
        assert_eq!(answers.usage.input_tokens, 125);
        assert_eq!(answers.usage.output_tokens, None, "百炼无 output_tokens");
        assert_eq!(
            answers.request_id.as_deref(),
            Some("7b986c65-b223-9341-b5f0-b988e27ecaac")
        );
        let Answer::Score { score, .. } = answers.answers["severity"] else {
            panic!("score 变体");
        };
        assert!((score - 2.25).abs() < f64::EPSILON);
    }

    /// state 摘要:字符串与 JSON 对象两形态均可稳定摘要
    #[test]
    fn state_digest_stable_across_shapes() {
        let text = DecisionRequest {
            state: json!("hello"),
            questions: vec![],
            model: "m".into(),
        };
        let object = DecisionRequest {
            state: json!({ "a": 1 }),
            questions: vec![],
            model: "m".into(),
        };
        assert_eq!(text.state_digest().len(), 16);
        assert_eq!(text.state_digest(), text.state_digest(), "同状态同摘要");
        assert_ne!(text.state_digest(), object.state_digest(), "异状态异摘要");
    }
}
