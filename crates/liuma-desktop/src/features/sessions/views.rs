//! 侧栏:会话列表(280px;收起完全隐藏)。
//! 按工作区分组(前缀推导),支持本地搜索过滤、新建、切换。

use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::Icon;
use gpui_kit::component::IconName;
use gpui_kit::component::InteractiveElementExt as _;
use gpui_kit::component::StyledExt;
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, App, Entity, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, StatefulInteractiveElement, Styled, div, px,
};
use liuma_core::proto::SessionSummary;

use crate::features::search;
use crate::features::sessions::store::{GroupMode, OrderMode};
use crate::features::settings;
use crate::kits::i18n::t;
use crate::kits::icons::{LiumaIcon, fixed};
use crate::kits::popup::PopTrigger;
use crate::kits::theme;
use crate::shell::reducer::{relative_time, workspace_of};
use crate::shell::store::AppStore;

/// 侧栏整体(展开 280px 胶囊卡;收起完全隐藏不渲染——
/// 折叠不再保留 56px rail,展开入口仅标题栏缩进钮)。
/// 设置模式 = 侧栏切换为设置菜单:
/// 标题 + 通用/模型/关于 导航项 + 底部「返回」行。
///
/// 展开态右缘挂拖宽把手,宽存于 `sidebar_px`。
pub fn render(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    if store.read(cx).sidebar_collapsed {
        return div().into_any_element();
    }
    if store.read(cx).settings.settings_open {
        return settings::menu(store, cx).into_any_element();
    }
    let st = store.read(cx);
    // 拖宽锚点在场:窗口级 move/up 经 canvas.paint(Paint 相位)注册,非 render
    // (render 在 Prepaint 相位跑,on_mouse_event 会 panic;canvas.paint 每帧
    // 重跑,指针移出把手仍收拖动事件,无指针捕获的 GPUI 惯例)。
    // 锚点为 None 时每次 paint 仍注册但内层早退(no-op),开销可忽略;结束
    // 后下一帧 anchor 清空,不再拖动。
    let drag_active = st.sidebar_resize_anchor.is_some();
    let mut body = div()
        .v_flex()
        .h_full()
        .w(crate::shell::metrics::sidebar_width_for(
            false,
            st.sidebar_px,
        ))
        .flex_shrink_0()
        // 扁平面板(去胶囊卡):SIDEBAR 色阶 + 右缘发丝线与内容区分界;
        // 组件层(行 hover/搜索框)浮于其上
        .bg(theme::SIDEBAR())
        .border_r_1()
        .border_color(theme::BORDER())
        // 测试钩子:布局回归断言双栏分离(release 空操作)
        .debug_selector(|| "sidebar-card".to_string())
        .px(px(12.))
        // 顶部让位:macOS 交通灯浮于卡上(方案 B 全高侧栏);该区为
        // 窗口拖拽条(见 drag_strip)
        .pt(px(36.))
        .pb(px(10.))
        .gap(px(8.))
        .child(drag_strip())
        // 顶部序:「新会话」全局钮最顶,其下为顶栏二态
        // (搜索关 = 「工作区」标题 + 三图标钮;开 = 搜索框)
        .child(new_session_row(store))
        .child(if st.search.search_open {
            search::search_field(store, cx).into_any_element()
        } else {
            header_row(store, cx).into_any_element()
        })
        .when(store.read(cx).search.search_hits.is_some(), |el| {
            el.child(search::search_hits_panel(store, cx))
        })
        .when(store.read(cx).search.search_hits.is_none(), |el| {
            el.child(session_list(store, cx))
        })
        .child(settings::settings_row(store))
        .child(sidebar_resize_handle(store));
    // 拖宽进行中:整窗 canvas 覆盖层负责窗口级 move/up 注册(Paint 相位)。
    if drag_active {
        body = body.child(drag_overlay(store));
    }
    body.into_any_element()
}

/// 右缘竖向拖宽把手(8px 命中区,右缘外沿)。折叠态
/// 不渲染。on_mouse_down 在 paint 相位
/// 注册(div 惯例),开始拖拽只置锚点;move/up 由 [`drag_overlay`] 接管。
fn sidebar_resize_handle(store: &Entity<AppStore>) -> impl IntoElement {
    let s = store.clone();
    div()
        .id("sidebar-resize")
        .debug_selector(|| "sidebar-resize".to_string())
        .absolute()
        // 右缘外侧 4px,命中区 8px 覆盖边界
        .right(px(-4.))
        .top_0()
        .bottom_0()
        .w(px(8.))
        .cursor(gpui_kit::CursorStyle::ResizeLeftRight)
        .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            s.update(cx, |st, cx| {
                st.sidebar_resize_begin(f32::from(ev.position.x), cx)
            });
        })
}

/// 拖宽进行中的窗口级 move/up 注册(Paint 相位):canvas.paint 每帧重跑,
/// 指针移出把手仍收拖动事件;锚点为 None 时内层早退(no-op)。
fn drag_overlay(store: &Entity<AppStore>) -> impl IntoElement {
    let m = store.clone();
    let u = store.clone();
    gpui_kit::canvas(
        // prepaint:无自定义绘制
        |_, _, _| (),
        move |_, _, window, cx| {
            // 锚点在场才注册移动;清空后早退(闭包仍每帧跑,paint 相位合法)
            if m.read(cx).sidebar_resize_anchor.is_some() {
                let m2 = m.clone();
                window.on_mouse_event(move |ev: &MouseMoveEvent, _, _, cx| {
                    m2.update(cx, |st, cx| {
                        st.sidebar_resize_move(f32::from(ev.position.x), cx)
                    });
                });
            }
            let u2 = u.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, _, _, cx| {
                u2.update(cx, |st, cx| st.sidebar_resize_end(cx));
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
    .into_any_element()
}

/// 卡顶拖拽条(方案 B:交通灯浮于全高侧栏卡上,顶部让位区兼作窗口
/// 拖拽/双击缩放;区域内无交互子元素,mousedown 即拖)
pub(crate) fn drag_strip() -> impl IntoElement {
    div()
        .id("sidebar-drag-strip")
        .debug_selector(|| "sidebar-drag-strip".to_string())
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(34.))
        .cursor(gpui_kit::CursorStyle::Arrow)
        .on_mouse_down(MouseButton::Left, |_, window, _| {
            window.start_window_move();
        })
        .on_double_click(|_, window, _| {
            window.titlebar_double_click();
        })
}

/// 「新会话」行(全局唯一;⊕ 图标 + 文字居中)
fn new_session_row(store: &Entity<AppStore>) -> impl IntoElement {
    let s = store.clone();
    div().flex().h(px(36.)).items_center().child(
        div()
            .id("new-session")
            .debug_selector(|| "new-session".to_string())
            .flex()
            .flex_1()
            .h(px(36.))
            .items_center()
            .justify_center()
            .gap(px(6.))
            .rounded(px(12.))
            .border_1()
            .border_color(theme::BORDER())
            .bg(theme::LAYER())
            .cursor_pointer()
            .text_size(px(13.))
            .font_weight(gpui_kit::FontWeight::MEDIUM)
            .text_color(theme::LABEL())
            .hover(|s| s.bg(theme::DOCK()))
            .child(fixed(LiumaIcon::NewChat, 14.))
            .child(t!("sessions.new_session"))
            .on_click(move |_, _, cx| {
                s.update(cx, |st, cx| st.create_session(cx));
            }),
    )
}

/// 侧栏顶栏:「工作区」标题(单列表态显示「会话」)+ 右缘三个圆形
/// hover 图标钮(搜索 / 视图选项 / 添加工作区)。头部空白处 mousedown
/// 即拖(兼窗口拖拽条,同 panel_header 约定:交互子件自挂 mousedown
/// stop_propagation,点击不触发窗口拖拽)
fn header_row(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let flat = store.read(cx).sessions.group_mode == GroupMode::Flat;
    let (s_search, s_view, s_add) = (store.clone(), store.clone(), store.clone());
    div()
        .id("sidebar-header")
        .debug_selector(|| "sidebar-header".to_string())
        .flex()
        .h(px(28.))
        .flex_shrink_0()
        .items_center()
        .pl(px(4.))
        .on_mouse_down(MouseButton::Left, |_, window, _| {
            window.start_window_move();
        })
        .on_double_click(|_, window, _| {
            window.titlebar_double_click();
        })
        .child(
            div()
                .text_size(px(13.))
                .text_color(theme::LABEL_3())
                .child(if flat {
                    t!("sessions.group_flat")
                } else {
                    t!("sessions.group_by_ws")
                }),
        )
        .child(div().flex_1())
        .child(
            header_icon_button(
                "search",
                t!("sessions.search_ph"),
                fixed(LiumaIcon::SearchOutline, 14.),
            )
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                s_search.update(cx, |st, cx| st.toggle_search_open(window, cx));
            }),
        )
        .child({
            // 视图选项菜单(组件库 Popover 托管定位/外点关闭)。受控
            // 开态:钮在侧栏头拖拽区上,mousedown 豁免不可去(头行为
            // 窗口拖拽区),库内部开态收不到点击
            let open = store.read(cx).sessions.view_menu_open;
            Popover::new("view-menu-pop")
                .appearance(false)
                .anchor(Anchor::TopRight)
                .open(open)
                .on_open_change({
                    let s_open = s_view.clone();
                    move |open, _, cx| {
                        s_open.update(cx, |st, _| st.sessions.view_menu_open = *open);
                    }
                })
                .trigger({
                    let s_click = s_view.clone();
                    PopTrigger(
                        header_icon_button(
                            "view-options",
                            t!("sessions.view_options"),
                            fixed(LiumaIcon::Personalization, 15.),
                        )
                        .on_click(move |_, _, cx| {
                            s_click.update(cx, |st, cx| st.toggle_view_menu(cx));
                        }),
                    )
                })
                .content({
                    let s_view = s_view.clone();
                    move |_, _, cx| {
                        let pop = cx.entity();
                        view_options_menu_card(&s_view, pop, cx).into_any_element()
                    }
                })
        })
        .child(
            header_icon_button(
                "add-workspace",
                t!("sessions.add_workspace"),
                fixed(LiumaIcon::ProjectAdd, 16.),
            )
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                s_add.update(cx, |st, cx| st.add_workspace_via_picker(cx));
            }),
        )
}

