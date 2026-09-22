//! 全库检索 UI(从 ui::sidebar 切出):侧栏搜索框(本地过滤 + Enter
//! 触发全库检索)与命中面板(替换会话列表;行点击 = 开会话切轨迹定位)。

use gpui_kit::component::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::StyledExt;
use gpui_kit::component::input::Input;
use gpui_kit::{
    App, Entity, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, div, px,
};

use crate::kits::i18n::dict;
use crate::kits::icons::{LiumaIcon, fixed};
use crate::kits::theme;
use crate::shell::store::AppStore;

/// 顶栏搜索框(搜索态:替换标题行;本地过滤 + Enter 全库检索)。
/// 前导放大镜 + 尾部 × 钮(清空并收起);输入区自挂 mousedown 豁免,
/// 点击聚焦不得触发头行的窗口拖拽
pub(crate) fn search_field(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let s_clear = store.clone();
    let input = st.search.search_input.as_ref().map(|e| {
        div()
            .flex()
            .flex_1()
            .min_w(px(0.))
            .h(px(30.))
            .items_center()
            .rounded(px(10.))
            .border_1()
            .border_color(theme::BORDER_2())
            .pl(px(6.))
            .pr(px(4.))
            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                cx.stop_propagation()
            })
            .child(
                Input::new(e)
                    .small()
                    // 无组件自带边框/底色/聚焦环:外框由包装层提供,
                    // 避免双层描边错位重叠
                    .appearance(false)
                    .prefix(fixed(LiumaIcon::SearchOutline, 13.).text_color(theme::CAPTION()))
                    .suffix(
                        div()
                            .id("search-clear")
                            .debug_selector(|| "search-clear".to_string())
                            .flex()
                            .size(px(20.))
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .cursor_pointer()
                            .text_color(theme::CAPTION())
                            .hover(|s| s.bg(theme::SIDEBAR_HOVER()).text_color(theme::LABEL_2()))
                            .child(fixed(IconName::Close, 12.))
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                s_clear.update(cx, |st, cx| st.clear_search(window, cx));
                            }),
                    ),
            )
    });
    div().flex().h(px(30.)).items_center().children(input)
}

/// 全库检索命中面板:回车检索后替换会话列表;行 = 会话标题
/// + 命中内容摘要,点击 = 开会话切轨迹定位台账行;清空搜索框即回列表
pub(crate) fn search_hits_panel(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let hits = st.search.search_hits.clone().unwrap_or_default();
    let back = store.clone();
    let mut rows: Vec<gpui_kit::AnyElement> = vec![
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme::CAPTION())
                    .flex_1()
                    .child(dict::misc::hits_full(hits.len())),
            )
            .child(
                div()
                    .id("search-hits-back")
                    .debug_selector(|| "search-hits-back".to_string())
                    .flex()
                    .h(px(20.))
                    .items_center()
                    .px(px(6.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .text_size(px(11.))
                    .text_color(theme::LABEL_3())
                    .hover(|s| s.bg(theme::SIDEBAR_HOVER()))
                    .child(dict::misc::back_to_list())
                    .on_click(move |_, window, cx| {
                        back.update(cx, |st, cx| st.clear_search(window, cx));
                    }),
            )
            .into_any_element(),
    ];
    if hits.is_empty() {
        rows.push(
            div()
                .text_size(px(12.))
                .text_color(theme::LABEL_3())
                .child(dict::misc::no_hits())
                .into_any_element(),
        );
    }
    for (ix, h) in hits.iter().enumerate() {
        let s = store.clone();
        let sid = h["sessionId"].as_str().unwrap_or_default().to_string();
        let seq = h["seq"].as_u64().unwrap_or(0);
        let title = st.title_for(&sid);
        let kind = h["kind"].as_str().unwrap_or_default();
        let preview: String = {
            let c = h["content"].as_str().unwrap_or_default();
            let first = c.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            first.chars().take(40).collect()
        };
        let sel = format!("search-hit-{ix}");
        rows.push(
            div()
                .id(("search-hit", ix))
                .debug_selector(move || sel.clone())
                .flex()
                .h(px(34.))
                .items_center()
                .gap(px(8.))
                .rounded(px(8.))
                .pl(px(8.))
                .pr(px(8.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::SIDEBAR_HOVER()))
                .child(fixed(LiumaIcon::MessageSquare, 14.).text_color(theme::LABEL_3()))
                .child(
                    div()
                        .v_flex()
                        .min_w(px(0.))
                        .flex_1()
                        .gap(px(1.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .truncate()
                                .text_color(theme::LABEL_2())
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .truncate()
                                .text_color(theme::CAPTION())
                                .child(format!("[{kind}] {preview}")),
                        ),
                )
                .on_click(move |_, _, cx| {
                    let (sid, seq) = (sid.clone(), seq);
                    s.update(cx, |st, cx| st.open_search_hit(&sid, seq, cx));
                })
                .into_any_element(),
        );
    }
    div()
        .id("search-hits-panel")
        .debug_selector(|| "search-hits-panel".to_string())
        .v_flex()
        .min_h(px(0.))
        .flex_1()
        .overflow_y_scroll()
        .gap(px(2.))
        .pb(px(8.))
        .children(rows)
}
