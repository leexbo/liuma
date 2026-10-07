//! gpui-pre-macros 兼容垫片:转发到 crates.io 的 gpui-fast-macros。
//! 注意:gpui-fast 的宏展开输出字面 `gpui::` 路径,不做 stock 宏的
//! `::gpui_kit::` 门面改写 —— 消费方 crate 根需 `extern crate gpui_kit as gpui;`。
pub use gpui_fast_macros::*;
