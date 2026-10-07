//! gpui-pre 兼容垫片:名称/版本与 gpui-kit 所钉 `=0.3.8` 快照一致,
//! 内容转发到 crates.io 的 gpui-fast(retained-mode 核心)。见根
//! Cargo.toml 的 `[patch.crates-io]` 与 AGENTS.md §1 `[Deps]` 守卫。
pub use gpui_fast::*;
