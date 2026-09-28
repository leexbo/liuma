//! 问答卡:
//! 模型 `ask_user_question` 抛出的问题集,分页一题一答(单选/多选),
//! 可自定义文本;提交(整批)或放弃(取消)。应答经 host.respond 回填,
//! 工具结果作为同一 tool-call 的 tool/result。
//!
//! 选项、分页、校验与键盘全部归库 `Questionnaire`:状态机与 wire 形状的
//! 互译见 [`crate::features::ask::store::AppStore::ensure_ask_questionnaire`],
//! 本文件只负责卡壳与卡头(eyebrow + 取消)。库按当前题自动收放动作钮
//! (`navigation_state`:首题无「上一题」、末题才出「提交」、非必答恒出
//! 「跳过」),故四处动作件一律挂上,由库决定可见性。

use gpui_kit::component::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::StyledExt;
use gpui_kit::component::questionnaire::{
    Questionnaire, QuestionnaireActions, QuestionnaireChoice, QuestionnaireChoices,
    QuestionnaireError, QuestionnaireInput, QuestionnaireItem, QuestionnaireNext,
    QuestionnairePrevious, QuestionnaireProgress, QuestionnaireSkip, QuestionnaireSubmit,
    QuestionnaireTitle, QuestionnaireValidationError,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Entity, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    StyleRefinement, Styled, Window, div, px,
};

use crate::kits::i18n::dict;
use crate::kits::icons::fixed;
use crate::kits::theme;
use crate::shell::store::AppStore;

