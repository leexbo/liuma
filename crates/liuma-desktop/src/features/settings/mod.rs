//! 设置功能切片:独立页(render + 两栏壳)与 provider CRUD 表单、
//! onboarding,另含侧栏设置模式菜单(menu;自 ui::sidebar 切出)与
//! 全权确认/删除确认/拉取模型三弹层(Dialog 层,store 桥开)。

pub(crate) mod store;
mod views;

pub(crate) use store::{FullAccessAsk, SettingsNav, SettingsStore};
pub(crate) use views::{
    menu, onboarding_modal, open_delete_provider_dialog, open_fetch_models_dialog,
    open_full_access_dialog, render, settings_row, usage_bar,
};
