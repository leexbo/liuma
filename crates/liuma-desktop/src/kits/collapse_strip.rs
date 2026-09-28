//! composer 上方折叠条外壳(计划条与子代理任务条共用同一视觉语言)。
//!
//! 开合行为归库 `Accordion`(头行 = 触发器、行列表 = 面板);本模块只
//! 固定两处共用的**应用演示面**:容器 chrome(圆角描边卡 + LAYER 底)、
//! 头行几何(30px 定高 / 常规字重)与面板内边距。
//!
//! 展开态受控:真相源在各 feature 自己的 store 字段,由 `on_toggle`
//! 回传的**结果态**写回(库给的是点击后的开集,不是「切换」请求)。
//!
//! 两处必设的覆盖,均已核实过 0.7.0 源码:
//! - `.bordered(false)`:库默认那套是实心圆角描边卡,叠在容器 chrome 上
//!   会出第二层边框;
//! - `.font_weight(NORMAL)`:库触发器自带 `font_medium`,而 `refine_style`
//!   只能以 `Some` 覆盖,不写回就整条头行变中粗。
//!
//! 库触发器尾部的 chevron 颜色硬编码为 `cx.theme().muted_foreground`
//! (本仓主题 = `label_3`,α 0.55/0.50),**无覆盖点**;与手绘版原用的
//! `theme::CAPTION()`(α 0.38/0.35)相比箭头更重,这是迁移的既知代价。

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::accordion::Accordion;
use gpui_kit::{
    AnyElement, App, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StyleRefinement, Styled as _, Window, div, px,
};

use crate::kits::theme;

/// 折叠条整体:容器 chrome + 单个 Accordion 项。
///
/// - `id` 同时用作容器 `id` 与测试选择器(release 下选择器为空操作);
/// - `title` 是头行内容(chevron 由库追加在尾部);
/// - `rows` 是展开后的行集合;`pad_x` 为面板左右内边距。
pub fn strip(
    id: &'static str,
    open: bool,
    title: impl IntoElement,
    pad_x: f32,
    rows: impl IntoIterator<Item = impl IntoElement>,
    on_toggle: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .w_full()
        .rounded(px(14.))
        .border_1()
        .border_color(theme::BORDER())
        .bg(theme::LAYER())
        .overflow_hidden()
        .child(
            Accordion::new(id)
                .bordered(false)
                // 库根是 `size_full()`;本条的容器高度由内容决定
                .h_auto()
                .w_full()
                .item(|item| {
                    item.open(open)
                        // 面板透出容器的 LAYER(库默认读 tokens.accordion)
                        .bg(theme::TRANSPARENT())
                        .title_style(head_style())
                        .title(div().flex().items_center().gap(px(8.)).child(title))
                        .content_style(panel_style(pad_x))
                        .child(
                            div().v_flex().gap(px(2.)).children(
                                rows.into_iter()
                                    .map(IntoElement::into_any_element)
                                    .collect::<Vec<_>>(),
                            ),
                        )
                })
                .on_toggle_click(move |open_indices, window, cx| {
                    on_toggle(!open_indices.is_empty(), window, cx);
                }),
        )
        .into_any_element()
}

/// 头行样式:库触发器默认按 Medium 尺寸排(py 8 / px 12、间距 12、
/// `font_medium`);这里压回本族的 30px 定高 + 8px 间距 + 常规字重。
fn head_style() -> StyleRefinement {
    StyleRefinement::default()
        .h(px(30.))
        .py(px(0.))
        .px(px(12.))
        .gap(px(8.))
        .font_weight(FontWeight::NORMAL)
}

/// 面板样式:库默认 `pb_2 px_3`,这里只改左右内边距(任务条比计划条窄)
fn panel_style(pad_x: f32) -> StyleRefinement {
    StyleRefinement::default().px(px(pad_x)).pb(px(8.))
}
