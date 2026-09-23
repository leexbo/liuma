//! 侧栏:会话列表(280px;收起完全隐藏)。
//! 按工作区分组(前缀推导),支持本地搜索过滤、新建、切换。

use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::Icon;
use gpui_kit::component::IconName;
use gpui_kit::component::InteractiveElementExt as _;
use gpui_kit::component::StyledExt;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Entity, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, StatefulInteractiveElement, Styled, div, px,
};
use liuma_core::proto::SessionSummary;

use crate::features::search;
use crate::features::sessions::store::{GroupMode, OrderMode};
use crate::features::settings;
use crate::kits::i18n::dict;
use crate::kits::icons::{LiumaIcon, fixed};
use crate::kits::theme;
use crate::shell::reducer::{relative_time, workspace_of};
use crate::shell::store::{AppStore, TIP_ADD_WS, TIP_ARCHIVE_BASE, TIP_SEARCH, TIP_VIEW_MENU};
use crate::shell::tip_capture_layer;

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
            .child(dict::sessions::new_session())
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
                    dict::sessions::group_flat()
                } else {
                    dict::sessions::group_by_ws()
                }),
        )
        .child(div().flex_1())
        .child(
            header_icon_button(
                store,
                TIP_SEARCH,
                dict::sessions::search_ph(),
                fixed(LiumaIcon::SearchOutline, 14.),
            )
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                s_search.update(cx, |st, cx| st.toggle_search_open(window, cx));
            }),
        )
        .child(
            header_icon_button(
                store,
                TIP_VIEW_MENU,
                dict::sessions::view_options(),
                fixed(LiumaIcon::Personalization, 15.),
            )
            .on_click(move |ev: &gpui_kit::ClickEvent, _, cx| {
                cx.stop_propagation();
                let pos = match ev {
                    gpui_kit::ClickEvent::Mouse(m) => m.down.position,
                    _ => gpui_kit::Point::default(),
                };
                s_view.update(cx, |st, cx| st.open_view_menu_at(pos, cx));
            }),
        )
        .child(
            header_icon_button(
                store,
                TIP_ADD_WS,
                dict::sessions::add_workspace(),
                fixed(LiumaIcon::ProjectAdd, 16.),
            )
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                s_add.update(cx, |st, cx| st.add_workspace_via_picker(cx));
            }),
        )
}

/// 顶栏图标钮(圆形 hover 底;hover 500ms 出 tooltip)。含渲染期
/// bounds 捕获层(canvas 写 tip_bounds[slot],tooltip 锚定用)与
/// mousedown 豁免(头行为窗口拖拽区,钮点击不得触发拖窗)
fn header_icon_button(
    store: &Entity<AppStore>,
    slot: usize,
    tip: &'static str,
    icon: Icon,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let s = store.clone();
    let sel = format!("header-btn-{slot}");
    div()
        .id(("header-btn", slot))
        .debug_selector(move || sel.clone())
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
        .on_hover(move |enter: &bool, _, cx| {
            s.update(cx, |st, cx| st.header_tip_hover(slot, tip, *enter, cx));
        })
        .child(icon)
        .child(tip_capture_layer(store, slot))
}