/// 问答卡整体(无 pending_ask → None;挂在消息列 children 链)。
pub fn render(
    store: &Entity<AppStore>,
    window: &mut Window,
    cx: &mut App,
) -> Option<impl IntoElement> {
    let ask = store.read(cx).state.pending_ask.clone()?;
    let current = store.read(cx).state.current_id.clone()?;
    if ask.session_id != current {
        eprintln!(
            "[q] 问答卡跳过:帧 session={} != 当前 {}",
            ask.session_id, current
        );
        return None;
    }
    // 库状态懒建(需要 Window):题集 → item 定义 + 提交订阅
    store.update(cx, |st, cx| st.ensure_ask_questionnaire(window, cx));
    let state = store.read(cx).ask.ask_questionnaire.clone()?;
    let total = ask.questions.len();
    let index = state
        .read(cx)
        .current_ix()
        .unwrap_or(0)
        .min(total.saturating_sub(1));
    let name = index.to_string();
    let eyebrow = ask
        .questions
        .get(index)
        .and_then(|q| q.header.clone())
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty());
    let question_text = ask
        .questions
        .get(index)
        .map(|q| q.question.clone())
        .unwrap_or_default();
    let option_count = ask
        .questions
        .get(index)
        .and_then(|q| q.options.as_ref().map(Vec::len))
        .unwrap_or(0);
    let cancel = store.clone();
    // 库只报「为什么不合格」,句子归表现层(其自带译文仅默认值):
    // 本卡无必答项,故 Required 与 Unanswered 同取「请选择一个选项或
    // 填写自定义答案。」
    let error_line = state.read(cx).error(&name).map(|error| match error {
        QuestionnaireValidationError::Message(message) => message.to_string(),
        _ => dict::ask::err_pick().to_string(),
    });
    let progress = state.read(cx).progress();
    let counter = format!("{}/{}", progress.current(), progress.total());
    // 选项文字列的 min_w(0):flex item 缺省最小宽 = 内容 max-content,长
    // ASCII 词元(不可断行)会把卡片撑出列宽——压回可用宽度让文本换行
    // (D54 同款修复;库的 content 槽默认不带,须由调用方经 content_style 给)
    let choice_content = StyleRefinement::default().min_w(px(0.));
    let choices = (0..option_count)
        .map(|oi| {
            let sel = format!("ask-opt-{oi}");
            div()
                .debug_selector(move || sel.clone())
                .child(
                    QuestionnaireChoice::new(&state, name.clone(), oi.to_string())
                        .content_style(choice_content.clone()),
                )
                .into_any_element()
        })
        .collect::<Vec<_>>();
    Some(
        div()
            .id("ask-question")
            .debug_selector(|| "ask-question".to_string())
            .w_full()
            .v_flex()
            .rounded(px(14.))
            .border_1()
            .border_color(theme::BORDER())
            .bg(theme::LAYER())
            .p(px(14.))
            .gap(px(10.))
            .child(
                // xsmall 是全卡唯一的尺寸来源(根节点发布,各部按 state id
                // 取回):库排版走 token 基准(md=16px),本仓正文是 11–13px,
                // 取默认 Medium 会让题面 18px、选项 14px 地整体放大一档。
                // xsmall 落到题面 14px / 选项与描述 12px,与卡壳同体量。
                Questionnaire::new(&state)
                    .xsmall()
                    .gap(px(10.))
                    .child(
                        QuestionnaireItem::new(&state, name.clone())
                            .child(
                                // 卡头行:左 = eyebrow(header,可选)+ 题面;
                                // header 缺席不回退到 question——回退曾造成
                                // 标题重复
                                div()
                                    .flex()
                                    .items_start()
                                    .justify_between()
                                    .w_full()
                                    .child(
                                        div()
                                            .v_flex()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .gap(px(2.))
                                            .when_some(eyebrow, |el, eyebrow| {
                                                el.child(
                                                    div()
                                                        .id("ask-eyebrow")
                                                        .debug_selector(|| {
                                                            "ask-eyebrow".to_string()
                                                        })
                                                        .text_size(px(11.))
                                                        .text_color(theme::CAPTION())
                                                        .child(eyebrow),
                                                )
                                            })
                                            .child(
                                                // 题面经子元素给定(库的回落文本即
                                                // accessibility_label,二者同源);
                                                // 包一层带 selector 的壳供测试寻址
                                                QuestionnaireTitle::new(&state, name.clone())
                                                    .child(
                                                        div()
                                                            .debug_selector(|| {
                                                                "ask-title".to_string()
                                                            })
                                                            .child(question_text),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id("ask-cancel")
                                            .debug_selector(|| "ask-cancel".to_string())
                                            .size(px(24.))
                                            .flex_shrink_0()
                                            .rounded_full()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .text_color(theme::CAPTION())
                                            .hover(|s| s.bg(theme::DOCK()))
                                            .on_click(move |_, _, cx| {
                                                cancel.update(cx, |st, cx| st.cancel_ask(cx))
                                            })
                                            .child(fixed(IconName::Close, 12.)),
                                    ),
                            )
                            .child(
                                QuestionnaireChoices::new(&state, name.clone()).children(choices),
                            )
                            .child(QuestionnaireInput::new(&state, name.clone()))
                            .children(error_line.map(|text| {
                                QuestionnaireError::new(&state, name.clone()).child(text)
                            })),
                    )
                    // 动作行:上一题 / 计数 / 下一题 在左,跳过 / 提交在右
                    // (库把「跳过」与「跳过缺席时的下一题」自动右靠);
                    // 计数在单题时同原实现一并省去
                    .child(
                        QuestionnaireActions::new(&state)
                            .when(total > 1, |el| {
                                el.child(QuestionnaireProgress::new(&state).child(counter))
                            })
                            .child(
                                QuestionnairePrevious::new(&state)
                                    .child(dict::ask::prev_q().to_string()),
                            )
                            .child(
                                QuestionnaireNext::new(&state)
                                    .child(dict::ask::next_q().to_string()),
                            )
                            .child(
                                QuestionnaireSkip::new(&state)
                                    .child(dict::ask::skip_q().to_string()),
                            )
                            .child(
                                QuestionnaireSubmit::new(&state)
                                    .child(dict::ask::submit_q().to_string()),
                            ),
                    ),
            ),
    )
}
