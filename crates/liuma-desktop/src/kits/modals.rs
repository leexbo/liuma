//! 模态/菜单 chrome(无状态组件):菜单行、工作区/会话菜单卡片、
//! 重命名与删除确认模态、overlay 与 toast 卡片、关闭钮;由 shell
//! 底座的 WorkspaceView 渲染与 hero 态装配使用。

use gpui_kit::component::{IconName, StyledExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Entity, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, div, px,
};

use crate::kits::i18n::dict;
use crate::kits::icons::fixed;
use crate::kits::theme;
use crate::shell::store::AppStore;

/// 附件拒收 toast(底部居中 + 关闭钮)。
/// 点击任一位置关闭;3s 自动消失由 store 定时清理兜底。
pub(crate) fn attachment_toast_card(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let Some(toast) = st.attachments.attachment_toast.clone() else {
        return div().into_any_element();
    };
    let close_store = store.clone();
    div()
        .id("attachment-toast")
        .debug_selector(|| "attachment-toast".to_string())
        .absolute()
        .size_full()
        .top_0()
        .left_0()
        .flex()
        .items_end()
        .justify_center()
        .pb(px(120.))
        .on_click(move |_, _, cx| {
            close_store.update(cx, |st, _| st.attachments.attachment_toast = None);
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(12.))
                .bg(gpui_kit::Rgba {
                    a: 0.95,
                    ..theme::LAYER()
                })
                .border_1()
                .border_color(theme::BORDER())
                .px(px(14.))
                .py(px(10.))
                .text_size(px(13.))
                .text_color(theme::LABEL())
                .child(fixed(IconName::TriangleAlert, 14.))
                .child(toast.text.clone()),
        )
        .into_any_element()
}

/// 工作区菜单行组(标题栏下拉与 hero 工作区 chip 共用;组件库
/// Popover 内容):工作区行(文件夹 + 名 + 勾选;分支只在 StatusBar
/// 徽标显示)+ 添加工作区。行点击 = 收起弹层 + select_workspace
pub(crate) fn workspace_menu_rows(
    store: &Entity<AppStore>,
    pop: gpui_kit::Entity<gpui_kit::component::popover::PopoverState>,
    cx: &App,
) -> Vec<gpui_kit::AnyElement> {
    let st = store.read(cx);
    let active = st
        .state
        .active_workspace
        .clone()
        .unwrap_or_else(|| st.default_workspace());
    let mut rows: Vec<gpui_kit::AnyElement> = vec![];
    for (ix, ws) in st.state.host_info.workspaces.iter().enumerate() {
        let s = store.clone();
        let w = ws.clone();
        let is_active = *ws == active;
        let pop = pop.clone();
        let sel = format!("ws-row-{w}");
        rows.push(
            div()
                .id(("ws-item", ix))
                .flex()
                .h(px(32.))
                .items_center()
                .gap(px(8.))
                .rounded(px(6.))
                .px(px(8.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::DOCK()))
                .text_size(px(13.))
                .text_color(if is_active {
                    theme::LABEL()
                } else {
                    theme::LABEL_2()
                })
                .child(fixed(
                    if is_active {
                        IconName::FolderOpen
                    } else {
                        IconName::FolderClosed
                    },
                    15.,
                ))
                .child(w.clone())
                .child(div().flex_1())
                .when(is_active, |el| {
                    el.child(fixed(IconName::Check, 14.).text_color(theme::LABEL()))
                })
                // 测试钩子(release 空操作)
                .debug_selector(move || sel.clone())
                .on_click(move |_, window, cx| {
                    let pop = pop.clone();
                    let w = w.clone();
                    pop.update(cx, |state, cx| state.dismiss(window, cx));
                    s.update(cx, |st, cx| st.select_workspace(&w, cx));
                })
                .into_any_element(),
        );
    }
    rows.push(
        div()
            .h(px(1.))
            .my(px(2.))
            .bg(theme::BORDER())
            .into_any_element(),
    );
    let s_add = store.clone();
    rows.push(
        div()
            .id("ws-add")
            .flex()
            .h(px(30.))
            .items_center()
            .gap(px(8.))
            .rounded(px(6.))
            .px(px(8.))
            .cursor_pointer()
            .hover(|s| s.bg(theme::DOCK()))
            .text_size(px(13.))
            .text_color(theme::LABEL_2())
            .child(fixed(IconName::Plus, 14.))
            .child(dict::misc::add_workspace_ellipsis())
            .on_click(move |_, window, cx| {
                let pop = pop.clone();
                pop.update(cx, |state, cx| state.dismiss(window, cx));
                s_add.update(cx, |st, cx| st.add_workspace_via_picker(cx));
            })
            .into_any_element(),
    );
    rows
}

/// 下拉卡容器(工作区/hero 菜单共用形态;整卡 mousedown 豁免)
pub(crate) fn overlay_card(
    id: &'static str,
    width: f32,
    rows: Vec<gpui_kit::AnyElement>,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    use gpui_kit::MouseButton;
    div()
        .id(id)
        .v_flex()
        .w(px(width))
        .gap(px(2.))
        .rounded(px(12.))
        .border_1()
        .border_color(theme::BORDER())
        // 浅盘白浮层(同 composer menu_card;LAYER 是画布 hover 语言)
        .bg(if theme::is_dark() {
            theme::LAYER()
        } else {
            theme::CARD()
        })
        .p(px(4.))
        .shadow_md()
        .children(rows)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

// 重命名模态已迁组件库 Dialog(sessions/store.rs open_rename 经
// AppStore::with_window 桥打开;输入/确认/取消与 Enter 订阅不变)。
