//! 设置功能域:按导航页拆分,每页一个模块(方法 + 视图同文件)。
//!
//! 页面:general(常规)/ provider(模型设置,含计费)/ mcp / hooks /
//! decision / archived(归档聊天);跨页:`state`(切片状态/枚举/解析)、
//! `shared`(字段行族等辅助)、`views`(render 两栏壳与侧栏菜单)、
//! `onboarding`/`full_access`(首启与全权确认弹层)。
//! `mod.rs` 持有全量导入(`pub(crate) use`,子模块经 `use super::*`
//! 取用)与子模块 glob 再导出——外部 `features::settings::X` 路径不变。

pub(crate) mod archived;
pub(crate) mod decision;
pub(crate) mod full_access;
pub(crate) mod general;
pub(crate) mod hooks;
pub(crate) mod mcp;
pub(crate) mod onboarding;
pub(crate) mod provider;
pub(crate) mod shared;
pub(crate) mod state;
pub(crate) mod views;

// ── 共享导入(子模块 `use super::*` 的取用面) ──
pub(crate) use std::collections::{HashMap, HashSet};

pub(crate) use crate::kits::fmt::fmt_clock_md;
pub(crate) use crate::kits::i18n::{self, t};
pub(crate) use crate::kits::icons::{LiumaIcon, fixed};
pub(crate) use crate::kits::popup::PopTrigger;
pub(crate) use crate::kits::theme;
pub(crate) use crate::shell::store::AppStore;
pub(crate) use gpui_kit::AppContext;
pub(crate) use gpui_kit::component::IconName;
pub(crate) use gpui_kit::component::IndexPath;
pub(crate) use gpui_kit::component::InteractiveElementExt as _;
pub(crate) use gpui_kit::component::Sizable;
pub(crate) use gpui_kit::component::StyledExt;
pub(crate) use gpui_kit::component::button::{Button, ButtonVariants as _};
pub(crate) use gpui_kit::component::input::{EditorState, InputEvent, MaskPattern};
pub(crate) use gpui_kit::component::input::{Input, InputState};
pub(crate) use gpui_kit::component::popover::{Popover, PopoverState};
pub(crate) use gpui_kit::component::radio::{Radio, RadioGroup};
pub(crate) use gpui_kit::component::select::{Select, SelectEvent, SelectState};
pub(crate) use gpui_kit::component::switch::Switch;
pub(crate) use gpui_kit::prelude::FluentBuilder as _;
pub(crate) use gpui_kit::{
    Anchor, App, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px,
};
pub(crate) use liuma_core::proto::ArchivedSessionSummary;

// ── 子模块再导出(外部 features::settings::X 与跨文件调用的可见面) ──
pub(crate) use archived::*;
pub(crate) use decision::*;
pub(crate) use full_access::*;
pub(crate) use general::*;
pub(crate) use hooks::*;
pub(crate) use mcp::*;
pub(crate) use onboarding::*;
pub(crate) use provider::*;
pub(crate) use shared::*;
pub(crate) use state::*;
pub(crate) use views::*;