/// 顶栏图标钮(圆形 hover 底;tooltip 走组件库 `.tooltip()` 托管)。
/// 含 mousedown 豁免(头行为窗口拖拽区,钮点击不得触发拖窗)
/// 侧栏头小图标钮:`sel` = 稳定 ASCII selector,`tip` = 可见 tooltip。
///
/// selector 与 tip 分离:tip 随语言切换,selector 必须恒定(旧实现拿中文
/// tip 拼 selector,测试也按中文寻址)。
fn header_icon_button(
    sel: &'static str,
    tip: impl Into<gpui_kit::SharedString>,
    icon: Icon,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let tip = tip.into();
    let id: gpui_kit::SharedString = format!("header-btn-{sel}").into();
    let sel = id.clone();
    div()
        .id(id)
        .debug_selector(move || sel.to_string())
        .relative()
        .flex()
        .size(px(26.))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .ml(px(2.))
        .rounded_full()
        .cursor_pointer()
        .text_color(theme::LABEL_3())
        .hover(|s| s.bg(theme::LAYER()).text_color(theme::LABEL_2()))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .tooltip(crate::shell::tip(tip))
        .child(icon)
}

/// 侧栏组内会话预览条数(超出折叠为「展开显示」行)
const GROUP_PREVIEW: usize = 5;

/// 侧栏小节标签(「置顶」/「项目」;12px 三级色,同参照布局的节头)
fn section_label(text: impl Into<gpui_kit::SharedString>) -> gpui_kit::AnyElement {
    let text = text.into();
    div()
        .flex_shrink_0()
        .pt(px(6.))
        .pb(px(2.))
        .px(px(8.))
        .text_size(px(12.))
        .text_color(theme::CAPTION())
        .child(text)
        .into_any_element()
}

/// 组内会话行 + 预览截断(>GROUP_PREVIEW 且未展开 → 前 N 条 +
/// 「展开显示」行;展开 → 全量 + 「收起显示」行;搜索时恒全量)
fn preview_rows(
    store: &Entity<AppStore>,
    cx: &App,
    ws: &str,
    idxs: Vec<usize>,
    query: &str,
) -> Vec<gpui_kit::AnyElement> {
    preview_rows_in(store, cx, ws, idxs, query, "sess")
}

/// 同 [`session_row_in`]:ns 透传给行 hover 组
fn preview_rows_in(
    store: &Entity<AppStore>,
    cx: &App,
    ws: &str,
    idxs: Vec<usize>,
    query: &str,
    ns: &str,
) -> Vec<gpui_kit::AnyElement> {
    let st = store.read(cx);
    let extra = st.sessions.expanded_show.get(ws).copied().unwrap_or(0);
    // 分页渐进:预览 5 条起步,每点一次「展开显示」多显一批(5 条);
    // 搜索时恒全量(与折叠纪律同源)
    let shown = if query.is_empty() {
        (GROUP_PREVIEW * (1 + extra)).min(idxs.len())
    } else {
        idxs.len()
    };
    let mut rows: Vec<gpui_kit::AnyElement> = idxs[..shown]
        .iter()
        .map(|ix| session_row_in(store, cx, &st.state.sessions[*ix], *ix, ns).into_any_element())
        .collect();
    if shown < idxs.len() {
        let hidden = idxs.len() - shown;
        rows.push(show_more_row(store, ws, false, hidden).into_any_element());
    } else if query.is_empty() && extra > 0 {
        rows.push(show_more_row(store, ws, true, 0).into_any_element());
    }
    rows
}

/// 组空状态行(「暂无聊天」):组内无可见会话时的占位(会话全上提
/// 置顶或本无会话);几何同 show_more_row,灰字不可交互
fn group_empty_row(ws: &str) -> impl IntoElement {
    div()
        .debug_selector(move || format!("ws-empty-{ws}"))
        .flex()
        .h(px(28.))
        .flex_shrink_0()
        .items_center()
        .rounded(px(8.))
        .px(px(8.))
        .ml(px(22.))
        .text_size(px(12.))
        .text_color(theme::CAPTION())
        .child(t!("sessions.empty_chats"))
}

/// 「展开显示 / 收起显示」行(灰字;点击切换该组展开态)
fn show_more_row(
    store: &Entity<AppStore>,
    ws: &str,
    expanded: bool,
    hidden: usize,
) -> impl IntoElement {
    let s = store.clone();
    let ws = ws.to_string();
    let ws_sel = ws.clone();
    div()
        .id(gpui_kit::SharedString::from(format!("ws-show-more-{ws}")))
        .debug_selector(move || format!("ws-show-more-{ws_sel}"))
        .flex()
        .h(px(28.))
        .flex_shrink_0()
        .items_center()
        .rounded(px(8.))
        .px(px(8.))
        .ml(px(22.))
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(theme::CAPTION())
        .hover(|s| s.bg(theme::SIDEBAR_HOVER()).text_color(theme::LABEL_2()))
        .child(if expanded {
            t!("sessions.collapse_show").to_string()
        } else {
            t!("sessions.expand_show_more", hidden = hidden).into_owned()
        })
        .on_click(move |_, _, cx| {
            let ws = ws.clone();
            s.update(cx, |st, cx| {
                if expanded {
                    st.collapse_workspace(&ws, cx);
                } else {
                    st.expand_workspace_more(&ws, cx);
                }
            });
        })
}

