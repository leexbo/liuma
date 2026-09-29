//! 问答/计划审批的 store 域:UI 交互态(AskUiState)+ 应答/取消/分页/
//! 提交/自定义输入。pending_ask/pending_plan 是 StoreState 的会话镜像
//! (reducer 侧),本域应答后经 host.respond 回填。

use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::questionnaire::{
    QuestionnaireChoiceDefinition, QuestionnaireEvent, QuestionnaireInputDefinition,
    QuestionnaireItemDefinition, QuestionnaireItemStatus, QuestionnaireState,
    QuestionnaireSubmission,
};
use gpui_kit::{AppContext, Context, Entity, Window};

use liuma_core::proto::RpcResult;

use crate::kits::i18n::t;
use crate::shell::store::AppStore;

/// 库 `QuestionnaireState` 与本仓 wire 形状之间的反查表。
///
/// 库内 item `name` 与 choice `value` 一律用**下标字符串**:下标天然唯一,
/// schema 校验(重名/重值)无从触发,也就不必在渲染路径上处理
/// `QuestionnaireSchemaError`。标签另存于此,提交时按下标反查回 wire。
#[derive(Debug, Clone)]
pub(crate) struct AskWire {
    /// 每题 wire id(与问题序对齐)
    pub ids: Vec<String>,
    /// 每题选项标签(与选项序对齐)
    pub labels: Vec<Vec<String>>,
}

/// 问答/计划审批功能切片状态(问答卡交互态 + 「其他」输入)。
#[derive(Default)]
pub(crate) struct AskStore {
    /// 问答卡 = 库 `QuestionnaireState`(选项/分页/校验/键盘全归库;
    /// 挂窗后经 ensure_ask_questionnaire 懒建)
    pub ask_questionnaire: Option<Entity<QuestionnaireState>>,
    /// 建卡身份(rpcId):换卡即重建,同卡不重建
    pub ask_questionnaire_key: Option<String>,
    /// 库状态 → wire 的反查表(与 ask_questionnaire 同生命周期)
    pub ask_wire: Option<AskWire>,
    /// 审批卡选项②「否,并告诉它应该如何做不同」的行内输入(直接在
    /// 卡内输入,非拒绝后聚焦 composer;ensure 懒建同上)
    pub plan_decline_input: Option<Entity<TextareaState>>,
    /// 审批卡选项选择态(选择→批准两步;None=未选,
    /// Some(true)=①实施 / Some(false)=②修改意见)。提交后随 pending 清空
    pub plan_selection: Option<bool>,
}

impl AppStore {
    /// 计划审批应答(approve = 批准标签;应答形状见 registry respond)
    pub fn answer_plan(&mut self, approve: bool, cx: &mut Context<Self>) {
        let Some(plan) = self.state.pending_plan.take() else {
            return;
        };
        self.ask.plan_decline_input = None;
        self.ask.plan_selection = None;
        // 应答 label 优先用发起方 options(label 随答案回给模型)
        let opts = plan.question.options.clone().unwrap_or_default();
        let label = if approve {
            opts.first()
                .map(|o| o.label.clone())
                .unwrap_or_else(|| t!("ask.approve").into())
        } else {
            opts.get(1)
                .map(|o| o.label.clone())
                .unwrap_or_else(|| t!("ask.reject").into())
        };
        let result = RpcResult::Ok(serde_json::json!({
            "sessionId": plan.session_id,
            "answer": { "answers": [ { "id": plan.question.id, "selected": [label] } ] },
        }));
        self.bridge.host().respond(&plan.rpc_id, &result);
        cx.notify();
    }

    /// 去聊天里说(计划卡第三动作):取消请求 → 工具收到
    /// 取消结果,模型回到对话;composer 恢复,用户直接说修改意见
    pub fn dismiss_plan(&mut self, cx: &mut Context<Self>) {
        let Some(plan) = self.state.pending_plan.take() else {
            return;
        };
        self.ask.plan_decline_input = None;
        self.ask.plan_selection = None;
        self.bridge.host().respond(
            &plan.rpc_id,
            &RpcResult::Err(liuma_core::proto::RpcError {
                code: "cancelled".into(),
                message: t!("ask.user_cancelled").into(),
                details: serde_json::Value::Null,
            }),
        );
        cx.notify();
    }

