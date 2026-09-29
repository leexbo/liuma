//! 构建脚本:locales/*.yml 的重编追踪。
//!
//! `rust_i18n::i18n!` 是过程宏,在展开期把 `locales/**` 读进编译产物;cargo
//! 看不见这次文件读取(proc-macro 的文件访问不进 fingerprint),实测改
//! YAML 后 `cargo build` 不重编、界面照旧显示旧文案。本脚本把 locales 目录
//! 登记为构建输入,任何一条文案改动都会触发重编。

fn main() {
    println!("cargo:rerun-if-changed=locales");
}
