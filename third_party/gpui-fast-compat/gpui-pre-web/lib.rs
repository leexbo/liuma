//! gpui-pre-web 兼容垫片:转发到 crates.io 的 gpui-fast-web(wasm 后端,
//! 桌面目标不编译;不 patch 会与 gpui-fast-web 在 unicode-properties 上撞版)。
pub use gpui_fast_web::*;
