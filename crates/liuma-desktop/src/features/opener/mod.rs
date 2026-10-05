//! 「在编辑器中打开」切片:顶栏分体钮(选中应用直开工作区目录)+
//! 应用菜单(本机探测,真身图标,选中记忆)。参照 dsh open-in-app
//! 的交互语义:清单 = 编译期白名单 + 存在性探测(访达/终端 fixed
//! 恒在);启动 = `open -a`/`open`/`xed` 分型;非 macOS 与无工作区
//! 目录时控件整体不渲染。

pub(crate) mod catalog;
pub(crate) mod probe;
pub(crate) mod store;
mod views;

pub(crate) use store::OpenerStore;
pub(crate) use views::{launch_error_toast, open_with_split, split_button_visible};
