[English](README.md) | [简体中文](README.zh-CN.md)

<div align="center">
  <img src="crates/liuma-desktop/assets/logo.svg" alt="liuma 流马" width="200"/>
  <h1>liuma 流马</h1>
</div>

[![ci](https://github.com/leexbo/liuma/actions/workflows/ci.yml/badge.svg)](https://github.com/leexbo/liuma/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

liuma is an AI coding agent that runs locally: it connects to multiple LLM providers and drives multi-turn conversation with tool execution; tools run safely inside a controlled sandbox, and everything is fully recorded and replayable; a native desktop client is included. The name comes from the *wooden ox and flowing horse* of Three Kingdoms lore — transport agents that need neither food nor rest and keep moving on their own.

The project took early inspiration from the development-snapshot semantics of [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness).

![liuma desktop client](screenshot.png)

## Features

- **Event sourcing**: all session state is an append-only JSONL event log; model history, audit, and telemetry are all projections — replaying the same log at any moment yields identical results, and the session recovers from the log after a crash.
- **Structural invariant**: "visible to the model ⟺ recorded" is enforced by the invariant gate at the single outbound choke point, not by caller discipline; execution is refused outright when the sandbox is unavailable.
- **WASM component tools**: interfaces are defined by WIT contracts (`wit/`) and run on wasmtime; component permissions are bounded by explicit capability bundles, and clock and random sources are injected explicitly to keep replay deterministic.
- **Sandboxed execution**: macOS Seatbelt / Linux Landlock / bubblewrap / Windows restricted token + capability SID authorization sandbox chain (fail-closed); three-state permissions plus an approval gate (ask / never).
- **Decision model integration**: open-category access to System One decision models (`noul` / `choice` / `score` question types returning structured answers constrained to pre-declared options), covering four scenarios — approval reviewer / stop sentinel / tool guard / context judge — plus a `decide` tool the model can consult proactively. Thresholds and question copy live in one place for easy human review; everything is off by default, advises without auto-executing, falls back to the previous behavior when the service is unavailable, and leaves an audit record for every query (single configuration surface under "Decision model" in Settings).
- **Native desktop client**: GPUI desktop app (chat / plan approval / Q&A cards / trajectory inspector / full-text search / session export), plus a `liuma` CLI (REPL / JSON-RPC stdio gateway).

## Getting Started

Prerequisites (`just preflight` self-checks them and prints the install command for anything missing):

| Dependency | Purpose & install |
|---|---|
| Rust 1.99+ | pinned in `rust-toolchain.toml`; Unix: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh`, Windows: [rustup](https://rustup.rs) |
| `wasm32-wasip2` target | contract tests compile real wasm components: `rustup target add wasm32-wasip2` |
| `just` | command entry point: `cargo install just` / `brew install just` / `winget install Casey.Just` |
| Real Python 3 | required by `scripts/verify-links` and the MCP fixture; Unix: `brew install python3` / `apt install python3`, Windows: `winget install --id Python.Python.3.13` |
| `bash` | preinstalled on Unix; on Windows use the copy bundled with Git for Windows |

Windows additionally needs `pwsh` (the runtime behind shell tools): `winget install --id Microsoft.PowerShell`. The `python3` from the Microsoft Store is merely an app-execution alias (the command exists and exits 49 immediately); it is not a real interpreter.

```bash
git clone git@github.com:leexbo/liuma.git && cd liuma
just preflight    # dev-prerequisite self-check: what's missing and how to install it, in one pass
```

Build:

```bash
cargo build --workspace    # all crates; the first build includes wasm components and the desktop app and takes a while
just desktop               # build only the GPUI desktop client
```

Run:

```bash
# offline self-check (fake provider, scripted echo)
cargo run -p liuma -- chat --fake

# connect to a real provider (reads DEEPSEEK_API_KEY by default; --dialect switches anthropic / openai-chat etc.)
cargo run -p liuma -- chat

# JSON-RPC stdio gateway
cargo run -p liuma -- serve

# GPUI desktop client
just desktop-run        # passes args through, e.g. just desktop-run --fake
```

## macOS Packaging

```bash
just package-macos    # produces dist/Liuma.app + zip + dmg (Darwin only)
```

The release build (Liuma-\<version\>-\<arch\>.zip / .dmg, version taken from the root `Cargo.toml`) is ad-hoc signed and works out of the box on this machine; it is not notarized, so when distributing to others Gatekeeper requires right-click → Open or `xattr -cr` on first launch. The app icon and the runtime Dock icon share one source: `logo.svg` is rasterized offline by resvg (`examples/emit-app-icon`) and packaged into `.icns` via `iconutil`; the bundle template lives in `crates/liuma-desktop/packaging/`.

Packaging currently covers macOS only; Windows / Linux are not yet supported.

## Verify

```bash
just verify    # fmt / clippy / tests / WIT / component contracts / e2e / link check / desktop copy
```

## Repository Layout

| Path | Responsibility |
|---|---|
| `wit/` | Contract layer; the single source of truth for all WIT packages |
| `crates/liuma` | Host binary (thin CLI shell) |
| `crates/liuma-app` | Session assembly layer (config merge / prompt assembly / preset tool assembly) |
| `crates/liuma-core` | Multi-session application core (registry / client-side protocol types / trajectory & stats projections) |
| `crates/liuma-desktop` | GPUI desktop client |
| `crates/liuma-desktop/packaging/` | macOS bundle template (Info.plist) |
| `crates/liuma-host` | Component host (wasmtime component manager / event bus / persistence / gateway) |
| `crates/liuma-llm` | LLM access (dialect engines / HTTP+SSE transport / invariant gate) |
| `crates/liuma-sandbox` | Execution primitives (sandbox chain / controlled spawn / PTY) |
| `crates/liuma-sandbox-winacl` | Windows sandbox backend (restricted token + capability SID authorization + Job Object) |
| `crates/liuma-agent-loop` | turn/step state machine and port traits |
| `crates/liuma-compaction` | Context compaction strategies (pure functions: threshold / retained tail / cut points never split tool pairs) |
| `crates/liuma-plan` | Plan-mode collaboration state (per-agent boolean state + `exit_plan_mode` blocking review) |
| `crates/liuma-session` | Event log (envelope / seq / message derivation; wasm32-wasip2 artifact + rlib) |
| `crates/liuma-attachment` | Attachment storage & admission (content-addressed store / image decoding / paste & drop admission chain) |
| `crates/liuma-prompt` | System prompt assembly (pure functions) |
| `crates/liuma-hooks` | Hooks bridge (Claude Code / Codex shell hooks integration) |
| `crates/liuma-mcp` | MCP client bridge (rmcp; stdio + streamable-http transports / reconnect / tool bridging) |
| `crates/liuma-decision` | Decision model integration (System One protocol / four scenario policies / `decide` tool) |
| `crates/liuma-skill` | Skill subsystem (`.agents/skills` directory loading / progressive disclosure / skill tool) |
| `crates/liuma-tools` | Tool registry and built-in tools |
| `crates/liuma-wit` | Host-side bindgen and component contract tests |
| `crates/liuma-example-tool` | Example tool component (reference implementation of the `liuma:tools` world) |
| `presets/` | Built-in capability preset manifests (standard / minimal) |
| `scripts/` | Verify and packaging scripts |

## Documentation

- [docs/design.md](docs/design.md) — design document (quality goals / architectural constraints / component & runtime views / cross-cutting concepts / glossary)
- [docs/plugin-sdk.md](docs/plugin-sdk.md) — plugin SDK
- [wit/](wit) — WIT contract definitions

## License

[MIT](LICENSE)