/// 置顶会话行(气泡图标 + 标题;点击打开会话)
fn pinned_session_row(store: &Entity<AppStore>, cx: &App, s: &SessionSummary) -> impl IntoElement {
    let st = store.read(cx);
    let active = st.state.current_id.as_deref() == Some(&s.session_id);
    let title = st.title_for(&s.session_id);
    let running = st.is_running(&s.session_id);
    let pinned = st
        .sessions
        .pinned_sessions
        .iter()
        .any(|v| v == &s.session_id);
    let target = store.clone();
    let pin_store = store.clone();
    let archived = store.clone();
    let id = s.session_id.clone();
    let pin_id = id.clone();
    let arch_id = id.clone();
    let sel = format!("pinned-session-{id}");
    let sel_pin = format!("pinned-session-pin-{id}");
    let sel_arch = format!("pinned-session-archive-{id}");
    let grp = format!("pin-sess-grp-{id}");
    div()
        .id(gpui_kit::SharedString::from(format!("pinned-{id}")))
        .debug_selector(move || sel.clone())
        .relative()
        .group(grp.clone())
        .flex()
        .h(px(34.))
        .flex_shrink_0()
        .items_center()
        .rounded(px(8.))
        .when(active, |el| el.bg(theme::SIDEBAR_ACTIVE()))
        .pl(px(8.))
        .pr(px(52.))
        .gap(px(8.))
        .cursor_pointer()
        .hover(|s| s.bg(theme::SIDEBAR_HOVER()))
        .child(
            div()
                .flex()
                .w(px(14.))
                .flex_shrink_0()
                .justify_center()
                .text_color(theme::LABEL_3())
                .children(
                    running
                        .then(running_dot)
                        .or_else(|| Some(fixed(LiumaIcon::Message, 13.).into_any_element())),
                ),
        )
        .child(
            div()
                .flex()
                .min_w(px(0.))
                .flex_1()
                .truncate()
                .text_size(px(13.))
                .text_color(if active {
                    theme::LABEL()
                } else {
                    theme::LABEL_2()
                })
                .child(title),
        )
        // 尾部槽(absolute,同 session_row 形制):置顶 + 归档双钮,行
        // hover 同显,悬停钮白色高亮 + tooltip。恒 Pin 图标中性灰,状
        // 态由 tooltip 文案区分
        .child(
            div()
                .absolute()
                .right_0()
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .gap(px(2.))
                .px(px(4.))
                .rounded(px(6.))
                .opacity(0.)
                .group_hover(grp.clone(), |s| s.opacity(1.))
                .child(
                    div()
                        .id(gpui_kit::SharedString::from(format!("pinned-pin-{id}")))
                        .debug_selector(move || sel_pin.clone())
                        .flex()
                        .size(px(20.))
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::SIDEBAR_ACTIVE()).text_color(theme::LABEL()))
                        .text_color(theme::CAPTION())
                        .child(fixed(
                            if pinned {
                                LiumaIcon::PinOff
                            } else {
                                LiumaIcon::Pin
                            },
                            14.,
                        ))
                        .tooltip(crate::shell::tip(if pinned {
                            t!("sessions.unpin")
                        } else {
                            t!("sessions.pin")
                        }))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            let id = pin_id.clone();
                            pin_store.update(cx, |st, cx| st.toggle_pinned_session(&id, cx));
                        }),
                )
                .child(
                    div()
                        .id(gpui_kit::SharedString::from(format!("pinned-arch-{id}")))
                        .debug_selector(move || sel_arch.clone())
                        .flex()
                        .size(px(20.))
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::SIDEBAR_ACTIVE()).text_color(theme::LABEL()))
                        .text_color(theme::CAPTION())
                        .child(fixed(LiumaIcon::Archive, 14.))
                        .tooltip(crate::shell::tip(t!("sessions.tip_archive")))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            let id = arch_id.clone();
                            archived.update(cx, |st, cx| st.archive(&id, cx));
                        }),
                ),
        )
        .on_click(move |_, _, cx| {
            let id = id.clone();
            target.update(cx, |st, cx| st.open_session(&id, cx));
        })
}

/// 置顶工作区块:文件夹行(点击切工作区)+ 其会话清单(5 条预览纪律)
fn pinned_workspace_block(
    store: &Entity<AppStore>,
    cx: &App,
    ws: &str,
    groups: &[(String, Vec<usize>)],
) -> impl IntoElement {
    let st = store.read(cx);
    // 选中态 = 显式选择(active_workspace 仅由点击工作区行/下拉设置,
    // 互斥由 store 写入端保证:选中工作区即清会话选中,见
    // select_workspace 文档);与置顶会话气泡的激活互不点亮
    let selected = st.state.active_workspace.as_deref() == Some(ws);
    let display = st.title_for_workspace(ws);
    let idxs = groups
        .iter()
        .find(|(k, _)| k == ws)
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let target = store.clone();
    let target_edit = store.clone();
    let ws_sel = ws.to_string();
    let ws = ws.to_string();
    let ws_menu = ws.clone();
    let ws_edit = ws.clone();
    let ws_rows = ws.clone();
    let pin_grp = format!("pinws-grp-{ws}");
    div()
        .debug_selector(move || format!("pinned-ws-{ws_sel}"))
        .v_flex()
        .flex_shrink_0()
        .gap(px(2.))
        .child(
            div()
                .id(gpui_kit::SharedString::from(format!("pinned-ws-row-{ws}")))
                .group(pin_grp.clone())
                .flex()
                .h(px(34.))
                .flex_shrink_0()
                .items_center()
                .rounded(px(8.))
                .when(selected, |el| el.bg(theme::SIDEBAR_ACTIVE()))
                .px(px(8.))
                .gap(px(8.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::SIDEBAR_HOVER()))
                .text_color(if selected {
                    theme::LABEL()
                } else {
                    theme::LABEL_2()
                })
                .child(fixed(LiumaIcon::FolderClose, 14.).text_color(if selected {
                    theme::BRAND()
                } else {
                    theme::LABEL_3()
                }))
                .child(
                    div()
                        .flex()
                        .min_w(px(0.))
                        .flex_1()
                        .truncate()
                        .text_size(px(13.))
                        .child(display),
                )
                // 行内弹性空区:推钮贴右缘 + 信息卡悬停源
                .child(
                    div()
                        .id(gpui_kit::SharedString::from(format!(
                            "pinned-ws-info-hover-{ws}"
                        )))
                        .flex_1()
                        .h_full()
                        .on_hover({
                            let s_hover = target_edit.clone();
                            let ws_hover = ws_edit.clone();
                            move |hovering: &bool, window, cx| {
                                let ws = ws_hover.clone();
                                s_hover.update(cx, |st, cx| {
                                    st.set_ws_info_hover_row(
                                        hovering.then(|| ws.clone()),
                                        window.mouse_position(),
                                        cx,
                                    );
                                });
                            }
                        }),
                )
                // hover ⋯:整理菜单(点击;悬停 ⋯ 不出信息卡)
                .child(
                    Popover::new(gpui_kit::SharedString::from(format!(
                        "pinned-ws-menu-pop-{ws}"
                    )))
                    .appearance(false)
                    .anchor(Anchor::TopRight)
                    .trigger(PopTrigger(
                        div()
                            .id(gpui_kit::SharedString::from(format!("pinned-ws-menu-{ws}")))
                            .flex()
                            .size(px(20.))
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .opacity(0.)
                            .group_hover(pin_grp.clone(), |s| s.opacity(1.))
                            .hover(|s| s.bg(theme::SIDEBAR_ACTIVE()).text_color(theme::LABEL()))
                            .text_color(theme::CAPTION())
                            .tooltip(crate::shell::tip(t!("sessions.ws_actions")))
                            .child(fixed(IconName::Ellipsis, 14.)),
                    ))
                    .content({
                        let s_menu = store.clone();
                        let ws_menu = ws_menu.clone();
                        move |_, _, cx| {
                            let pop = cx.entity();
                            ws_menu_card(&s_menu, cx, &ws_menu, pop).into_any_element()
                        }
                    }),
                )
                // 编辑钮:信息卡悬停源之一(卡本体根级渲染,见 shell/mod;
                // 卡锚 = 悬停进入时指针位置,无渲染期 bounds 捕获)
                .child(
                    div()
                        .id(gpui_kit::SharedString::from(format!("pinned-ws-edit-{ws}")))
                        .flex()
                        .size(px(20.))
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .opacity(0.)
                        .group_hover(pin_grp.clone(), |s| s.opacity(1.))
                        .hover(|s| s.bg(theme::SIDEBAR_ACTIVE()).text_color(theme::LABEL()))
                        .text_color(theme::CAPTION())
                        .tooltip(crate::shell::tip(t!("sessions.edit_project")))
                        .child(fixed(LiumaIcon::Pencil, 14.))
                        .on_hover({
                            let s_hover = target_edit.clone();
                            let ws_edit = ws_edit.clone();
                            move |hovering: &bool, window, cx| {
                                let ws = ws_edit.clone();
                                s_hover.update(cx, |st, cx| {
                                    st.set_ws_info_hover_row(
                                        hovering.then(|| ws.clone()),
                                        window.mouse_position(),
                                        cx,
                                    );
                                });
                            }
                        }),
                )
                .on_click(move |_, _, cx| {
                    let ws = ws.to_string();
                    // 仅选中工作区,不开/不建会话(见 select_workspace 文档)
                    target.update(cx, |st, cx| st.select_workspace(&ws, cx));
                }),
        )
        // 清单(置顶会话已上提为气泡行);全上提/本无会话时显「暂无
        // 聊天」空状态,与项目节组空状态同款
        .children(if idxs.is_empty() {
            vec![group_empty_row(&ws_rows).into_any_element()]
        } else {
            preview_rows_in(store, cx, &ws_rows, idxs, "", "pinws")
        })
}