    /// 选项②「否,并告诉它应该如何做不同」提交
    /// (卡内行内输入,Enter 提交):拒绝应答 + custom 反馈,宿主经引导轮
    /// 直送模型(见 registry plan_decline_guide)
    pub fn decline_plan_with_feedback(&mut self, feedback: String, cx: &mut Context<Self>) {
        let Some(plan) = self.state.pending_plan.take() else {
            return;
        };
        self.ask.plan_decline_input = None;
        let opts = plan.question.options.clone().unwrap_or_default();
        let label = opts
            .get(1)
            .map(|o| o.label.clone())
            .unwrap_or_else(|| t!("ask.reject").into());
        let result = RpcResult::Ok(serde_json::json!({
            "sessionId": plan.session_id,
            "answer": { "answers": [ {
                "id": plan.question.id,
                "selected": [label],
                "custom": feedback,
            } ] },
        }));
        self.bridge.host().respond(&plan.rpc_id, &result);
        cx.notify();
    }

    /// 审批卡选项选择(选择→批准两步——点选项行
    /// 只标记选择,不提交;提交走「批准」钮)
    pub fn select_plan_option(&mut self, approve: bool, cx: &mut Context<Self>) {
        self.ask.plan_selection = Some(approve);
        cx.notify();
    }

    /// 提交所选选项:① → 批准;② → 有反馈 = 拒绝+反馈,空反馈 =
    /// 仅拒绝(原「跳过」语义并入)。未选择时为 no-op
    pub fn submit_plan_selection(&mut self, cx: &mut Context<Self>) {
        let Some(selected) = self.ask.plan_selection else {
            return;
        };
        if selected {
            self.answer_plan(true, cx);
            return;
        }
        let feedback = self
            .ask
            .plan_decline_input
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
            .unwrap_or_default();
        if feedback.is_empty() {
            self.answer_plan(false, cx);
        } else {
            self.decline_plan_with_feedback(feedback, cx);
        }
    }
    /// 懒建审批卡选项②行内输入(需要 Window;render 期首现调用——
    /// 仅选项②选中时渲染)。Enter(非 shift)= 按当前选择提交
    pub fn ensure_plan_decline_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ask.plan_decline_input.is_some() {
            return;
        }
        let input = cx.new(|cx| TextareaState::new(window, cx).placeholder(t!("ask.decline_ph")));
        cx.subscribe(&input, |this, _input, event: &InputEvent, cx| match event {
            InputEvent::PressEnter { shift: false, .. } => {
                this.submit_plan_selection(cx);
            }
            InputEvent::Change => cx.notify(),
            _ => {}
        })
        .detach();
        self.ask.plan_decline_input = Some(input);
    }

    /// 通用问答应答(answers 数组;每项 {id, selected[], custom?})。
    /// 经 host.respond 回填,工具结果作为同一 tool-call 的 tool/result。
    pub fn answer_ask(&mut self, answers: Vec<serde_json::Value>, cx: &mut Context<Self>) {
        let Some(ask) = self.state.pending_ask.take() else {
            return;
        };
        let result = RpcResult::Ok(serde_json::json!({
            "sessionId": ask.session_id,
            "answer": { "answers": answers },
        }));
        self.bridge.host().respond(&ask.rpc_id, &result);
        cx.notify();
    }

    /// 沙箱升级审批(allow-once / rejected;应答形状见 registry respond
    /// Approval 分支 value["answer"]["approved"])
    pub fn answer_approval(&mut self, approve: bool, cx: &mut Context<Self>) {
        let Some(p) = self.state.pending_approval.take() else {
            return;
        };
        let result = RpcResult::Ok(serde_json::json!({
            "sessionId": p.session_id,
            "answer": { "approved": approve },
        }));
        self.bridge.host().respond(&p.rpc_id, &result);
        cx.notify();
    }

    /// 取消审批(✕ → cancelled;模型收到逐字取消文案,审计对收口)
    pub fn dismiss_approval(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.state.pending_approval.take() else {
            return;
        };
        self.bridge.host().respond(
            &p.rpc_id,
            &RpcResult::Err(liuma_core::proto::RpcError {
                code: "cancelled".into(),
                message: t!("ask.user_cancelled").into(),
                details: serde_json::Value::Null,
            }),
        );
        cx.notify();
    }

    /// 放弃整组问题(取消;respond ok:false → host reject)
    pub fn cancel_ask(&mut self, cx: &mut Context<Self>) {
        let Some(ask) = self.state.pending_ask.take() else {
            return;
        };
        self.bridge.host().respond(
            &ask.rpc_id,
            &RpcResult::Err(liuma_core::proto::RpcError {
                code: "cancelled".into(),
                message: "user cancelled the question".into(),
                details: serde_json::Value::Null,
            }),
        );
        cx.notify();
    }

    // ── 问答卡交互(全归库 `QuestionnaireState`)──────────────────

    /// 懒建问答卡状态(需要 Window;render 期首现调用)。题目集 → 库的
    /// item 定义(选项 → choices、「其他」→ input),并订阅库的提交事件
    /// 把 `QuestionnaireSubmission` 翻回本仓 wire 形状。
    ///
    /// 库的状态机自带本卡原先手写的全部语义:空答案阻塞提交(未答 →
    /// `Unanswered`,与「未答不得提交」同义)、`skip_current` 是唯一的
    /// 放行口(与「跳过才放行」同义)、单选 choice 与自由文本互斥
    /// (`activate_choice` 清自由文本、输入清 choice,与「选项/自定义互斥」
    /// 同义)、末题 `confirm_current` 即提交。
    pub fn ensure_ask_questionnaire(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ask) = self.state.pending_ask.clone() else {
            self.ask.ask_questionnaire = None;
            self.ask.ask_questionnaire_key = None;
            self.ask.ask_wire = None;
            return;
        };
        if self.ask.ask_questionnaire_key.as_deref() == Some(ask.rpc_id.as_str()) {
            return;
        }
        let mut ids = Vec::with_capacity(ask.questions.len());
        let mut labels = Vec::with_capacity(ask.questions.len());
        let mut items = Vec::with_capacity(ask.questions.len());
        for (i, q) in ask.questions.iter().enumerate() {
            let options = q.options.clone().unwrap_or_default();
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("ask.answer_ph")));
            let choices = options
                .iter()
                .enumerate()
                .map(|(oi, o)| {
                    let def = QuestionnaireChoiceDefinition::new(oi.to_string(), o.label.clone());
                    match o.description.clone() {
                        Some(d) => def.with_description(d),
                        None => def,
                    }
                })
                .collect::<Vec<_>>();
            items.push(
                QuestionnaireItemDefinition::new(i.to_string(), q.question.clone())
                    .with_multiple(q.multi_select.unwrap_or(false))
                    .with_choices(choices)
                    .with_input(QuestionnaireInputDefinition::new(
                        input,
                        t!("ask.other_option"),
                    )),
            );
            ids.push(q.id.clone());
            labels.push(options.iter().map(|o| o.label.clone()).collect());
        }
        // 构造不可能失败:item name 与 choice value 都是下标字符串(唯一),
        // 且不声明任何 default_selected —— `validate_schema` 的三条判据
        //(重名 item / 重值 choice / 单选多项默认)全无从成立。
        let state = cx.new(|cx| {
            QuestionnaireState::new(items, cx).expect("下标命名的 questionnaire schema 不会冲突")
        });
        cx.subscribe(&state, |this, _state, event: &QuestionnaireEvent, cx| {
            if let QuestionnaireEvent::Submit(submission) = event {
                this.submit_ask_submission(submission, cx);
            }
        })
        .detach();
        self.ask.ask_questionnaire = Some(state);
        self.ask.ask_questionnaire_key = Some(ask.rpc_id.clone());
        self.ask.ask_wire = Some(AskWire { ids, labels });
    }

    /// 库提交事件 → wire `{id, selected[], custom?}` 数组 + 收卡应答。
    /// 跳过题的提交形状 = 空 selected(wire 无 skipped 标记),与库清空
    /// 答案后的 `Skipped` 状态一致。
    fn submit_ask_submission(
        &mut self,
        submission: &QuestionnaireSubmission,
        cx: &mut Context<Self>,
    ) {
        let Some(wire) = self.ask.ask_wire.take() else {
            return;
        };
        self.ask.ask_questionnaire = None;
        self.ask.ask_questionnaire_key = None;
        let items: Vec<AskSubmittedItem> = submission
            .items()
            .iter()
            .map(|item| AskSubmittedItem {
                name: item.name().to_string(),
                skipped: item.status() == QuestionnaireItemStatus::Skipped,
                choices: item
                    .answer()
                    .choices()
                    .iter()
                    .map(|value| value.to_string())
                    .collect(),
                freeform: item.answer().freeform().map(|text| text.to_string()),
            })
            .collect();
        self.answer_ask(answers_from_submission(&items, &wire), cx);
    }
}

