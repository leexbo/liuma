## 1. 编码约束 (Coding Constraints)

- **[Error] (MUST)**: 库层强制 `thiserror`，装配层强制 `anyhow`。**(FORBIDDEN)**: 无前置保证的 `unwrap()` / `expect()`。**(ALLOWED)**: ① 守卫/构造已保证非空后的不变式断言（`expect` 须带说明前提的消息）；② 数学上不可失败的序列化。锁获取统一恢复式 `lock().unwrap_or_else(|p| p.into_inner())`，禁止 `expect("锁中毒")`——后台任务 panic 不得连坐整个进程；数据完整性由结构自身校验兜底（如 `EventLog` 的 seq 连续性守卫），不靠进程崩溃。
- **[Deps] (MUST)**: 统一锁定于 `[workspace.dependencies]`。**(FORBIDDEN)**: 未经指令授权的 `cargo update`。
- **[Wasm] (MUST)**: 100% 重放确定性。**(FORBIDDEN)**: 组件内直读系统时钟/随机数，强制通过 WASI 显式 import。
- **[FFI] (MUST)**: Linux 沙箱隔离使用纯 Rust 实现 (如 `landlock`)。**(FORBIDDEN)**: 引入任何 C 交付物。
- **[Docs] (MUST)**: `liuma-host` 与 `liuma-wit` 公开 API 强制 `#![deny(missing_docs)]`。
- **[GPUI] (MUST)**: UI 开发前强制挂载 `gpui-kit` 与 `gpui-kit-design-guides` 上下文。**(FORBIDDEN)**: 手搓已有基础控件。弹层 (Popover/Tooltip) 强制遵守：`根级渲染 (Root-render) + 锚定计算 (Anchor) + 遮蔽打断 (Occlude)`。

## 2. CI/CD 测试门禁 (Test Gates)

执行管线：`fmt-check` -> `clippy` -> `test` -> `wit` -> `component-contracts` -> `e2e` -> `links`。

- **[L1-Unit]**: 纯逻辑校验（编解码 / 状态机 / 配置合并）。
- **[L2-Contract]**: `tests/` 目录通过 Wasmtime 真实实例化，验证 World 语义、不变式与取消逻辑。
- **[L3-E2E]**: Mock Provider 驱动全链路，断言日志 Bit-exact 可重放。
- **[Regression] (MUST)**: 任何行为修复 (Fix) **必须** 携带回归锁测试 (Regression Test)。
- **[Flaky] (FORBIDDEN)**: 严禁使用 `#[ignore]` 或跳过 (Skip) 机制掩盖偶发失败，必须修复根因。
- **[Platform] (MUST)**: 平台差异用 `#[cfg]` 门控或平台条件断言表达；运行时早退只允许出现在「已显式检测到的环境事实」分支，且该分支本身必须有断言（「探测不到就 return」= 假绿，属 Flaky 禁令覆盖范围）。

### 平台前置（just preflight 自检）

`just verify` 在 macOS / Linux / Windows 上都应全绿。环境缺件是环境问题，不得表述为通过。

- **真 Python 3**：Microsoft Store 的 `python3` 只是应用执行别名（命令存在、运行即退出 49），`just links`（`scripts/verify-links`）与 liuma-mcp / liuma-core 的 Python fixture 都要真解释器 —— `winget install --id Python.Python.3.13`。解释器名按平台分叉：Windows 用 `python`（官方安装器不产 `python3.exe`），Unix 用 `python3`。
- **`wasm32-wasip2` 目标**（`rustup target add wasm32-wasip2`）：组件契约测试要真编译 wasm 组件。
- **`just`**、**`bash`**（Git for Windows 自带）；**`pwsh`** 是 Windows 侧 shell 工具的运行时。
- **行尾**由 `.gitattributes` 固定（文本 = LF，`*.ps1/*.cmd/*.bat` = CRLF）；不要用 `core.autocrlf` 把它改回去 —— 工作树里的 `\r` 会被当成实参的一部分传给脚本与 cargo。

## 3. Git 交付契约 (Commit Protocol)

- **[Format] (MUST)**: 遵循 Conventional Commits 规范 `<type>(<scope>): <subject>`。
    - `type` ∈ `feat` | `fix` | `docs` | `refactor` | `perf` | `test` | `chore`。
    - `subject`: 英文祈使句 (如 `add xyz` / `fix abc`)，**结尾不加句号**。
    - `body`: 说明动机 (Motivation) 与机制 (Mechanism)。若是 `fix`，需明确指出回归锁所在文件及用例。
- **[Signature] (MUST)**: Commit 提交信息末行强制追加 Agent 签名。
    - 格式: `Co-Authored-By: <当前模型名> <bot@liuma-agent.local>`
    - (例:  `Co-Authored-By: GLM 5.3 <bot@liuma-agent.local>`)