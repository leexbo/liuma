//! 组件库浮层适配:Popover::trigger 要求 `Selectable + IntoElement`,
//! 库只给自家组件(Button 等)实现了 Selectable,自绘 div 触发钮用不
//! 上 —— 本地新类型 [`PopTrigger`] 包装 Stateful<Div> 补上(孤儿规则
//! 允许:本地类型可实现外部 trait)。选中态即「触发钮随弹层开合提亮」
//! 的挂点,liuma 触发钮暂无选中形制,恒 no-op。

use gpui_kit::component::Selectable;
use gpui_kit::{Div, IntoElement, Stateful};

/// Popover 触发钮适配(包一层自绘 Stateful div)
pub(crate) struct PopTrigger(pub Stateful<Div>);

impl Selectable for PopTrigger {
    fn selected(self, _selected: bool) -> Self {
        self
    }

    fn is_selected(&self) -> bool {
        false
    }
}

impl IntoElement for PopTrigger {
    type Element = <Stateful<Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        self.0.into_element()
    }
}
