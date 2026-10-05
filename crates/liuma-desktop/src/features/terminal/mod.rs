//! 终端功能切片(右栏「终端」标签):PTY + alacritty_terminal 仿真 +
//! gpui 网格渲染,类 VS Code 集成终端(Zed 同架构路线)。
//!
//! 范围:多标签长驻会话(一路标签 = 一路 shell)、OSC Title 接标签
//! 文案、拖选/双击取词/三击取行 + ⌘C/右键复制、滚轮回看、全屏 TUI/
//! 方向键/Ctrl-C/bracketed paste。范围外:搜索、分屏、IME(留
//! InputHandler 接缝)、鼠标事件编码(接后需按 TermMode 门控选区)、
//! kitty keyboard protocol、Windows 验证(ConPTY 随沙箱阶段 4)。
//! 子模块:input(按键编码)、palette(网格→runs)、store(会话与
//! 线程边界)、views(视图)。

pub(crate) mod input;
pub(crate) mod palette;
pub(crate) mod store;
pub(crate) mod views;

use gpui_kit::actions;

actions!(terminal, [CopyTerminalSelection]);

pub(crate) use views::render;
