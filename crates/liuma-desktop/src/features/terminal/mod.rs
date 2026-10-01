//! 终端功能切片(右栏「终端」标签):PTY + alacritty_terminal 仿真 +
//! gpui 网格渲染,类 VS Code 集成终端(Zed 同架构路线)。
//!
//! v1 范围:单会话长驻(tab 存活期),全屏 TUI/方向键/Ctrl-C/滚轮回看/
//! bracketed paste。范围外:选区复制、搜索、多 tab、分屏、IME(留
//! InputHandler 接缝)、鼠标事件编码、kitty keyboard protocol、
//! Windows 验证(ConPTY 随沙箱阶段 4)。子模块:input(按键编码)、
//! palette(网格→runs)、store(会话与线程边界)、views(视图)。

pub(crate) mod input;
pub(crate) mod palette;
pub(crate) mod store;
pub(crate) mod views;

pub(crate) use views::render;