/// 工作区信息卡(参照 DSH 编辑卡):工作区名 + 置钉(点击 toggle
/// 置顶)+ 会话数 + 路径 + 「编辑项目」行(点击打开重命名模态)。
/// 根级渲染(见 shell/mod):库受控 Popover 的 set_open 抢窗口焦点,
/// 与 hover 驱动开合不兼容,此卡单独回归手绘浮层
pub(crate) fn ws_info_card(
    store: &Entity<AppStore>,
    cx: &App,
    ws: &str,
    session_count: usize,
) -> impl IntoElement {
    let st = store.read(cx);
    let display = st.title_for_workspace(ws);
    let pinned = st.sessions.pinned_workspaces.iter().any(|v| v == ws);
    let path = st
        .sessions
        .ws_paths
        .get(ws)
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let active = st.state.active_workspace.as_deref() == Some(ws);
    let (pin_store, edit_store) = (store.clone(), store.clone());
    let (ws_pin, ws_edit) = (ws.to_string(), ws.to_string());
    let row = |icon: gpui_kit::AnyElement, label: gpui_kit::SharedString, sel: &'static str| {
        div()
            .flex()
            .h(px(34.))
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .text_size(px(13.))
            .text_color(theme::LABEL())
            .child(icon)
            .child(div().flex().min_w(px(0.)).truncate().child(label))
            .debug_selector(|| sel.to_string())
    };
    div()
        .id("ws-info-card")
        .debug_selector(|| "ws-info-card".to_string())
        .v_flex()
        .w(px(280.))
        .rounded(px(12.))
        .border_1()
        .border_color(theme::BORDER())
        .bg(if theme::is_dark() {
            theme::LAYER()
        } else {
            theme::CARD()
        })
        .p(px(4.))
        .shadow_md()
        .child(
            div()
                .flex()
                .h(px(34.))
                .items_center()
                .gap(px(10.))
                .px(px(12.))
                .text_size(px(13.))
                .font_weight(gpui_kit::FontWeight::MEDIUM)
                .text_color(theme::LABEL())
                .child(
                    fixed(
                        if active {
                            LiumaIcon::FolderOpen
                        } else {
                            LiumaIcon::FolderClose
                        },
                        14.,
                    )
                    .text_color(if active {
                        theme::BRAND()
                    } else {
                        theme::LABEL_2()
                    }),
                )
                .child(div().flex().min_w(px(0.)).truncate().child(display))
                .child(div().flex_1())
                // 置钉:已置顶亮钉,点击 toggle 置顶(不收卡)
                .child(
                    div()
                        .id("ws-info-pin")
                        .flex()
                        .size(px(22.))
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::DOCK()))
                        .text_color(theme::CAPTION())
                        .child(fixed(
                            if pinned {
                                LiumaIcon::PinOff
                            } else {
                                LiumaIcon::Pin
                            },
                            14.,
                        ))
                        .tooltip(crate::shell::tip(if pinned {
                            t!("sessions.unpin")
                        } else {
                            t!("sessions.pin")
                        }))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            let ws = ws_pin.clone();
                            pin_store.update(cx, |st, cx| st.toggle_pinned_workspace(&ws, cx));
                        }),
                ),
        )
        .child(row(
            fixed(LiumaIcon::Message, 14.)
                .text_color(theme::LABEL_2())
                .into_any_element(),
            t!("sessions.ws_task_count", n = session_count).into(),
            "ws-info-count",
        ))
        .child(div().h(px(1.)).mx(px(8.)).my(px(3.)).bg(theme::BORDER()))
        .child(row(
            fixed(LiumaIcon::FolderClose, 14.)
                .text_color(theme::LABEL_2())
                .into_any_element(),
            path.into(),
            "ws-info-path",
        ))
        .child(div().h(px(1.)).mx(px(8.)).my(px(3.)).bg(theme::BORDER()))
        .child(
            div()
                .id("ws-info-edit")
                .flex()
                .h(px(34.))
                .items_center()
                .gap(px(10.))
                .px(px(12.))
                .rounded(px(8.))
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(theme::LABEL())
                .hover(|s| s.bg(theme::DOCK()))
                .child(fixed(LiumaIcon::Settings, 14.).text_color(theme::LABEL_2()))
                .child(t!("sessions.edit_project"))
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    let ws = ws_edit.clone();
                    edit_store.update(cx, |st, cx| st.open_rename_workspace(&ws, window, cx));
                }),
        )
}

