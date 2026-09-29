//! rust-i18n 后端装配:全仓唯一的 `i18n!` 调用点。
//!
//! `i18n!` 在宏展开期读 `CARGO_MANIFEST_DIR/locales/**/*.{yml,yaml,json,toml}`
//! 并生成本 crate 专属的 backend(每个 crate 一份,`gpui-component` 自己那
//! 份互不覆盖)与 `_rust_i18n_*` 辅助项;`available_locales!` / `extend!` /
//! `tkv!` 等支撑面按需取用,故整模块豁免 `dead_code`(bin crate 下未接线
//! 的 `pub fn` 会触发 clippy `-D warnings`)。
//!
//! `t!` 展开为 `crate::_rust_i18n_t!` → `crate::_rust_i18n_try_translate`,
//! 两个名字由 crate 根 `pub(crate) use` 再导出(见 `main.rs`)。
//!
//! locale 文件改动不触发重编的问题由 `build.rs` 的
//! `cargo:rerun-if-changed=locales` 兜住(proc-macro 的文件读取不进 cargo
//! fingerprint)。
#![allow(dead_code)]

rust_i18n::i18n!("locales", fallback = "en");
