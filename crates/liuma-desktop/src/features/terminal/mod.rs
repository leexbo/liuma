//! 终端功能切片(右栏「终端」标签):PTY + alacritty_terminal 仿真 +
//! gpui 网格渲染,类 VS Code 集成终端(Zed 同架构路线)。
//!
//! 本模块当前仅含纯逻辑层(按键编码 / 网格调色);会话与视图接线随后
//! 续提交落地。

pub(crate) mod input;
pub(crate) mod palette;