/// 会话树:按工作区分组(首见序),搜索过滤;单列表态(视图选项)
/// 无组头全平铺
fn session_list(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let default = st.default_workspace();
    let query = st
        .search
        .search_input
        .as_ref()
        .map(|e| e.read(cx).value().trim().to_lowercase())
        .unwrap_or_default();

    // 单列表:全部会话平铺(subagent 仍隐藏;宿主清单序 = 最近更新,
    // 手动排序未实现前 order_mode 无渲染差异)
    if st.sessions.group_mode == GroupMode::Flat {
        let rows: Vec<gpui_kit::AnyElement> = st
            .state
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.origin.as_deref() != Some("subagent")
                    && (query.is_empty()
                        || st.title_for(&s.session_id).to_lowercase().contains(&query))
            })
            .map(|(ix, s)| session_row(store, cx, s, ix).into_any_element())
            .collect();
        return div()
            .id("sidebar-sessions")
            .v_flex()
            .min_h(px(0.))
            .flex_1()
            .overflow_y_scroll()
            .pb(px(8.))
            .gap(px(2.))
            .children(rows);
    }

    // 分组(组 = 工作区清单全量,清单序;无会话的工作区仍渲染组头——
    // 删除会话后组不可消失)。清单外的工作区名防御性追加在尾部
    let mut groups: Vec<(String, Vec<usize>)> = st
        .state
        .host_info
        .workspaces
        .iter()
        .map(|w| (w.clone(), Vec::new()))
        .collect();
    for (ix, s) in st.state.sessions.iter().enumerate() {
        // subagent 会话在侧栏隐藏(仅经页头目录进入)
        if s.origin.as_deref() == Some("subagent") {
            continue;
        }
        if !query.is_empty() && !st.title_for(&s.session_id).to_lowercase().contains(&query) {
            continue;
        }
        let key = workspace_of(&s.session_id, &default).to_string();
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(ix),
            None => groups.push((key, vec![ix])),
        }
    }
    // 搜索时隐藏无匹配会话的组(空组对搜索不可达,保持既有搜索语义)
    if !query.is_empty() {
        groups.retain(|(_, v)| !v.is_empty());
    }

    let pinned_ws: Vec<&String> = st
        .sessions
        .pinned_workspaces
        .iter()
        .filter(|w| groups.iter().any(|(k, _)| k == *w))
        .collect();
    let pinned_sessions: Vec<usize> = st
        .sessions
        .pinned_sessions
        .iter()
        .filter_map(|id| {
            st.state
                .sessions
                .iter()
                .position(|s| &s.session_id == id)
                .filter(|ix| {
                    st.state.sessions[*ix].origin.as_deref() != Some("subagent")
                        && (query.is_empty()
                            || st
                                .title_for(&st.state.sessions[*ix].session_id)
                                .to_lowercase()
                                .contains(&query))
                })
        })
        .collect();

    // 清单用组:置顶会话已上提为置顶节气泡行,从各组清单剔除(会话
    // 只显示一处,置顶块预览与项目节组预览共用本清单)。置顶工作区
    // 存在性判定仍用原始 groups(会话全上提的组其块不消失)
    let list_groups: Vec<(String, Vec<usize>)> = groups
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                v.iter()
                    .filter(|ix| !pinned_sessions.contains(ix))
                    .copied()
                    .collect(),
            )
        })
        .collect();

    let mut children: Vec<gpui_kit::AnyElement> = vec![];
    // 置顶节(参照 DSH 侧栏):置顶会话(气泡行)+ 置顶工作区(文件夹行,
    // 行下嵌其会话清单,5 条预览纪律同组;清单已剔除置顶会话)。搜索
    // 时置顶项同样走过滤
    if !pinned_sessions.is_empty() || !pinned_ws.is_empty() {
        children.push(section_label(t!("sessions.pinned_section")));
        for ix in &pinned_sessions {
            let s = &st.state.sessions[*ix];
            children.push(pinned_session_row(store, cx, s).into_any_element());
        }
        for ws in &pinned_ws {
            children.push(pinned_workspace_block(store, cx, ws, &list_groups).into_any_element());
        }
    }

    // 项目节:未置顶的工作区组(置顶工作区只在置顶节显示,两节互斥;
    // 同组双渲染会让 active/选中底色整屏重复,观感即「全被选中」)。
    // 组头恒在(删除会话后组不可消失);组内无可见会话(全上提置顶
    // 或本无会话)渲染「暂无聊天」空状态。全无项目组时隐藏节标签
    let mut projects: Vec<gpui_kit::AnyElement> = vec![];
    for (gi, (ws, raw_idxs)) in groups.iter().enumerate() {
        if st.sessions.pinned_workspaces.contains(ws) {
            continue;
        }
        let idxs: Vec<usize> = raw_idxs
            .iter()
            .filter(|ix| !pinned_sessions.contains(ix))
            .copied()
            .collect();
        // 折叠态隐藏组内行;搜索时忽略折叠(否则折叠组内匹配不可达)
        let collapsed = st.sessions.collapsed_workspaces.contains(ws) && query.is_empty();
        projects.push(group_header(store, cx, ws, gi, collapsed).into_any_element());
        if collapsed {
            continue;
        }
        if idxs.is_empty() {
            projects.push(group_empty_row(ws).into_any_element());
        } else {
            projects.extend(preview_rows(store, cx, ws, idxs, &query));
        }
    }
    if !projects.is_empty() {
        children.push(section_label(t!("sessions.projects_section")));
        children.extend(projects);
    }

    div()
        .id("sidebar-sessions")
        .v_flex()
        .min_h(px(0.))
        .flex_1()
        .overflow_y_scroll()
        .pb(px(8.))
        .gap(px(2.))
        .children(children)
}

