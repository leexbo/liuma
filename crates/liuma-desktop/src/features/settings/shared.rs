//! 跨页共享辅助:selector id、字段行族、标题/说明行、分隔线、空态居中。

use super::*;

/// 动态元素 id(SharedString 进 ElementId)
pub(crate) fn sid(prefix: &str, key: &str) -> gpui_kit::SharedString {
    gpui_kit::SharedString::from(format!("{prefix}-{key}"))
}

/// 区说明行(13/tertiary)
pub(crate) fn intro_line(text: impl Into<String>) -> impl IntoElement {
    div()
        .text_size(px(14.))
        .text_color(theme::LABEL_3())
        .child(text.into())
}

/// 区标题(16/500)
pub(crate) fn section_title(text: impl Into<gpui_kit::SharedString>) -> impl IntoElement {
    let text = text.into();
    div()
        .text_size(px(16.))
        .font_weight(gpui_kit::FontWeight::MEDIUM)
        .child(text.to_string())
}

/// MCP Servers 区:server 行卡(id/command/enabled 开关/移除)+ 添加卡。
/// 字段控件构造器(fn 指针 + AnyElement:闭包泛型在此会撞上
/// HRTB 推断,函数指针直接绕开)
type FieldControl =
    fn(&gpui_kit::Entity<gpui_kit::component::input::InputState>) -> gpui_kit::AnyElement;

/// 通用字段输入行核心(标签 + 由 `control` 构建的控件)。`sel` = 输入
/// 包装的布局回归锚;标签用 LABEL_2——表头是行内主控的名称,38% 的
/// CAPTION 让它淡得像禁用态,72% 的次级档才是表单标签的常规层级
pub(crate) fn field_row(
    label: impl Into<gpui_kit::SharedString>,
    sel: &'static str,
    input: &Option<gpui_kit::Entity<gpui_kit::component::input::InputState>>,
    control: FieldControl,
) -> impl IntoElement {
    let label = label.into();
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .w(px(64.))
                .flex_shrink_0()
                .text_size(px(11.))
                .text_color(theme::LABEL_2())
                .child(label.to_string()),
        )
        .children(input.as_ref().map(|e| {
            div()
                .id(gpui_kit::SharedString::from(sel))
                .debug_selector(move || sel.to_string())
                .flex_1()
                .min_w(px(0.))
                .h(px(32.))
                .child(control(e))
        }))
}

/// 文本字段行
pub(crate) fn field_input(
    label: impl Into<gpui_kit::SharedString>,
    sel: &'static str,
    input: &Option<gpui_kit::Entity<gpui_kit::component::input::InputState>>,
) -> impl IntoElement {
    field_row(label, sel, input, |e| Input::new(e).into_any_element())
}

/// 数字控件(NumberInput:步进按钮 + 失焦 clamp,区间/步长在
/// InputState 构造侧配,见 `ensure_decision_form_inputs`)
pub(crate) fn field_number_control(
    e: &gpui_kit::Entity<gpui_kit::component::input::InputState>,
) -> gpui_kit::AnyElement {
    gpui_kit::component::input::NumberInput::new(e).into_any_element()
}

/// 定宽数字字段行 + 尾随 hint(决策区的阈值/超时行;hint 是 11px
/// CAPTION)。四个数字行共用固定 160px 输入宽:hint 文字长短不一,若
/// 让输入框吃 flex_1,各行右缘会参差不齐;定宽后输入列对齐,hint 由
/// 弹性空档统一贴行尾
pub(crate) fn field_number_hinted(
    label: impl Into<gpui_kit::SharedString>,
    sel: &'static str,
    input: &Option<gpui_kit::Entity<gpui_kit::component::input::InputState>>,
    hint: impl Into<gpui_kit::SharedString>,
) -> impl IntoElement {
    let label = label.into();
    let hint = hint.into();
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .w(px(64.))
                .flex_shrink_0()
                .text_size(px(11.))
                .text_color(theme::LABEL_2())
                .child(label.to_string()),
        )
        .children(input.as_ref().map(|e| {
            div()
                .id(gpui_kit::SharedString::from(sel))
                .debug_selector(move || sel.to_string())
                .w(px(160.))
                .flex_shrink_0()
                .h(px(32.))
                .child(field_number_control(e))
        }))
        .child(div().flex_1().min_w(px(0.)))
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(11.))
                .text_color(theme::CAPTION())
                .child(hint.to_string()),
        )
}

/// 段间分隔线(字段区 / 模型目录 / 计费段之间)
pub(crate) fn section_divider() -> gpui_kit::AnyElement {
    div()
        .w_full()
        .h(px(1.))
        .bg(theme::BORDER_2())
        .into_any_element()
}

pub(crate) fn field_label(text: impl Into<gpui_kit::SharedString>) -> impl IntoElement {
    let text = text.into();
    div()
        .text_size(px(12.))
        .font_weight(gpui_kit::FontWeight::MEDIUM)
        .text_color(theme::LABEL_2())
        .child(text.to_string())
}

/// 信息行(label 11 说明号 + 值 13 正文号)
pub(crate) fn info_line(
    label: impl Into<gpui_kit::SharedString>,
    value: impl Into<String>,
) -> impl IntoElement {
    let label = label.into();
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .w(px(72.))
                .flex_shrink_0()
                .text_size(px(11.))
                .text_color(theme::CAPTION())
                .child(label.to_string()),
        )
        .child(
            div()
                .flex()
                .min_w(px(0.))
                .flex_1()
                .truncate()
                .text_size(px(13.))
                .text_color(theme::LABEL_2())
                .child(value.into()),
        )
}

/// 说明行(11 说明号)
pub(crate) fn caption_line(text: impl Into<String>) -> impl IntoElement {
    div()
        .text_size(px(11.))
        .text_color(theme::CAPTION())
        .child(text.into())
}

/// 归档区居中态(加载/空/零命中):余高内水平垂直居中
pub(crate) fn centered_state(sel: &'static str, text: impl Into<String>) -> impl IntoElement {
    div()
        .debug_selector(move || sel.to_string())
        .flex_1()
        .min_h(px(0.))
        .flex()
        .items_center()
        .justify_center()
        .child(caption_line(text))
}