/// 提交快照的一题(自库 `QuestionnaireSubmissionItem` 摘出的纯数据,
/// 好让「库快照 → wire」这段纯映射可被单测直接锁住)
#[derive(Debug, Clone)]
pub(crate) struct AskSubmittedItem {
    /// 库内 item 名(= 题序字符串)
    pub name: String,
    pub skipped: bool,
    /// 库内 choice value(= 选项序字符串)
    pub choices: Vec<String>,
    pub freeform: Option<String>,
}

/// 库快照 → wire `{id, selected[], custom?}` 数组。
///
/// 两处反查是这段的全部风险:库内 item/choice 一律是**下标字符串**,
/// 而 wire 要的是 wire id 与 option **标签**;反查失手会静默产出
/// `["0"]` 这类"形状合法、内容错"的答案,模型侧无从察觉,故单测钉死。
/// 越界(下标对不上问题集)一律跳过该题,不猜不补。
pub(crate) fn answers_from_submission(
    items: &[AskSubmittedItem],
    wire: &AskWire,
) -> Vec<serde_json::Value> {
    let mut answers: Vec<serde_json::Value> = Vec::with_capacity(items.len());
    for item in items {
        let Ok(ix) = item.name.parse::<usize>() else {
            continue;
        };
        let (Some(id), Some(option_labels)) = (wire.ids.get(ix), wire.labels.get(ix)) else {
            continue;
        };
        let mut entry = serde_json::Map::new();
        entry.insert("id".into(), serde_json::json!(id));
        let selected: Vec<String> = if item.skipped {
            Vec::new()
        } else {
            item.choices
                .iter()
                .filter_map(|value| value.parse::<usize>().ok())
                .filter_map(|oi| option_labels.get(oi).cloned())
                .collect()
        };
        entry.insert("selected".into(), serde_json::json!(selected));
        // 跳过题连 custom 一并丢掉:库的 skip 本就会清空答案,这里再显式
        // 收一道——带 custom 的跳过等于静默代答
        if !item.skipped
            && let Some(custom) = &item.freeform
        {
            entry.insert("custom".into(), serde_json::json!(custom));
        }
        answers.push(serde_json::Value::Object(entry));
    }
    answers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire() -> AskWire {
        AskWire {
            ids: vec!["switch_mode".into(), "q2".into()],
            labels: vec![
                vec!["默认走百炼".into(), "彻底替换".into()],
                vec!["b1".into()],
            ],
        }
    }

    /// 库内 choice 是**下标**,wire 要的是**标签**:反查失手会产出
    /// `["1"]` 这种形状合法、内容错的答案(模型侧无从察觉),故钉死。
    #[test]
    fn submission_maps_choice_indices_back_to_labels() {
        let items = vec![AskSubmittedItem {
            name: "0".into(),
            skipped: false,
            choices: vec!["1".into()],
            freeform: None,
        }];
        let answers = answers_from_submission(&items, &wire());
        assert_eq!(
            answers,
            vec![serde_json::json!({
                "id": "switch_mode",
                "selected": ["彻底替换"],
            })],
            "选项下标必须反查回标签"
        );
    }

    /// 跳过题:wire 无 skipped 标记,形状 = 空 selected 且不带 custom
    /// (库的 skip 同时清空选项与自由文本,这里再显式钉一次)。
    #[test]
    fn skipped_item_submits_empty_selection() {
        let items = vec![AskSubmittedItem {
            name: "1".into(),
            skipped: true,
            choices: vec!["0".into()],
            freeform: Some("残留草稿".into()),
        }];
        let answers = answers_from_submission(&items, &wire());
        let entry = answers[0].as_object().expect("应为对象");
        assert_eq!(entry["id"], serde_json::json!("q2"));
        assert_eq!(entry["selected"], serde_json::json!([] as [String; 0]));
        assert!(
            !entry.contains_key("custom"),
            "跳过题不得携带 custom(否则等于静默代答)"
        );
    }

    /// 「其他」自由文本随答案回传;空文本(库侧只会给 None)不入 wire。
    #[test]
    fn freeform_travels_with_the_answer() {
        let items = vec![AskSubmittedItem {
            name: "0".into(),
            skipped: false,
            choices: vec![],
            freeform: Some("都换成百炼".into()),
        }];
        let answers = answers_from_submission(&items, &wire());
        assert_eq!(answers[0]["selected"], serde_json::json!([] as [String; 0]));
        assert_eq!(answers[0]["custom"], serde_json::json!("都换成百炼"));
    }

    /// 下标对不上问题集(库多给/乱给)时跳过该题,不猜不补。
    #[test]
    fn out_of_range_item_is_dropped() {
        let items = vec![
            AskSubmittedItem {
                name: "9".into(),
                skipped: false,
                choices: vec![],
                freeform: None,
            },
            AskSubmittedItem {
                name: "not-a-number".into(),
                skipped: false,
                choices: vec![],
                freeform: None,
            },
        ];
        assert!(answers_from_submission(&items, &wire()).is_empty());
    }
}
