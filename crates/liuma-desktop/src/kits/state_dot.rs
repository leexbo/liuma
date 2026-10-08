//! 会话「进行中」状态点:dsh 圆环 spinner——静默轨道环(0.2 透明度)
//! + 呼吸弧旋转,颜色 = 三级灰(theme::caption,静默非蓝)。
//!
//! 实现走组件库 `ProgressCircle::loading`(自带 1s 循环的弧呼吸动画,
//! 「UI 先查组件库」;dsh 的整环旋转 + 弧长呼吸视觉近似,严格一致需自绘
//! 弧,后续有需要再换)。相位是元素各自的动画时钟,跨实例不钉同一
//! 起点(dsh 钉文档时间零)——差异仅在多个会话同时运行时可感知,
//! 接受该近似。
//!
//! 减弱动态(reduce_motion):静止为 1/4 弧段(对齐 dsh 的静态弧回退)。

use gpui_kit::component::Sizable as _;
use gpui_kit::component::progress::ProgressCircle;
use gpui_kit::{App, IntoElement, ParentElement, Styled, div, px};

use crate::kits::theme;

/// 进行中状态点(圆环 spinner;`size` 为外径 px)。
pub fn ongoing_ring(size: f32, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .flex_shrink_0()
        .size(px(size))
        .child(if cx.reduce_motion() {
            // 静态 1/4 弧:减弱动态不转
            ProgressCircle::new("state-dot-static")
                .value(25.)
                .color(theme::caption(cx))
                .with_size(px(size))
        } else {
            ProgressCircle::new("state-dot-ongoing")
                .loading(true)
                .color(theme::caption(cx))
                .with_size(px(size))
        })
}

/// 待答实心点(琥珀;10px 槽内 6px 圆点,对齐 dsh warning 态)。
pub fn pending_dot(cx: &App) -> impl IntoElement {
    div()
        .flex()
        .flex_shrink_0()
        .size(px(10.))
        .items_center()
        .justify_center()
        .child(div().size(px(6.)).rounded_full().bg(theme::warning(cx)))
}
