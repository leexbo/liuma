//! 全库检索 UI(从 ui::sidebar 切出):侧栏搜索框(本地过滤 + Enter
//! 触发全库检索)与命中面板(替换会话列表;行点击 = 开会话切轨迹定位)。

use gpui_kit::component::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::StyledExt;
use gpui_kit::component::input::Input;
use gpui_kit::{
    App, Entity, HighlightStyle, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, StyledText, div, px,
};

use crate::kits::i18n::t;
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
            .border_color(theme::border_2(cx))
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
                    .prefix(fixed(LiumaIcon::SearchOutline, 13.).text_color(theme::caption(cx)))
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
                            .text_color(theme::caption(cx))
                            .hover(|s| {
                                s.bg(theme::sidebar_hover(cx))
                                    .text_color(theme::label_2(cx))
                            })
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
                    .text_color(theme::caption(cx))
                    .flex_1()
                    .child(t!("misc.hits_full", n = hits.len())),
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
                    .text_color(theme::label_3(cx))
                    .hover(|s| s.bg(theme::sidebar_hover(cx)))
                    .child(t!("misc.back_to_list"))
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
                .text_color(theme::label_3(cx))
                .child(t!("misc.no_hits"))
                .into_any_element(),
        );
    }
    // 查询词取当前输入框值(命中面板仅在检索后呈现,输入仍在)
    let query = st
        .search
        .search_input
        .as_ref()
        .map(|i| i.read(cx).value().to_string())
        .unwrap_or_default();
    for (ix, h) in hits.iter().enumerate() {
        let s = store.clone();
        let sid = h["sessionId"].as_str().unwrap_or_default().to_string();
        let seq = h["seq"].as_u64().unwrap_or(0);
        let title = st.title_for(&sid);
        let kind = h["kind"].as_str().unwrap_or_default();
        let content = h["content"].as_str().unwrap_or_default();
        let (preview, hl_ranges) = hit_preview(content, &query, 40);
        let mark = HighlightStyle {
            background_color: Some(gpui_kit::Hsla::from(theme::brand(cx)).opacity(0.35)),
            ..Default::default()
        };
        let runs: Vec<(std::ops::Range<usize>, HighlightStyle)> =
            hl_ranges.into_iter().map(|r| (r, mark)).collect();
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
                .hover(|s| s.bg(theme::sidebar_hover(cx)))
                .child(fixed(LiumaIcon::Message, 14.).text_color(theme::label_3(cx)))
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
                                .text_color(theme::label_2(cx))
                                .child(title),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .min_w(px(0.))
                                .truncate()
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_size(px(11.))
                                        .text_color(theme::caption(cx))
                                        .child(format!("[{kind}]")),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .truncate()
                                        .text_color(theme::caption(cx))
                                        .child(StyledText::new(preview).with_highlights(runs)),
                                ),
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

/// 命中摘要 + 高亮区间(纯函数;§6 检索收口的呈现侧):忽略大小写
/// 子串定位(char 对齐,CJK 安全),预览窗口以首命中为中心;无命中
/// 回落平文前 max 字符。返回 (预览文本, 预览内字节高亮区间)
pub(crate) fn hit_preview(
    content: &str,
    query: &str,
    max: usize,
) -> (String, Vec<std::ops::Range<usize>>) {
    // 平文折叠 + 逐字符小写(1:1 字符对齐;不取 to_lowercase 的整串
    // 变换——个别字符会展开变长,字节错位)
    let flat: String = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = flat.chars().collect();
    let lower: String = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.trim_end_matches('*').replace('"', "").trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    // 全部词出现区间(char 索引;上限防御超长文档)
    let mut hits: Vec<(usize, usize)> = Vec::new();
    for t in &terms {
        let tl: String = t
            .chars()
            .map(|c| c.to_lowercase().next().unwrap_or(c))
            .collect();
        if tl.is_empty() {
            continue;
        }
        let tlen = tl.chars().count();
        let mut from = 0usize;
        while let Some(rel) = lower[from..].find(&tl) {
            let b = from + rel;
            let cs = lower[..b].chars().count();
            hits.push((cs, cs + tlen));
            from = b + tl.len();
            if hits.len() >= 32 {
                break;
            }
        }
    }
    let byte_at = |ci: usize| {
        flat.char_indices()
            .nth(ci)
            .map(|(b, _)| b)
            .unwrap_or(flat.len())
    };
    match hits.first() {
        None => (chars.iter().take(max).collect(), vec![]),
        Some(&(hs, he)) => {
            let total = chars.len();
            let win = max.min(total);
            let start = hs
                .saturating_sub(win.saturating_sub(he - hs) / 2)
                .min(total.saturating_sub(win));
            let sb = byte_at(start);
            let end_char = (start + win).min(total);
            let preview: String = chars[start..end_char].iter().collect();
            // 窗口内全部命中映射为预览内字节区间
            let ranges = hits
                .iter()
                .filter(|(s, e)| *s >= start && *e <= end_char)
                .map(|(s, e)| byte_at(*s) - sb..byte_at(*e) - sb)
                .collect();
            (preview, ranges)
        }
    }
}

#[cfg(test)]
mod preview_tests {
    use super::hit_preview;

    #[test]
    fn hit_preview_centers_and_marks_cjk() {
        let content = "这是开头。中间有一段提到持久化的设计,后面还有内容继续延伸";
        let (preview, ranges) = hit_preview(content, "持久化", 20);
        assert!(preview.contains("持久化"), "预览应含命中: {preview}");
        assert_eq!(ranges.len(), 1);
        let marked = &preview[ranges[0].clone()];
        assert_eq!(marked, "持久化", "区间应精确切中命中词");
        // 首命中居中:命中不在窗口首位
        assert!(ranges[0].start > 0, "命中应居中而非贴窗首");
    }

    #[test]
    fn hit_preview_case_insensitive_ascii_and_fallback() {
        // 星号剥除后按查询词长度标记(命中 Provider 的前 6 字符)
        let (preview, ranges) = hit_preview("fix Provider registry later on", "provid*", 40);
        assert!(preview.contains("Provider"));
        assert_eq!(ranges.len(), 1);
        assert_eq!(&preview[ranges[0].clone()], "Provid");
        // 无命中:平文回落,无区间
        let (preview, ranges) = hit_preview("普通内容一行", "不存在词", 40);
        assert_eq!(preview, "普通内容一行");
        assert!(ranges.is_empty());
        // 多词:各词分别定位
        let (preview, ranges) = hit_preview("alpha beta gamma", "beta gamma", 40);
        assert_eq!(ranges.len(), 2);
        assert_eq!(&preview[ranges[0].clone()], "beta");
        assert_eq!(&preview[ranges[1].clone()], "gamma");
    }
}