/// 工作区组头(树形层级第一级):chevron 折叠钮 + 文件夹 + 标题(显示名
/// 覆盖)+ hover ⋯(整理菜单)与铅笔(信息卡);行主体点击选中工作区
fn group_header(
    store: &Entity<AppStore>,
    cx: &App,
    ws: &str,
    gi: usize,
    collapsed: bool,
) -> impl IntoElement {
    let st = store.read(cx);
    // 工作区选中 = 显式选择态(active_workspace 仅由点击工作区行/
    // 下拉设置,互斥由 store 写入端保证:选中工作区即清会话选中——
    // 见 select_workspace 文档),高亮上底色 + 亮字 + 品牌色打开
    // 文件夹;会话选中只亮会话行
    let selected = st.state.active_workspace.as_deref() == Some(ws);
    let display = st.title_for_workspace(ws);
    let (s, s_fold, s_menu, s_edit) = (store.clone(), store.clone(), store.clone(), store.clone());
    let (ws_row, ws_fold, ws_menu, ws_head, ws_edit) = (
        ws.to_string(),
        ws.to_string(),
        ws.to_string(),
        ws.to_string(),
        ws.to_string(),
    );
    let fg = if selected {
        theme::LABEL()
    } else {
        theme::LABEL_3()
    };
    let sel = format!("ws-chevron-{}", if collapsed { "closed" } else { "open" });
    // 行 hover 组:展开态 chevron 悬停才淡入,折叠态常显
    let grp = format!("ws-grp-{gi}");
    div()
        .id(("ws", gi))
        .debug_selector(move || format!("ws-head-{ws_head}"))
        .group(grp.clone())
        .flex()
        .h(px(34.))
        .flex_shrink_0()
        .items_center()
        .rounded(px(8.))
        .when(selected, |el| el.bg(theme::SIDEBAR_ACTIVE()))
        .pl(px(4.))
        .pr(px(4.))
        .gap(px(4.))
        .cursor_pointer()
        .hover(|s| s.bg(theme::SIDEBAR_HOVER()))
        .text_size(px(13.))
        .text_color(fg)
        // 折叠/文件夹同槽互换:默认显文件夹,行 hover
        // 换成折叠箭头;折叠态箭头常显。点击前指针必已悬停于行,箭头
        // 届时可见,无隐形命中问题
        .child(
            div()
                .relative()
                .size(px(18.))
                .flex_shrink_0()
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(if selected {
                            theme::BRAND()
                        } else {
                            theme::LABEL_3()
                        })
                        .when(collapsed, |el| el.opacity(0.))
                        .group_hover(grp.clone(), |s| s.opacity(0.))
                        .child(fixed(
                            if selected {
                                LiumaIcon::FolderOpen
                            } else {
                                LiumaIcon::FolderClose
                            },
                            16.,
                        )),
                )
                .child(
                    div()
                        .id(("ws-fold", gi))
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .hover(|s| s.bg(theme::SIDEBAR_ACTIVE()).text_color(theme::LABEL()))
                        .text_color(theme::CAPTION())
                        .when(!collapsed, |el| el.opacity(0.))
                        .group_hover(grp.clone(), |s| s.opacity(1.))
                        .child(fixed(
                            if collapsed {
                                IconName::ChevronRight
                            } else {
                                IconName::ChevronDown
                            },
                            14.,
                        ))
                        // 测试钩子:开/合两态异键(消除断言只增 map 的歧义)
                        .debug_selector(move || sel.clone())
                        .on_click({
                            let ws = ws_fold.clone();
                            move |_, _, cx| {
                                cx.stop_propagation();
                                let ws = ws.clone();
                                s_fold.update(cx, |st, cx| st.toggle_workspace_collapsed(&ws, cx));
                            }
                        }),
                ),
        )
        .child(div().min_w(px(0.)).truncate().child(display))
        // 行内弹性空区:把动作钮推到行右缘(布局归属行,不进浮层)。
        // 空区同时是信息卡悬停源:hover 行右半空白即开卡
        .child(div().id(("ws-info-hover", gi)).flex_1().h_full().on_hover({
            let s_hover = s_edit.clone();
            let ws_hover = ws_edit.clone();
            move |hovering: &bool, window, cx| {
                let ws = ws_hover.clone();
                s_hover.update(cx, |st, cx| {
                    st.set_ws_info_hover_row(
                        hovering.then(|| ws.clone()),
                        window.mouse_position(),
                        cx,
                    );
                });
            }
        }))
        // hover ⋯:工作区整理菜单(点击;悬停 ⋯ 不触发信息卡——
        // 菜单弹出区与卡重叠,卡须让位)
        .child(
            Popover::new(("ws-menu-pop", gi))
                .appearance(false)
                .anchor(Anchor::TopRight)
                .trigger(PopTrigger(
                    div()
                        .id(("ws-menu", gi))
                        .debug_selector(|| "ws-menu-btn".to_string())
                        .flex()
                        .size(px(20.))
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .opacity(0.)
                        .group_hover(grp.clone(), |s| s.opacity(1.))
                        .hover(|s| s.bg(theme::SIDEBAR_ACTIVE()).text_color(theme::LABEL()))
                        .text_color(theme::CAPTION())
                        .tooltip(crate::shell::tip(t!("sessions.ws_actions")))
                        .child(fixed(IconName::Ellipsis, 14.)),
                ))
                .content({
                    let s_menu = s_menu.clone();
                    let ws_menu = ws_menu.clone();
                    move |_, _, cx| {
                        let pop = cx.entity();
                        ws_menu_card(&s_menu, cx, &ws_menu, pop).into_any_element()
                    }
                }),
        )
        // 编辑钮:信息卡悬停源之一(卡本体根级渲染,见 shell/mod;库受
        // 控 Popover 的 set_open 会抢窗口焦点,hover 驱动下开合震荡,
        // 故此卡单独回归根级手绘。卡锚 = 悬停进入时指针位置,无渲染期
        // bounds 捕获)
        .child(
            div()
                .id(("ws-edit", gi))
                .debug_selector(move || format!("ws-edit-{gi}"))
                .flex()
                .size(px(20.))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .opacity(0.)
                .group_hover(grp.clone(), |s| s.opacity(1.))
                .hover(|s| s.bg(theme::SIDEBAR_ACTIVE()).text_color(theme::LABEL()))
                .text_color(theme::CAPTION())
                .tooltip(crate::shell::tip(t!("sessions.edit_project")))
                .child(fixed(LiumaIcon::Pencil, 14.))
                .on_hover({
                    let s_hover = s_edit.clone();
                    let ws_edit = ws_edit.clone();
                    move |hovering: &bool, window, cx| {
                        let ws = ws_edit.clone();
                        s_hover.update(cx, |st, cx| {
                            st.set_ws_info_hover_row(
                                hovering.then(|| ws.clone()),
                                window.mouse_position(),
                                cx,
                            );
                        });
                    }
                }),
        )
        .on_click(move |_, _, cx| {
            let ws = ws_row.clone();
            // 仅选中工作区,不开/不建会话(见 select_workspace 文档)
            s.update(cx, |st, cx| st.select_workspace(&ws, cx));
        })
}

/// 单个会话行(行高 34:行首活动状态槽 + 标题 + 尾部时间↔归档;
/// 当前选中高亮)
fn session_row(
    store: &Entity<AppStore>,
    cx: &App,
    s: &SessionSummary,
    ix: usize,
) -> impl IntoElement {
    session_row_in(store, cx, s, ix, "sess")
}

