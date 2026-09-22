# liuma verify 全集
# 用法:just verify
#
# Windows 前置:真 Python 3(Store 的 python3 只是应用执行别名,运行时
# 退出 49)、just、wasm32-wasip2 目标、bash(Git for Windows 自带)。
# 用 `just preflight` 自检。

# Windows 上解释器名是 `python`(Python 官方安装器不产 `python3.exe`,
# `python3` 会落到 Store 别名);Unix 反之
python := if os() == "windows" { "python" } else { "python3" }

default:
    @just --list

# 开发前置自检:缺什么、怎么装,一次说清(不把「跑不了」当「通过」)
preflight:
    bash scripts/verify-preflight

# verify 脚本全集:格式 / lint / 测试 / 契约 / e2e / 链接
verify: fmt-check clippy test wit component-contracts e2e links
    @echo "verify: ALL GREEN"

# 桌面客户端(GPUI 原生 UI):构建 liuma-desktop;运行用 just desktop-run
desktop:
    cargo build -p liuma-desktop

# 桌面客户端运行;参数透传,如 just desktop-run --fake
desktop-run ARGS='':
    cargo run -p liuma-desktop -- {{ARGS}}

wit:
    bash scripts/verify-wit

component-contracts:
    bash scripts/verify-component-contracts

e2e:
    bash scripts/verify-e2e

links:
    {{python}} scripts/verify-links

fmt-check:
    cargo fmt --all --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace
