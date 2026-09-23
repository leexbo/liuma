//! 消息反馈 UI:
//! assistant 消息动作行的赞/踩/备注按钮 + 备注弹窗。
//! 反馈是 host sidecar(per-session JSON),不进模型上下文;
//! 语义:点当前评分 = 删除;点另一评分 = put 带原备注前移。

use gpui_kit::component::IconName;
use gpui_kit::component::StyledExt;
use gpui_kit::component::input::Textarea;
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::{
    Anchor, App, Entity, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, px,
};

use crate::kits::i18n::dict;
use crate::kits::icons::fixed;
use crate::kits::popup::PopTrigger;
use crate::kits::theme;
use crate::shell::store::AppStore;

/// assistant 动作行的反馈控件(赞/踩/备注;返回子元素数组)。
pub fn actions(store: &Entity<AppStore>, message_id: &str, cx: &App) -> Vec<gpui_kit::AnyElement> {
    let st = store.read(cx);
    if message_id.is_empty() {
        return Vec::new();
    }
    let item = st.feedback.feedback_by_message.get(message_id).cloned();
    let rating = item.as_ref().map(|it| it.rating.as_str()).unwrap_or("");
    let note = item.as_ref().and_then(|it| it.note.clone());
    let mut els: Vec<gpui_kit::AnyElement> = Vec::new();

    // 赞
    let like_store = store.clone();
    let like_sel = message_id.to_string();
    let like_active = rating == "positive";
    els.push(
        feedback_btn(
            gpui_kit::SharedString::from(format!("fb-like-{like_sel}")),
            format!("fb-like-{like_sel}"),
            IconName::ThumbsUp,
            like_active,
            move |cx| {
                let s = like_store.clone();
                let m = like_sel.clone();
                s.update(cx, |st, cx| {
                    st.rate_message(&m, "positive", None, cx);
                });
            },
        )
        .into_any_element(),
    );
    // 踩
    let dislike_store = store.clone();
    let dislike_sel = message_id.to_string();
    let dislike_active = rating == "negative";
    els.push(
        feedback_btn(
            gpui_kit::SharedString::from(format!("fb-dislike-{dislike_sel}")),
            format!("fb-dislike-{dislike_sel}"),
            IconName::ThumbsDown,
            dislike_active,
            move |cx| {
                let s = dislike_store.clone();
                let m = dislike_sel.clone();
                s.update(cx, |st, cx| {
                    st.rate_message(&m, "negative", None, cx);
                });
            },
        )
        .into_any_element(),
    );
    // 备注钮:仅在有评分时出现。无评分时动作行 = 复制+赞/踩,不常驻
    // 「补充说明」文字;有了评分才让用户补一句说明。
    if rating.is_empty() {
        return els;
    }
    let note_store = store.clone();
    let note_sel = message_id.to_string();
    els.push(
        // 备注弹层(组件库 Popover:下开、外点关闭;输入态懒建与聚焦
        // 由内容闭包的 window 完成,store 不再有坐标锚)
        Popover::new(gpui_kit::SharedString::from(format!(
            "fb-note-pop-{note_sel}"
        )))
        .appearance(false)
        .anchor(Anchor::TopLeft)
        .trigger(PopTrigger(
            div()
                .id(gpui_kit::SharedString::from(format!("fb-note-{note_sel}")))
                .flex()
                .h(px(24.))
                .items_center()
                .rounded(px(12.))
                .px(px(8.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::DOCK()))
                .text_size(px(11.))
                .text_color(if note.is_some() {
                    theme::BRAND()
                } else {
                    theme::CAPTION()
                })
                .child(if note.is_some() {
                    note.clone().unwrap_or_default()
                } else {
                    dict::misc::supplement().to_string()
                }),
        ))
        .content(move |_, window, cx| {
            let pop = cx.entity();
            let s_open = note_store.clone();
            s_open.update(cx, |st, cx| {
                st.open_feedback_note(window, &note_sel, cx);
                if let Some(input) = &st.feedback.feedback_input {
                    input.update(cx, |i, cx| i.focus(window, cx));
                }
            });
            note_editor_card(&note_store, pop, cx).into_any_element()
        })
        .into_any_element(),
    );
    els
}

/// 单个评分钮(选中态高亮 + 点击评分;`sel` = 调试选择器,按消息唯一)
fn feedback_btn(
    id: impl Into<gpui_kit::ElementId>,
    sel: String,
    icon: IconName,
    active: bool,
    on_click: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .debug_selector(move || sel.clone())
        .size(px(24.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(theme::DOCK()))
        .text_color(if active {
            theme::BRAND()
        } else {
            theme::CAPTION()
        })
        .on_click(move |_, _, cx| on_click(cx))
        .child(fixed(icon, 12.))
}

/// 备注卡(组件库 Popover 内容:textarea + 保存/取消;保存与取消均
/// 收起弹层,外点关闭由库托管)
fn note_editor_card(
    store: &Entity<AppStore>,
    pop: Entity<PopoverState>,
    cx: &mut App,
) -> impl IntoElement {
    let text = store
        .read(cx)
        .feedback
        .feedback_note_editor
        .clone()
        .map(|(_, t)| t)
        .unwrap_or_default();
    let (save, close) = (store.clone(), store.clone());
    let pop_save = pop.clone();
    div()
        .debug_selector(|| "fb-note-pop".to_string())
        .v_flex()
        .gap(px(10.))
        .w(px(320.))
        .max_w(px(360.))
        .rounded(px(12.))
        .border_1()
        .border_color(theme::BORDER())
        .bg(theme::LAYER())
        .p(px(12.))
        .shadow_md()
        .child(
            div()
                .id("fb-note-input")
                .flex()
                .min_h(px(72.))
                .items_center()
                .rounded(px(8.))
                .border_1()
                .border_color(theme::BORDER())
                // 卡上内嵌输入面:CODE(比 CARD 深一阶的内嵌语义)
                .bg(theme::CODE())
                .px(px(10.))
                .py(px(8.))
                .child(
                    store
                        .read(cx)
                        .feedback
                        .feedback_input
                        .clone()
                        .map(|input| {
                            Textarea::new(&input)
                                .appearance(false)
                                .text_size(px(13.))
                                .line_height(gpui_kit::relative(1.5))
                                .into_any_element()
                        })
                        .unwrap_or_else(|| div().child(text).into_any_element()),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.))
                .child(
                    div()
                        .id("fb-note-save")
                        .flex()
                        .h(px(28.))
                        .items_center()
                        .justify_center()
                        .rounded(px(14.))
                        // 白色主按钮(白底深字)
                        .bg(theme::LABEL())
                        .px(px(16.))
                        .text_size(px(13.))
                        .text_color(gpui_kit::black())
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::LABEL_2()))
                        .on_click(move |_, window, cx| {
                            pop_save.update(cx, |state, cx| state.dismiss(window, cx));
                            save.update(cx, |st, cx| st.commit_feedback_note(cx));
                        })
                        .child(dict::common::save()),
                )
                .child(
                    div()
                        .id("fb-note-cancel")
                        .flex()
                        .h(px(28.))
                        .items_center()
                        .rounded(px(14.))
                        .px(px(12.))
                        .text_size(px(13.))
                        .text_color(theme::CAPTION())
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::DOCK()))
                        .on_click(move |_, window, cx| {
                            pop.update(cx, |state, cx| state.dismiss(window, cx));
                            close.update(cx, |st, cx| st.close_feedback_note(cx));
                        })
                        .child(dict::common::cancel()),
                ),
        )
}