/// `grp_ns` = hover 组命名空间:置顶块(pinws)与项目节(sess)的行
/// 元素 id / hover 组各自独立。互斥渲染后同一会话只落一处,命名空间
/// 保留为防御(组 id 撞名会让 group_hover 互串)
fn session_row_in(
    store: &Entity<AppStore>,
    cx: &App,
    s: &SessionSummary,
    ix: usize,
    grp_ns: &str,
) -> impl IntoElement {
    let st = store.read(cx);
    let active = st.state.current_id.as_deref() == Some(&s.session_id);
    let title = st.title_for(&s.session_id);
    let running = st.is_running(&s.session_id);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let time = relative_time(now, s.updated_at);
    let (bg, fg) = if active {
        (theme::SIDEBAR_ACTIVE(), theme::LABEL())
    } else {
        (theme::TRANSPARENT(), theme::LABEL_2())
    };
    // 后台子代理运行中(自身非 running 时)替代时间位
    let sub_running = st.running_subagent_count(&s.session_id);
    let pinned = st
        .sessions
        .pinned_sessions
        .iter()
        .any(|v| v == &s.session_id);
    let target = store.clone();
    let archived = store.clone();
    let pin_store = store.clone();
    let id = s.session_id.clone();
    let arch_id = id.clone();
    let pin_id = id.clone();
    let sel = format!("session-row-{id}");
    let sel_arch = format!("session-archive-{ix}");
    let sel_pin = format!("session-pin-{id}");
    // 行 hover 组:尾部时间 ↔ 双钮(置顶+归档)互换
    let grp = format!("{grp_ns}-grp-{ix}");
    // 元素 id 同样带 ns:同一会话在置顶块与项目节双渲染,撞 id 会让
    // gpui 元素状态(hover/交互)跨实例串扰
    let id_ns = grp_ns.to_string();
    div()
        .id(gpui_kit::SharedString::from(format!("{id_ns}-{ix}")))
        .relative()
        .group(grp.clone())
        .flex()
        .h(px(34.))
        .flex_shrink_0()
        .items_center()
        .rounded(px(8.))
        .bg(bg)
        // 树形第二级:组头之下一层轻缩进(行首状态槽承担对齐)
        .pl(px(8.))
        .pr(px(8.))
        .gap(px(8.))
        .cursor_pointer()
        .hover(|s| s.bg(theme::SIDEBAR_HOVER()))
        // 行首状态槽(活动动画在行首,非运行时空占位对齐)
        .child(
            div()
                .flex()
                .w(px(14.))
                .flex_shrink_0()
                .justify_center()
                .children(running.then(running_dot)),
        )
        .child(
            div()
                .flex()
                .min_w(px(0.))
                .flex_1()
                .truncate()
                .text_size(px(13.))
                .text_color(fg)
                .child(title),
        )
        // 尾部槽:相对时间(或子代理徽标)常态显示,行 hover 换双钮
        // (置顶 + 归档;悬停钮白色高亮 + tooltip)
        .child(
            div()
                .relative()
                .h(px(20.))
                .w(px(64.))
                .flex_shrink_0()
                .child(
                    div()
                        .absolute()
                        .right_0()
                        .top_0()
                        .bottom_0()
                        .flex()
                        .items_center()
                        .group_hover(grp.clone(), |s| s.opacity(0.))
                        .child(if sub_running > 0 {
                            sub_running_badge(sub_running)
                        } else {
                            plain_time(&time)
                        }),
                )
                .child(
                    div()
                        .absolute()
                        .right_0()
                        .top_0()
                        .bottom_0()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .group_hover(grp.clone(), |s| s.opacity(1.))
                        .opacity(0.)
                        // 置顶钮(已置顶 = PinOff,标识点击取消;色恒中性灰)
                        .child(
                            div()
                                .id(gpui_kit::SharedString::from(format!(
                                    "{id_ns}-session-pin-{ix}"
                                )))
                                .debug_selector(move || sel_pin.clone())
                                .flex()
                                .size(px(20.))
                                .items_center()
                                .justify_center()
                                .rounded(px(4.))
                                .cursor_pointer()
                                .hover(|s| s.bg(theme::SIDEBAR_ACTIVE()).text_color(theme::LABEL()))
                                .text_color(theme::CAPTION())
                                .child(fixed(
                                    if pinned {
                                        LiumaIcon::PinOff
                                    } else {
                                        LiumaIcon::Pin
                                    },
                                    14.,
                                ))
                                .tooltip(crate::shell::tip(if pinned {
                                    t!("sessions.unpin")
                                } else {
                                    t!("sessions.pin")
                                }))
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    let id = pin_id.clone();
                                    pin_store
                                        .update(cx, |st, cx| st.toggle_pinned_session(&id, cx));
                                }),
                        )
                        .child(
                            div()
                                .id(gpui_kit::SharedString::from(format!(
                                    "{id_ns}-session-archive-{ix}"
                                )))
                                .debug_selector(move || sel_arch.clone())
                                .flex()
                                .size(px(20.))
                                .items_center()
                                .justify_center()
                                .rounded(px(4.))
                                .cursor_pointer()
                                .hover(|s| s.bg(theme::SIDEBAR_ACTIVE()).text_color(theme::LABEL()))
                                .text_color(theme::CAPTION())
                                .child(fixed(LiumaIcon::Archive, 14.))
                                .tooltip(crate::shell::tip(t!("sessions.tip_archive")))
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    let id = arch_id.clone();
                                    archived.update(cx, |st, cx| st.archive(&id, cx));
                                }),
                        ),
                ),
        )
        // 测试钩子:按会话 id 稳定检索(release 空操作)
        .debug_selector(move || sel.clone())
        .on_click(move |_, _, cx| {
            let id = id.clone();
            target.update(cx, |st, cx| st.open_session(&id, cx));
        })
}

/// 标题栏会话菜单卡(会话管理:重命名/归档/分叉 | 导出日志;作用于
/// **当前会话**,根级渲染按 ⋯ 钮点击坐标定位,向左展开避开窗口右缘)。
/// 整卡挂 mousedown 豁免(同 composer 菜单:防根级外点关闭吞掉菜单项
/// 点击)
pub(crate) fn session_menu_card(
    store: &Entity<AppStore>,
    pop: Entity<PopoverState>,
    cx: &App,
) -> impl IntoElement {
    let pinned_session = store
        .read(cx)
        .state
        .current_id
        .as_deref()
        .map(|id| {
            store
                .read(cx)
                .sessions
                .pinned_sessions
                .iter()
                .any(|v| v == id)
        })
        .unwrap_or(false);
    let (rename, archive, fork, export_log, pin_store) = (
        store.clone(),
        store.clone(),
        store.clone(),
        store.clone(),
        store.clone(),
    );
    let (p_arch, p_fork, p_export, p_pin) = (pop.clone(), pop.clone(), pop.clone(), pop.clone());
    div()
        .id("session-menu-card")
        .debug_selector(|| "session-menu-card".to_string())
        .v_flex()
        .w(px(112.))
        .gap(px(2.))
        .rounded(px(10.))
        .border_1()
        .border_color(theme::BORDER())
        .bg(theme::LAYER())
        .p(px(4.))
        .shadow_md()
        .child(menu_item(
            "menu-pin",
            if pinned_session {
                t!("sessions.unpin")
            } else {
                t!("sessions.pin")
            },
            fixed(LiumaIcon::Pin, 13.),
            move |_, window, cx| {
                p_pin.update(cx, |state, cx| state.dismiss(window, cx));
                pin_store.update(cx, |st, cx| {
                    let Some(id) = st.state.current_id.clone() else {
                        return;
                    };
                    st.toggle_pinned_session(&id, cx);
                });
            },
        ))
        .child(menu_item(
            "menu-rename",
            t!("sessions.rename"),
            fixed(LiumaIcon::Pencil, 13.),
            move |_, window, cx| {
                pop.update(cx, |state, cx| state.dismiss(window, cx));
                rename.update(cx, |st, cx| {
                    let Some(id) = st.state.current_id.clone() else {
                        return;
                    };
                    st.open_rename(&id, window, cx);
                });
            },
        ))
        .child(menu_item(
            "menu-archive",
            t!("sessions.archive"),
            fixed(LiumaIcon::Archive, 13.),
            move |_, window, cx| {
                p_arch.update(cx, |state, cx| state.dismiss(window, cx));
                archive.update(cx, |st, cx| {
                    let Some(id) = st.state.current_id.clone() else {
                        return;
                    };
                    st.archive(&id, cx);
                });
            },
        ))
        .child(menu_item(
            "menu-fork",
            t!("sessions.fork"),
            fixed(LiumaIcon::GitBranch, 13.),
            move |_, window, cx| {
                p_fork.update(cx, |state, cx| state.dismiss(window, cx));
                fork.update(cx, |st, cx| {
                    let Some(id) = st.state.current_id.clone() else {
                        return;
                    };
                    st.fork(&id, cx);
                });
            },
        ))
        .child(menu_divider())
        .child(menu_item(
            "menu-export-log",
            t!("sessions.export_log"),
            fixed(LiumaIcon::Download, 13.),
            move |_, window, cx| {
                p_export.update(cx, |state, cx| state.dismiss(window, cx));
                export_log.update(cx, |st, cx| {
                    let Some(id) = st.state.current_id.clone() else {
                        return;
                    };
                    st.export_session_log(&id, window, cx);
                });
            },
        ))
}

/// 菜单组分隔线
fn menu_divider() -> gpui_kit::AnyElement {
    div()
        .h(px(1.))
        .mx(px(8.))
        .my(px(3.))
        .bg(theme::BORDER())
        .into_any_element()
}

/// 菜单项(图标 + 文字)
/// 菜单项:`sel` = 稳定 ASCII selector,`label` = 可见文案。
///
/// selector 与文案分离:文案随语言切换,selector 必须恒定(旧实现把
/// 中文标签当 id,测试也按中文寻址,文案一动就全红)。
fn menu_item(
    sel: &'static str,
    label: impl Into<gpui_kit::SharedString>,
    icon: Icon,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut App) + 'static,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let label = label.into();
    div()
        .id(sel)
        .debug_selector(move || sel.to_string())
        .flex()
        .h(px(26.))
        .items_center()
        .gap(px(6.))
        .px(px(8.))
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|st| st.bg(theme::DOCK()))
        .text_size(px(12.))
        .text_color(theme::LABEL_2())
        .child(icon)
        .child(label)
        .on_click(move |ev, w, cx| {
            cx.stop_propagation();
            on_click(ev, w, cx)
        })
}

