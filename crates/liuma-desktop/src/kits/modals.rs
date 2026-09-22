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
use crate::kits::icons::{LiumaIcon, fixed};
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

/// 工作区菜单行组(标题栏下拉与 hero 工作区 chip 共用):工作区行
/// (文件夹 + 名 + 勾选;分支只在 StatusBar 徽标显示)+ 添加工作区。
/// 行点击 = close_all_menus + select_workspace(两处菜单同语义)
pub(crate) fn workspace_menu_rows(store: &Entity<AppStore>, cx: &App) -> Vec<gpui_kit::AnyElement> {
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
                .on_click(move |_, _, cx| {
                    let w = w.clone();
                    s.update(cx, |st, cx| {
                        st.close_all_menus(cx);
                        st.select_workspace(&w, cx);
                    });
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
            .on_click(move |_, _, cx| {
                s_add.update(cx, |st, cx| {
                    st.close_all_menus(cx);
                    st.add_workspace_via_picker(cx);
                });
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

/// 标题栏工作区下拉卡(JetBrains 式)。锚在标题栏左区(top 34 =
/// 标题栏高,left 80 = macOS 交通灯让位)
pub(crate) fn workspace_menu_card(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let card = overlay_card("ws-menu-card", 320., workspace_menu_rows(store, cx));
    // 锚右列左缘(侧栏宽随折叠态:8 边距 + 280/56 + 8 沟 + 8 内距)
    let sidebar_w = if store.read(cx).sidebar_collapsed {
        56.
    } else {
        280.
    };
    let left = sidebar_w + 1. + 8.;
    div().absolute().top(px(34.)).left(px(left)).child(card)
}

/// 重命名模态(输入 + 确认/取消;Enter 确认见 open_rename 订阅)
pub(crate) fn rename_modal(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let input = st.sessions.rename_input.clone();
    let (ok, cancel, close) = (store.clone(), store.clone(), store.clone());
    div()
        .id("rename-overlay")
        .absolute()
        .size_full()
        .top_0()
        .left_0()
        .flex()
        .items_start()
        .justify_center()
        .pt(px(160.))
        .bg(gpui_kit::Rgba {
            a: 0.6,
            ..theme::BASE()
        })
        .child(
            div()
                .v_flex()
                .w(px(420.))
                .gap(px(12.))
                .rounded(px(14.))
                .border_1()
                .border_color(theme::BORDER())
                .bg(theme::LAYER())
                .p(px(20.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(fixed(LiumaIcon::Pencil, 14.).text_color(theme::LABEL_2()))
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .child(dict::misc::rename_session()),
                        )
                        .child(div().flex_1())
                        .child(modal_close("rename-close", move |_, _, cx| {
                            close.update(cx, |st, cx| st.cancel_rename(cx));
                        })),
                )
                .children(input.map(|e| div().child(gpui_kit::component::input::Input::new(&e))))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .id("rename-ok")
                                .flex()
                                .h(px(28.))
                                .items_center()
                                .justify_center()
                                .rounded(px(14.))
                                .bg(theme::BRAND())
                                .px(px(16.))
                                .cursor_pointer()
                                .text_size(px(13.))
                                .text_color(theme::LABEL())
                                .child(dict::common::ok())
                                .on_click(move |_, _, cx| {
                                    ok.update(cx, |st, cx| st.confirm_rename(cx));
                                }),
                        )
                        .child(
                            div()
                                .id("rename-cancel")
                                .flex()
                                .h(px(28.))
                                .items_center()
                                .justify_center()
                                .rounded(px(14.))
                                .bg(theme::DOCK())
                                .px(px(16.))
                                .cursor_pointer()
                                .text_size(px(13.))
                                .text_color(theme::LABEL_2())
                                .child(dict::common::cancel())
                                .on_click(move |_, _, cx| {
                                    cancel.update(cx, |st, cx| st.cancel_rename(cx));
                                }),
                        ),
                ),
        )
}

/// 模态右上关闭钮(24px 命中区)
fn modal_close(
    id: &'static str,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .size(px(24.))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|s| s.bg(theme::DOCK()))
        .text_color(theme::LABEL_3())
        .child(fixed(IconName::Close, 14.))
        .on_click(move |ev, w, cx| on_click(ev, w, cx))
}
