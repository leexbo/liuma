//! 审批评审员:工具审批升级前给人类的风险标注(advisory)。
//!
//! 契约:**fail-open**——任何 Err/超时/disabled 返回 `None`,审批照常
//! 弹卡;评审员只产出建议,裁决权始终在人。挂接点在 approval=never
//! 短路**之后**(never 零调用零延迟,语义不可被评审绕过)。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::json;

use crate::types::{Answer, DecisionRequest, NoulCriteria, Question};

/// 评审输入(宿主审批面转译;与 liuma-tools EscalationRequest 解耦,
/// 本 crate 不依赖工具层)
#[derive(Debug, Clone)]
pub struct ReviewInput {
    /// 发起工具名(bash / …)
    pub tool_name: String,
    /// 命令/参数摘要(守卫面为 argsSummary)
    pub command: String,
    /// 目标沙箱模式(工具级审批无升级语义时为空)
    pub target_mode: String,
    /// 审批事由
    pub justification: String,
}

/// 风险标注(审批卡渲染;level 为"低风险概率",label 为三档文案)
#[derive(Debug, Clone, PartialEq)]
pub struct RiskAnnotation {
    /// noul 值(越接近 1 越像常规低风险操作)
    pub level: f64,
    /// 三档标签:low-risk / uncertain / risky
    pub label: &'static str,
    /// 一句话理由(模型面判断依据,人读)
    pub detail: String,
}

/// 审批评审员(宿主侧可选持有;None = 功能关)
pub trait ApprovalReviewer: Send + Sync {
    /// 评审一次升级请求;`None` = 无标注(fail-open,含一切失败)
    fn review(
        &self,
        input: &ReviewInput,
    ) -> Pin<Box<dyn Future<Output = Option<RiskAnnotation>> + Send + '_>>;
}

/// System One 后端的评审员实现
pub struct JevApprovalReviewer {
    port: Arc<dyn crate::DecisionPort>,
    model: String,
}

impl JevApprovalReviewer {
    /// 构造(端口与模型名来自决策模型装配)
    pub fn new(port: Arc<dyn crate::DecisionPort>, model: String) -> Self {
        Self { port, model }
    }
}

impl ApprovalReviewer for JevApprovalReviewer {
    fn review(
        &self,
        input: &ReviewInput,
    ) -> Pin<Box<dyn Future<Output = Option<RiskAnnotation>> + Send + '_>> {
        // input 小载荷先克隆(future 只绑 &self,签名单生命周期)
        let input = ReviewInput {
            tool_name: input.tool_name.clone(),
            command: input.command.clone(),
            target_mode: input.target_mode.clone(),
            justification: input.justification.clone(),
        };
        Box::pin(async move {
            let mut request = DecisionRequest {
                state: json!({
                    "tool": input.tool_name,
                    "command": input.command,
                    "target_mode": input.target_mode,
                    "justification": input.justification,
                }),
                questions: vec![(
                    "is_low_risk".to_string(),
                    Question::Noul {
                        instructions: json!(crate::thresholds::APPROVAL_LOW_RISK_QUESTION),
                        criteria: Some(NoulCriteria {
                            yes: crate::thresholds::APPROVAL_YES_MEANING.into(),
                            no: crate::thresholds::APPROVAL_NO_MEANING.into(),
                        }),
                    },
                )],
                model: self.model.clone(),
            };
            request.state =
                crate::client::fit_state(request.state, crate::thresholds::STATE_MAX_CHARS);
            let answers = self.port.ask(request).await.ok()?;
            let Answer::Noul { noul } = answers.answers.get("is_low_risk")? else {
                return None;
            };
            if !noul.is_finite() {
                return None;
            }
            let label = if *noul >= crate::thresholds::APPROVAL_LOW_RISK_PROBABILITY {
                "low-risk"
            } else if *noul <= crate::thresholds::APPROVAL_HIGH_RISK_PROBABILITY {
                "risky"
            } else {
                "uncertain"
            };
            // 理由随档位走(旧实现把 "low-risk" 写死进模板:判为 risky 时
            // 同一行里「有风险」与「judged low-risk」自相矛盾)。审计面用
            // 英文自足单句;卡面由桌面按档位就地组句(各自母语)
            let detail = match label {
                "low-risk" => format!(
                    "the model reads this {} call as routine and reversible (low-risk probability {noul:.2})",
                    input.tool_name
                ),
                "risky" => format!(
                    "the model reads this {} call as destructive, irreversible, or beyond the workspace (low-risk probability {noul:.2})",
                    input.tool_name
                ),
                _ => format!(
                    "the model is not confident about this {} call (low-risk probability {noul:.2})",
                    input.tool_name
                ),
            };
            Some(RiskAnnotation {
                level: *noul,
                label,
                detail,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::DecisionError;
    use crate::fake::FakeDecisionPort;
    use crate::types::DecisionAnswers;

    fn input() -> ReviewInput {
        ReviewInput {
            tool_name: "bash".into(),
            command: "rm -rf /tmp/build".into(),
            target_mode: "workspace-write".into(),
            justification: "clean build artifacts".into(),
        }
    }

    /// 高 noul → low-risk;低 noul → risky;中段 → uncertain
    #[tokio::test]
    async fn bands_map_to_labels() {
        for (noul, label) in [(0.95, "low-risk"), (0.1, "risky"), (0.5, "uncertain")] {
            let port = Arc::new(FakeDecisionPort::scripted(vec![Ok(
                FakeDecisionPort::noul_answers("is_low_risk", noul),
            )]));
            let reviewer = JevApprovalReviewer::new(port, "m".into());
            let note = reviewer.review(&input()).await.expect("有标注");
            assert_eq!(note.label, label, "noul={noul}");
            assert!((note.level - noul).abs() < f64::EPSILON);
        }
    }

    /// fail-open 契约:端口任何错误 → None(审批照常弹卡)
    #[tokio::test]
    async fn errors_fail_open_to_none() {
        let err = DecisionError::Timeout;
        let port = Arc::new(FakeDecisionPort::failing(err));
        let reviewer = JevApprovalReviewer::new(port, "m".into());
        assert!(reviewer.review(&input()).await.is_none());
    }

    /// 答案类型不符(应 noul 得 choice)→ None,不 panic
    #[tokio::test]
    async fn wrong_answer_shape_fails_open() {
        let mut answers = std::collections::BTreeMap::new();
        answers.insert(
            "is_low_risk".to_string(),
            Answer::Choice {
                choice: "x".into(),
                probabilities: std::collections::BTreeMap::new(),
                confidence: 0.5,
            },
        );
        let port = Arc::new(FakeDecisionPort::scripted(vec![Ok(DecisionAnswers {
            model: "m".into(),
            answers,
            usage: Default::default(),
            request_id: None,
        })]));
        let reviewer = JevApprovalReviewer::new(port, "m".into());
        assert!(reviewer.review(&input()).await.is_none());
    }
}