/// 工作区分组头 ⋯ 菜单(重命名/删除工作区;组件库 Popover 内容,
/// 库托管开合/定位。默认工作区不提供删除)
fn ws_menu_card(
    store: &Entity<AppStore>,
    cx: &App,
    ws: &str,
    pop: Entity<PopoverState>,
) -> impl IntoElement {
    let is_default = store.read(cx).default_workspace() == ws;
    let pinned = store
        .read(cx)
        .sessions
        .pinned_workspaces
        .iter()
        .any(|v| v == ws);
    let (rename, remove, pin_store) = (store.clone(), store.clone(), store.clone());
    let (wid_r, wid_x, wid_p) = (ws.to_string(), ws.to_string(), ws.to_string());
    let (p_ren, p_del, p_pin) = (pop.clone(), pop.clone(), pop.clone());
    div()
        .id("ws-menu-card")
        .debug_selector(|| "ws-menu-card".to_string())
        .v_flex()
        .w(px(112.))
        .gap(px(2.))
        .rounded(px(10.))
        .border_1()
        .border_color(theme::BORDER())
        .bg(theme::LAYER())
        .p(px(4.))
        .shadow_md()
        .child(menu_item(
            "menu-ws-pin",
            if pinned {
                t!("sessions.unpin")
            } else {
                t!("sessions.pin")
            },
            fixed(LiumaIcon::Pin, 13.),
            move |_, window, cx| {
                p_pin.update(cx, |state, cx| state.dismiss(window, cx));
                let id = wid_p.clone();
                pin_store.update(cx, |st, cx| st.toggle_pinned_workspace(&id, cx));
            },
        ))
        .child(menu_item(
            "menu-ws-rename",
            t!("sessions.rename"),
            fixed(LiumaIcon::Pencil, 13.),
            move |_, window, cx| {
                // 先开模态再收菜单:dismiss 在前会在点击对完成前移除
                // 自身(信息卡在场后该竞态显形)
                let id = wid_r.clone();
                rename.update(cx, |st, cx| st.open_rename_workspace(&id, window, cx));
                p_ren.update(cx, |state, cx| state.dismiss(window, cx));
            },
        ))
        .when(!is_default, |el| {
            el.child(menu_item(
                "menu-ws-delete",
                t!("sessions.delete_workspace"),
                fixed(IconName::Delete, 13.),
                move |_, window, cx| {
                    p_del.update(cx, |state, cx| state.dismiss(window, cx));
                    let id = wid_x.clone();
                    remove.update(cx, |st, cx| st.remove_workspace(&id, cx));
                },
            ))
        })
}

/// 顶栏视图选项菜单(分组方式/排序方式两组;组件库 Popover 内容,
/// 库托管开合/定位)。「手动排序」本期占位:置灰不可点,拖拽重排
/// 后续实现
fn view_options_menu_card(
    store: &Entity<AppStore>,
    pop: Entity<PopoverState>,
    cx: &App,
) -> impl IntoElement {
    let st = store.read(cx);
    let (group, order) = (st.sessions.group_mode, st.sessions.order_mode);
    let (s_ws, s_flat, s_updated) = (store.clone(), store.clone(), store.clone());
    let (p_ws, p_flat, p_updated) = (pop.clone(), pop.clone(), pop.clone());
    div()
        .id("view-menu-card")
        .debug_selector(|| "view-menu-card".to_string())
        .v_flex()
        .w(px(200.))
        .rounded(px(12.))
        .border_1()
        .border_color(theme::BORDER())
        .bg(theme::LAYER())
        .p(px(4.))
        .shadow_md()
        .child(menu_section_label(t!("sessions.group_label")))
        .child(view_menu_item(
            "view-group-ws",
            t!("sessions.by_workspace"),
            group == GroupMode::Workspace,
            p_ws.clone(),
            move |_, _, cx| {
                s_ws.update(cx, |st, cx| st.set_group_mode(GroupMode::Workspace, cx));
            },
        ))
        .child(view_menu_item(
            "view-group-flat",
            t!("sessions.single_list"),
            group == GroupMode::Flat,
            p_flat.clone(),
            move |_, _, cx| {
                s_flat.update(cx, |st, cx| st.set_group_mode(GroupMode::Flat, cx));
            },
        ))
        .child(div().h(px(1.)).mx(px(8.)).my(px(4.)).bg(theme::BORDER()))
        .child(menu_section_label(t!("sessions.sort_label")))
        .child(view_menu_item(
            "view-order-updated",
            t!("sessions.recent_updates"),
            order == OrderMode::Updated,
            p_updated.clone(),
            move |_, _, cx| {
                s_updated.update(cx, |st, cx| st.set_order_mode(OrderMode::Updated, cx));
            },
        ))
        .child(
            div()
                .id("view-order-manual")
                .debug_selector(|| "view-order-manual".to_string())
                .flex()
                .h(px(30.))
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .rounded(px(8.))
                .text_size(px(13.))
                .text_color(theme::CAPTION())
                .child(t!("sessions.manual_sort")),
        )
}

/// 菜单节标(分组方式/排序方式)
fn menu_section_label(label: impl Into<gpui_kit::SharedString>) -> gpui_kit::AnyElement {
    let label = label.into();
    div()
        .px(px(8.))
        .pt(px(6.))
        .pb(px(2.))
        .text_size(px(12.))
        .text_color(theme::CAPTION())
        .child(label)
        .into_any_element()
}

/// 视图选项菜单项(文字 + 选中尾部 ✓;选择即收菜单)
fn view_menu_item(
    id: &'static str,
    label: impl Into<gpui_kit::SharedString>,
    selected: bool,
    pop: Entity<PopoverState>,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut App) + 'static,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let label = label.into();
    div()
        .id(id)
        .debug_selector(move || format!("view-item-{id}"))
        .flex()
        .h(px(30.))
        .items_center()
        .gap(px(8.))
        .px(px(8.))
        .rounded(px(8.))
        .cursor_pointer()
        .hover(|st| st.bg(theme::DOCK()))
        .text_size(px(13.))
        .text_color(theme::LABEL_2())
        .child(div().flex_1().child(label))
        .children(selected.then(|| fixed(IconName::Check, 14.).text_color(theme::LABEL())))
        .on_click(move |ev, w, cx| {
            pop.update(cx, |state, cx| state.dismiss(w, cx));
            on_click(ev, w, cx)
        })
}

/// 执行中徽点(替代时间位;点阵追逐动画)
/// 「N 个子代理运行中」行尾状态
/// (优先级低于自身运行中,替代相对时间位)
fn sub_running_badge(n: usize) -> gpui_kit::AnyElement {
    div()
        .debug_selector(|| "session-row-sub-running".to_string())
        .flex()
        .items_center()
        .gap(px(4.))
        .flex_shrink_0()
        .child(div().size(px(6.)).rounded_full().bg(theme::ONGOING()))
        .child(
            div()
                .text_size(px(11.))
                .text_color(theme::CAPTION())
                .child(if n == 1 {
                    t!("sessions.subagents_one", n = n)
                } else {
                    t!("sessions.subagents_other", n = n)
                }),
        )
        .into_any_element()
}

fn running_dot() -> gpui_kit::AnyElement {
    crate::kits::state_dot::ongoing_dot(8.).into_any_element()
}

/// 相对时间
fn plain_time(time: &str) -> gpui_kit::AnyElement {
    div()
        .flex_shrink_0()
        .text_size(px(11.))
        .text_color(theme::CAPTION())
        .child(time.to_string())
        .into_any_element()
}