/// 顶栏钮 tooltip 卡(根级渲染;按钮下方居中,窗缘钳制;非交互不
/// occlude,不挡下方命中)
pub fn header_tip_card(
    text: gpui_kit::SharedString,
    b: gpui_kit::Bounds<gpui_kit::Pixels>,
    viewport_w: f32,
) -> impl IntoElement {
    let w = text.chars().count() as f32 * 12. + 20.;
    let center = f32::from(b.origin.x) + f32::from(b.size.width) / 2.;
    let left = (center - w / 2.).clamp(8., (viewport_w - w - 8.).max(8.));
    div()
        .absolute()
        .top(px(f32::from(b.origin.y) + f32::from(b.size.height) + 6.))
        .left(px(left))
        .flex()
        .h(px(24.))
        .items_center()
        .px(px(10.))
        .rounded(px(6.))
        .border_1()
        .border_color(theme::BORDER_2())
        .bg(theme::DOCK())
        .shadow_md()
        .text_size(px(12.))
        .text_color(theme::LABEL_2())
        .child(text)
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

    let mut children: Vec<gpui_kit::AnyElement> = vec![];
    for (gi, (ws, idxs)) in groups.into_iter().enumerate() {
        // 折叠态隐藏组内行;搜索时忽略折叠(否则折叠组内匹配不可达)
        let collapsed = st.sessions.collapsed_workspaces.contains(&ws) && query.is_empty();
        children.push(group_header(store, cx, &ws, gi, collapsed).into_any_element());
        if !collapsed {
            for ix in idxs {
                let s = &st.state.sessions[ix];
                children.push(session_row(store, cx, s, ix).into_any_element());
            }
        }
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
/// 覆盖)+ hover「+」(该工作区新建)与 ⋯(整理菜单);行主体点击切换
/// 工作区(兼自动展开)
fn group_header(
    store: &Entity<AppStore>,
    cx: &App,
    ws: &str,
    gi: usize,
    collapsed: bool,
) -> impl IntoElement {
    let st = store.read(cx);
    let active = st.state.active_workspace.as_deref() == Some(ws);
    let display = st.title_for_workspace(ws);
    let (s, s_add, s_fold, s_menu) = (store.clone(), store.clone(), store.clone(), store.clone());
    let (ws_row, ws_fold, ws_add, ws_menu, ws_head) = (
        ws.to_string(),
        ws.to_string(),
        ws.to_string(),
        ws.to_string(),
        ws.to_string(),
    );
    let fg = if active {
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
        .h(px(28.))
        .flex_shrink_0()
        .items_center()
        .rounded(px(6.))
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
                        // 选中工作区:打开文件夹 + 品牌色(duotone)
                        .text_color(if active {
                            theme::BRAND()
                        } else {
                            theme::LABEL_3()
                        })
                        .when(collapsed, |el| el.opacity(0.))
                        .group_hover(grp.clone(), |s| s.opacity(0.))
                        .child(fixed(
                            if active {
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
                        .hover(|s| s.bg(theme::SIDEBAR_HOVER()))
                        .text_color(theme::CAPTION())
                        .when(!collapsed, |el| el.opacity(0.))
                        .group_hover(grp.clone(), |s| s.opacity(1.))
                        .child(fixed(
                            if collapsed {
                                IconName::ChevronRight
                            } else {
                                IconName::ChevronDown
                            },
                            13.,
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
        .child(div().flex_1())
        // hover ⋯:工作区整理菜单(重命名/排序/移除)
        .child(
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
                .hover(|s| s.opacity(1.).bg(theme::SIDEBAR_HOVER()))
                .text_color(theme::CAPTION())
                .child(fixed(IconName::Ellipsis, 13.))
                .on_click(move |ev: &gpui_kit::ClickEvent, _, cx| {
                    cx.stop_propagation();
                    let pos = match ev {
                        gpui_kit::ClickEvent::Mouse(m) => m.down.position,
                        gpui_kit::ClickEvent::Keyboard(_) => gpui_kit::Point::default(),
                        gpui_kit::ClickEvent::Touch(_) => gpui_kit::Point::default(),
                    };
                    s_menu.update(cx, |st, cx| {
                        if st.sessions.menu_open_ws.as_deref() == Some(&ws_menu) {
                            st.sessions.menu_open_ws = None;
                            cx.notify();
                        } else {
                            st.open_ws_menu_at(&ws_menu, pos, cx);
                        }
                    });
                }),
        )
        // hover「+」:该工作区新建会话(web 同位)
        .child(
            div()
                .id(("ws-new", gi))
                .flex()
                .size(px(20.))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .opacity(0.)
                .hover(|s| s.opacity(1.).bg(theme::SIDEBAR_HOVER()))
                .text_color(theme::CAPTION())
                .child(fixed(IconName::Plus, 13.))
                .on_click({
                    let ws = ws_add.clone();
                    move |_, _, cx| {
                        cx.stop_propagation();
                        let ws = ws.clone();
                        s_add.update(cx, |st, cx| st.create_session_in(&ws, cx));
                    }
                }),
        )
        .on_click(move |_, _, cx| {
            let ws = ws_row.clone();
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
    let target = store.clone();
    let archived = store.clone();
    let arch_tip = store.clone();
    let id = s.session_id.clone();
    let arch_id = id.clone();
    let sel = format!("session-row-{id}");
    let sel_arch = format!("session-archive-{ix}");
    let arch_slot = TIP_ARCHIVE_BASE + ix;
    // 行 hover 组:尾部时间 ↔ 归档钮互换
    let grp = format!("sess-grp-{ix}");
    div()
        .id(("session", ix))
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
        // 尾部槽:相对时间(或子代理徽标)↔ 归档钮 hover 互换。
        // 绝对定位右对齐,槽宽按徽标上限
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
                        .id(("session-archive", ix))
                        .debug_selector(move || sel_arch.clone())
                        .absolute()
                        .right_0()
                        .top_0()
                        .bottom_0()
                        .flex()
                        .items_center()
                        .px(px(4.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .opacity(0.)
                        .group_hover(grp.clone(), |s| s.opacity(1.))
                        .hover(|s| s.bg(theme::SIDEBAR_HOVER()))
                        .text_color(theme::CAPTION())
                        .child(fixed(LiumaIcon::Archive, 14.))
                        .child(crate::shell::tip_capture_layer(store, arch_slot))
                        .on_hover(move |enter: &bool, _, cx| {
                            arch_tip.update(cx, |st, cx| {
                                st.header_tip_hover(
                                    arch_slot,
                                    dict::sessions::tip_archive(),
                                    *enter,
                                    cx,
                                )
                            });
                        })
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            let id = arch_id.clone();
                            archived.update(cx, |st, cx| st.archive(&id, cx));
                        }),
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
pub fn session_menu_card(
    store: &Entity<AppStore>,
    pos: gpui_kit::Point<gpui_kit::Pixels>,
) -> impl IntoElement {
    let (rename, archive, fork, export_log) =
        (store.clone(), store.clone(), store.clone(), store.clone());
    div()
        .id("session-menu-card")
        .absolute()
        // 阻断鼠标命中向卡后方穿透(否则点击会落到后面的内容区上)
        .occlude()
        .debug_selector(|| "session-menu-card".to_string())
        // 锚在标题栏 ⋯ 钮下方,卡右缘对齐钮右缘(钮 26px,点击点近似
        // 钮中心)、向左展开避开窗右缘
        .top(pos.y + px(14.))
        .left(pos.x - px(112.) + px(13.))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
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
            dict::sessions::rename(),
            fixed(LiumaIcon::Pencil, 13.),
            move |_, window, cx| {
                rename.update(cx, |st, cx| {
                    let Some(id) = st.state.current_id.clone() else {
                        return;
                    };
                    st.open_rename(&id, window, cx);
                });
            },
        ))
        .child(menu_item(
            dict::sessions::archive(),
            fixed(LiumaIcon::Archive, 13.),
            move |_, _, cx| {
                archive.update(cx, |st, cx| {
                    let Some(id) = st.state.current_id.clone() else {
                        return;
                    };
                    st.archive(&id, cx);
                    st.sessions.session_menu_pos = None;
                });
            },
        ))
        .child(menu_item(
            dict::sessions::fork(),
            fixed(LiumaIcon::GitBranch, 13.),
            move |_, _, cx| {
                fork.update(cx, |st, cx| {
                    let Some(id) = st.state.current_id.clone() else {
                        return;
                    };
                    st.fork(&id, cx);
                    st.sessions.session_menu_pos = None;
                });
            },
        ))
        .child(menu_divider())
        .child(menu_item(
            dict::sessions::export_log(),
            fixed(LiumaIcon::Download, 13.),
            move |_, window, cx| {
                export_log.update(cx, |st, cx| {
                    let Some(id) = st.state.current_id.clone() else {
                        return;
                    };
                    st.sessions.session_menu_pos = None;
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
fn menu_item(
    label: &'static str,
    icon: Icon,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(label)
        .debug_selector(move || label.to_string())
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

/// 工作区分组头 ⋯ 菜单卡(重命名/删除工作区;根级渲染按点击坐标
/// 定位,向左展开。默认工作区不提供删除)
pub fn ws_menu_card(
    store: &Entity<AppStore>,
    cx: &App,
    ws: &str,
    pos: gpui_kit::Point<gpui_kit::Pixels>,
) -> impl IntoElement {
    let is_default = store.read(cx).default_workspace() == ws;
    let (rename, remove) = (store.clone(), store.clone());
    let (wid_r, wid_x) = (ws.to_string(), ws.to_string());
    div()
        .id("ws-menu-card")
        .absolute()
        .occlude()
        .debug_selector(|| "ws-menu-card".to_string())
        .top(pos.y + px(14.))
        .left(pos.x - px(112.) - px(8.))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
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
            dict::sessions::rename(),
            fixed(LiumaIcon::Pencil, 13.),
            move |_, window, cx| {
                let id = wid_r.clone();
                rename.update(cx, |st, cx| st.open_rename_workspace(&id, window, cx));
            },
        ))
        .when(!is_default, |el| {
            el.child(menu_item(
                dict::sessions::delete_workspace(),
                fixed(IconName::Delete, 13.),
                move |_, _, cx| {
                    let id = wid_x.clone();
                    remove.update(cx, |st, cx| st.remove_workspace(&id, cx));
                },
            ))
        })
}

/// 顶栏视图选项菜单卡(分组方式/排序方式两组;根级渲染按点击坐标
/// 定位,右对齐滑块钮展开)。整卡挂 mousedown 豁免(同 row/ws 菜单)。
/// 「手动排序」本期占位:置灰不可点,拖拽重排后续实现
pub fn view_options_menu_card(
    store: &Entity<AppStore>,
    cx: &App,
    pos: gpui_kit::Point<gpui_kit::Pixels>,
) -> impl IntoElement {
    let st = store.read(cx);
    let (group, order) = (st.sessions.group_mode, st.sessions.order_mode);
    let (s_ws, s_flat, s_updated) = (store.clone(), store.clone(), store.clone());
    div()
        .id("view-menu-card")
        .absolute()
        .occlude()
        .debug_selector(|| "view-menu-card".to_string())
        // 锚在滑块钮下方,右对齐钮(钮 26px 宽)
        .top(pos.y + px(14.))
        .left(pos.x - px(200.) + px(26.))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .v_flex()
        .w(px(200.))
        .rounded(px(12.))
        .border_1()
        .border_color(theme::BORDER())
        .bg(theme::LAYER())
        .p(px(4.))
        .shadow_md()
        .child(menu_section_label(dict::sessions::group_label()))
        .child(view_menu_item(
            "view-group-ws",
            dict::sessions::by_workspace(),
            group == GroupMode::Workspace,
            move |_, _, cx| {
                s_ws.update(cx, |st, cx| st.set_group_mode(GroupMode::Workspace, cx));
            },
        ))
        .child(view_menu_item(
            "view-group-flat",
            dict::sessions::single_list(),
            group == GroupMode::Flat,
            move |_, _, cx| {
                s_flat.update(cx, |st, cx| st.set_group_mode(GroupMode::Flat, cx));
            },
        ))
        .child(div().h(px(1.)).mx(px(8.)).my(px(4.)).bg(theme::BORDER()))
        .child(menu_section_label(dict::sessions::sort_label()))
        .child(view_menu_item(
            "view-order-updated",
            dict::sessions::recent_updates(),
            order == OrderMode::Updated,
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
                .child(dict::sessions::manual_sort()),
        )
}

/// 菜单节标(分组方式/排序方式)
fn menu_section_label(label: &'static str) -> gpui_kit::AnyElement {
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
    label: &'static str,
    selected: bool,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static,
) -> gpui_kit::Stateful<gpui_kit::Div> {
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
            cx.stop_propagation();
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
                    dict::sessions::subagents_one(n)
                } else {
                    dict::sessions::subagents_other(n)
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
