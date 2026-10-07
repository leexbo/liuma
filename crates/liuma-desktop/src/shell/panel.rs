//! 右侧面板:全高独立列,手动开关 +
//! 左缘拖宽 + 标签页壳。标签 = [`PanelTab`](当前仅「计划」,同类去重);
//! 无激活标签 = 居中快捷菜单,与「+」同份视图清单,面板不自动收。
//! ⇧⌘P = 开计划标签(应用首例键绑定:键表 shell::bind_global_keys +
//! WorkspaceView::new 注册的 App 级 on_action,无焦点可达)。后续视图
//! (浏览器/命令行类)加 `PanelTab` 变体即自动进「+」与空态清单。
//! 设置页整列接管时面板隐藏。

use gpui_kit::component::popover::Popover;
use gpui_kit::component::{IconName, InteractiveElementExt as _, StyledExt};
use gpui_kit::{
    Anchor, App, Entity, InteractiveElement, IntoElement, MouseButton, MouseMoveEvent,
    MouseUpEvent, ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, actions,
    div, px,
};

use crate::features::chat::{ChatNode, PlanStatus};
use crate::kits::icons::{LiumaIcon, fixed};
use crate::kits::popup::PopTrigger;
use crate::kits::theme;
use crate::shell::store::AppStore;

actions!(panel, [OpenPanelPlan]);

/// 终端标签 id(一路标签 = 一路 shell;`NEW` = 菜单哨兵,经
/// `open_panel_tab` 兑换为真实 id)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalTabId(pub u64);

impl TerminalTabId {
    pub const NEW: Self = Self(0);
}

/// 浏览器标签 id(一路标签 = 一只 webview 会话;`NEW` = 菜单哨兵,
/// 经 `open_panel_tab` 兑换为真实 id)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserTabId(pub u64);

impl BrowserTabId {
    pub const NEW: Self = Self(0);
}

/// 预览标签数据(工作区相对路径 + 行导航参数)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewTab {
    /// 目标文件(工作区相对路径)
    pub path: std::path::PathBuf,
    /// 跳行导航(1-based;入口带行时更新,重开同文件聚焦不重开)
    pub line: Option<u32>,
}

/// 面板标签页(静态种同类去重,序 = 打开序;Preview 按路径去重、
/// 只经文件树点击进入,不进「+」与空态清单;Terminal/Browser 按 id
/// 可多开,「+」与空态清单带 NEW 哨兵,每次点 = 新开一路)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelTab {
    /// 计划(当前会话最新计划只读)
    Plan,
    /// 轨迹(时间线/台账/检查器整页,原样呈现)
    Trajectory,
    /// 文件(工作区文件树,lazy 逐层装载;点文件开预览)
    Files,
    /// 终端(交互式 shell 会话,长驻至关闭标签;features::terminal)
    Terminal(TerminalTabId),
    /// 浏览器(应用内网页视图;features::browser)
    Browser(BrowserTabId),
    /// 文档预览(渲染器注册表见 kits::filetype)
    Preview(PreviewTab),
}

impl PanelTab {
    /// 全部静态标签(「+」菜单与空态清单共用的视图源;Preview 不列;
    /// Terminal/Browser 携 NEW 哨兵)
    pub const ALL: [PanelTab; 5] = [
        PanelTab::Plan,
        PanelTab::Trajectory,
        PanelTab::Files,
        PanelTab::Terminal(TerminalTabId::NEW),
        PanelTab::Browser(BrowserTabId::NEW),
    ];

    /// 是否文件树标签(切会话换根门控判据)
    pub fn is_files(&self) -> bool {
        matches!(self, PanelTab::Files)
    }

    /// 标签标题(Preview = 文件名;Terminal = OSC Title,回落 cwd
    /// 目录名,再回落通用名)
    pub fn title(&self, store: &AppStore) -> String {
        use crate::kits::i18n::t;
        match self {
            PanelTab::Plan => t!("shell.plan_tab").to_string(),
            PanelTab::Trajectory => t!("shell.trajectory_tab").to_string(),
            PanelTab::Files => t!("shell.files_tab").to_string(),
            PanelTab::Terminal(id) => store
                .terminal
                .tab(*id)
                .and_then(|tab| {
                    tab.title.clone().or_else(|| {
                        tab.session
                            .as_ref()
                            .and_then(|s| s.cwd.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                    })
                })
                .unwrap_or_else(|| t!("shell.terminal_tab").to_string()),
            PanelTab::Browser(id) => store
                .browser
                .tab(*id)
                .and_then(|tab| tab.nav.title.clone())
                .unwrap_or_else(|| t!("shell.browser_tab").to_string()),
            PanelTab::Preview(p) => p
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| t!("shell.preview_tab").to_string()),
        }
    }

    /// 定尺寸图标(标签 pill 13 / 清单行 16;Preview = 类型染色)
    pub fn icon(&self, size: f32) -> gpui_kit::component::Icon {
        match self {
            PanelTab::Plan => fixed(LiumaIcon::ListChecks, size),
            PanelTab::Trajectory => fixed(LiumaIcon::Trajectory, size),
            PanelTab::Files => fixed(LiumaIcon::FolderTree, size),
            PanelTab::Terminal(_) => fixed(IconName::SquareTerminal, size),
            PanelTab::Browser(_) => fixed(IconName::Globe, size),
            PanelTab::Preview(p) => {
                let name = p
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let class = crate::kits::filetype::file_class(&name);
                crate::kits::filetype::class_icon(class, size)
                    .text_color(theme::file_type_tint(class))
            }
        }
    }

    /// 稳定 id / debug selector 片段(Preview 带路径哈希、Terminal 带
    /// 标签 id,防同域撞名)
    fn key(&self) -> String {
        match self {
            PanelTab::Plan => "plan".to_string(),
            PanelTab::Trajectory => "trajectory".to_string(),
            PanelTab::Files => "files".to_string(),
            PanelTab::Terminal(id) => format!("terminal-{}", id.0),
            PanelTab::Browser(id) => format!("browser-{}", id.0),
            PanelTab::Preview(p) => {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                p.path.hash(&mut hasher);
                format!("preview-{:016x}", hasher.finish())
            }
        }
    }
}

/// 面板列(根 flex 第三子节点;settings 整列接管 / 收起时不渲染)
pub fn render(store: &Entity<AppStore>, window: &mut Window, cx: &mut App) -> impl IntoElement {
    let viewport_w = f32::from(window.viewport_size().width);
    let (col_w, resizing, tabs, active_tab) = {
        let st = store.read(cx);
        (
            f32::from(crate::shell::metrics::panel_width_for(
                st.panel_open,
                st.panel_px,
                viewport_w,
                st.sidebar_collapsed,
                st.sidebar_px,
            )),
            st.panel_resize_anchor.is_some(),
            st.panel_tabs.clone(),
            st.panel_active_tab.clone(),
        )
    };
    if col_w <= 0. {
        return div().into_any_element();
    }
    // 快捷键徽标文案从键表生成(单一事实源,不手写;未绑定时回落 action 名)
    let shortcut = window.keystroke_text_for(&OpenPanelPlan);
    let (s_resize, s_resize_move) = (store.clone(), store.clone());
    let mut col = div()
        .debug_selector(|| "right-panel".to_string())
        .relative()
        .flex_shrink_0()
        .h_full()
        .w(px(col_w))
        .v_flex()
        // 面板与聊天区同底(透 Root 毛玻璃涂层,双模式一致)——分隔
        // 只靠左侧发丝线;深色下取 SIDEBAR 亮一档会显「灰底」
        .border_l_1()
        .border_color(theme::border(cx))
        // 右栏域尾哨兵(栈底,盖整个面板列):右栏拖选落空时终点钳在
        // 右栏域,同时压住窗口级聊天哨兵在本列的命中(后注册居上)
        .child(
            div()
                .absolute()
                .size_full()
                .child(crate::shell::SelectionDomainSink::new(
                    "sel-sink-panel",
                    crate::kits::selection_order::PANEL_TAIL_ORDER,
                )),
        )
        .child(panel_header(store, &tabs, active_tab.clone(), cx))
        .child(match active_tab {
            Some(tab) => tab_body(store, tab, window, cx),
            None => empty_menu(store, &shortcut, cx),
        });
    // 左缘拖宽把手(absolute 外沿 4px;同 sidebar_resize_handle)
    col = col.child(
        div()
            .id("panel-resize")
            .debug_selector(|| "panel-resize".to_string())
            .absolute()
            .left(px(-4.))
            .top(px(0.))
            .bottom(px(0.))
            .w(px(8.))
            .cursor(gpui_kit::CursorStyle::ResizeLeftRight)
            .on_mouse_down(
                MouseButton::Left,
                move |ev: &gpui_kit::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    let vp = f32::from(window.viewport_size().width);
                    s_resize.update(cx, |st, cx| {
                        st.panel_resize_begin(ev.position.x.as_f32(), vp, cx);
                    });
                },
            ),
    );
    // 拖拽 move/up 窗口级注册(仅拖拽中挂;渲染期 canvas 注册,同侧栏
    // drag_overlay:move 带锚点守卫,up 独立注册保证抬起必收尾)
    if resizing {
        let m = s_resize_move.clone();
        let u = s_resize_move.clone();
        col = col.child(
            gpui_kit::canvas(
                // prepaint:无自定义绘制
                |_, _, _| (),
                move |_, _, window, cx| {
                    if m.read(cx).panel_resize_anchor.is_some() {
                        let m2 = m.clone();
                        window.on_mouse_event(move |ev: &MouseMoveEvent, _, _, cx| {
                            m2.update(cx, |st, cx| {
                                st.panel_resize_move(ev.position.x.as_f32(), cx)
                            });
                        });
                    }
                    let u2 = u.clone();
                    window.on_mouse_event(move |_: &MouseUpEvent, _, _, cx| {
                        u2.update(cx, |st, cx| st.panel_resize_end(cx));
                    });
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
    }
    col.into_any_element()
}

/// 面板头部 40px(与内容区标题行同高对齐;36→40 给
/// 标签卡上下留间隙):窗口拖拽 + 双击缩放;左段 = 标签条 + 紧随的「+」
/// (视图清单菜单)——**无标签时左段全空**(不渲染默认「面板」标题与
/// 「+」,开视图走正文居中的空态清单);右缘 = 面板开关(内容区标题栏钮的开态形态整体挪入,点击收起;关态
/// 时同款钮回标题栏——原 ✕ 关闭钮撤除)。头部空白处 mousedown 即拖;
/// 交互子件自挂 mousedown stop_propagation,点击不触发窗口拖拽
/// (同 drag_strip「区域内无交互子元素」约定的子件侧豁免)
fn panel_header(
    store: &Entity<AppStore>,
    tabs: &[PanelTab],
    active: Option<PanelTab>,
    cx: &App,
) -> impl IntoElement {
    let mut strip: Vec<gpui_kit::AnyElement> = Vec::new();
    for tab in tabs {
        strip.push(
            panel_tab_pill(store, tab.clone(), Some(tab.clone()) == active, cx).into_any_element(),
        );
    }
    let s_toggle = store.clone();
    if !tabs.is_empty() {
        strip.push(
            // 「+」清单菜单(组件库 Popover 托管开态/外点关闭/下方向左
            // 展开;快捷键文案由内容闭包的 window 现取)
            Popover::new("panel-plus-pop")
                .appearance(false)
                .anchor(Anchor::TopRight)
                .trigger(PopTrigger(
                    div()
                        .id("panel-plus")
                        .debug_selector(|| "panel-plus".to_string())
                        .flex()
                        .size(px(22.))
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .text_color(theme::caption(cx))
                        .hover(|s| s.bg(theme::dock(cx)).text_color(theme::label(cx)))
                        .child(fixed(IconName::Plus, 13.)),
                ))
                .content({
                    let store = store.clone();
                    move |_, window, cx| {
                        let shortcut = window.keystroke_text_for(&OpenPanelPlan);
                        let pop = cx.entity();
                        plus_menu_card(&store, pop, shortcut, cx).into_any_element()
                    }
                })
                .into_any_element(),
        );
    }
    div()
        .id("panel-drag")
        .flex()
        .flex_shrink_0()
        .h(px(34.))
        .items_center()
        .on_mouse_down(MouseButton::Left, |_, window, _| {
            window.start_window_move();
        })
        .on_double_click(|_, window, _| {
            window.titlebar_double_click();
        })
        .child(
            div()
                .flex()
                .items_center()
                .flex_1()
                .min_w(px(0.))
                .h_full()
                .pl(px(10.))
                .overflow_hidden()
                .children(strip),
        )
        .child(
            div().flex().items_center().pr(px(10.)).child(
                div()
                    .id("panel-toggle")
                    .debug_selector(|| "panel-toggle".to_string())
                    .flex()
                    .size(px(26.))
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    // 开态形态(同 topbar 面板开关,不用蓝
                    // 色):前景提亮 + 素底,hover 显灰
                    .text_color(theme::label(cx))
                    .hover(|s| s.bg(theme::layer(cx)))
                    .child(fixed(IconName::PanelRight, 14.))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, _, cx| {
                        s_toggle.update(cx, |st, cx| st.toggle_panel(cx));
                    }),
            ),
        )
}

/// 标签卡:图标 + 标题 + ✕ 方钮(浏览器标签语言)。
/// 激活 = LAYER 圆角卡底 + 一级字;✕ = 卡内右侧**常显**独立
/// 圆角小方钮(深盘 DOCK 比卡底亮一档/浅盘白 CARD 浮起——两模式
/// 分层语言),未激活素底 hover 才显底(触发钮全站约定);
/// ✕ 独立点击(stop_propagation,不冒泡激活标签也不触发窗口拖拽)
fn panel_tab_pill(
    store: &Entity<AppStore>,
    tab: PanelTab,
    active: bool,
    cx: &App,
) -> gpui_kit::AnyElement {
    let s_tab = store.clone();
    let s_close = store.clone();
    let key = tab.key();
    let pill = tab.clone();
    let closer = tab.clone();
    div()
        .id(SharedString::from(format!("panel-tab-{key}")))
        .debug_selector(move || format!("panel-tab-{}", pill.key()))
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(8.))
        .h(px(30.))
        .mr(px(6.))
        .pl(px(14.))
        .pr(px(10.))
        .rounded(px(8.))
        .cursor_pointer()
        .bg(if active {
            theme::layer(cx)
        } else {
            theme::TRANSPARENT
        })
        .text_color(if active {
            theme::label(cx)
        } else {
            theme::label_3(cx)
        })
        .hover(|s| s.bg(theme::layer(cx)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(tab.icon(14.))
        .child(div().text_size(px(13.)).child(tab.title(store.read(cx))))
        .child(
            div()
                .id(SharedString::from(format!("panel-tab-close-{key}")))
                .ml(px(8.))
                .flex()
                .size(px(20.))
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .bg(match active {
                    true if theme::is_dark(cx) => theme::dock(cx),
                    true => theme::card(cx),
                    false => theme::TRANSPARENT,
                })
                .text_color(theme::caption(cx))
                .hover(|s| s.bg(theme::dock(cx)).text_color(theme::label(cx)))
                .child(fixed(IconName::Close, 11.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    s_close.update(cx, |st, cx| st.close_panel_tab(closer.clone(), cx));
                }),
        )
        .on_click(move |_, _, cx| {
            s_tab.update(cx, |st, cx| st.activate_panel_tab(tab.clone(), cx));
        })
        .into_any_element()
}

/// 标签正文:计划 = 最新计划快照只读;轨迹 = 整页视图自主区迁入(原样:
/// 工具栏 + 时间线 + 台账|检查器并排,检查器拖宽/时间线选区照旧)。
/// 文件 = 文件树视图;预览 = 渲染器分发视图。
/// 外层 flex_1 承接面板列剩余高度,各视图根 size_full 填满
fn tab_body(
    store: &Entity<AppStore>,
    tab: PanelTab,
    window: &mut Window,
    cx: &mut App,
) -> gpui_kit::AnyElement {
    match tab {
        PanelTab::Plan => {
            let latest = {
                let st = store.read(cx);
                let session_id = st.state.current_id.clone();
                session_id.as_deref().and_then(|sid| {
                    st.state.chats.get(sid).and_then(|c| {
                        c.nodes.iter().rev().find_map(|n| match n {
                            ChatNode::Plan { plan, status, .. } => Some((plan.clone(), *status)),
                            _ => None,
                        })
                    })
                })
            };
            div()
                .id("panel-body")
                .debug_selector(|| "panel-plan-view".to_string())
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .px(px(12.))
                .py(px(10.))
                .child(panel_plan_content(latest, cx))
                .into_any_element()
        }
        PanelTab::Trajectory => div()
            .debug_selector(|| "panel-trajectory-view".to_string())
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .child(crate::features::trajectory::render(store, window, cx))
            .into_any_element(),
        PanelTab::Files => div()
            .debug_selector(|| "panel-files-view".to_string())
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .child(crate::features::files::render(store, window, cx))
            .into_any_element(),
        PanelTab::Terminal(id) => div()
            .debug_selector(|| "panel-terminal-view".to_string())
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .child(crate::features::terminal::render(store, id, window, cx))
            .into_any_element(),
        PanelTab::Browser(id) => div()
            .debug_selector(|| "panel-browser-view".to_string())
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .child(crate::features::browser::render(store, id, window, cx))
            .into_any_element(),
        PanelTab::Preview(preview) => div()
            .debug_selector(|| format!("panel-preview-view-{}", preview.path.display()))
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .child(crate::features::preview::render(
                store, &preview, window, cx,
            ))
            .into_any_element(),
    }
}

/// 空态快捷菜单(面板开着、无激活标签):居中卡,
/// 每行 = 图标 + 标题 + 右侧快捷键徽标;与「+」菜单同份 [`PanelTab::ALL`]
fn empty_menu(store: &Entity<AppStore>, shortcut: &str, cx: &App) -> gpui_kit::AnyElement {
    let rows: Vec<gpui_kit::AnyElement> = PanelTab::ALL
        .iter()
        .map(|tab| {
            let s = store.clone();
            let open = tab.clone();
            div()
                .id(SharedString::from(format!("panel-empty-row-{}", tab.key())))
                .debug_selector(move || format!("panel-empty-row-{}", tab.key()))
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(40.))
                .px(px(14.))
                .rounded(px(10.))
                .cursor_pointer()
                // 行面分模式:浅盘素底 hover 才灰(灰胶囊行在白面板上成
                // 灰砖);深盘微亮面 LAYER——纯素底在
                // 深面板上无边界,取深色行极弱亮面
                .bg(if theme::is_dark(cx) {
                    theme::layer(cx)
                } else {
                    theme::TRANSPARENT
                })
                .hover(|s| s.bg(theme::dock(cx)))
                .child(tab.icon(16.).text_color(theme::label_2(cx)))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme::label(cx))
                        .child(tab.title(store.read(cx))),
                )
                .child(div().flex_1())
                .child(
                    // 快捷键徽标:小灰 pill,须比行面亮一档才可读
                    div()
                        .rounded(px(5.))
                        .bg(if theme::is_dark(cx) {
                            theme::dock(cx)
                        } else {
                            theme::layer(cx)
                        })
                        .px(px(6.))
                        .py(px(2.))
                        .text_size(px(11.))
                        .text_color(theme::caption(cx))
                        .child(shortcut.to_string()),
                )
                .on_click(move |_, _, cx| {
                    s.update(cx, |st, cx| st.open_panel_tab(open.clone(), cx))
                })
                .into_any_element()
        })
        .collect();
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .debug_selector(|| "panel-empty-menu".to_string())
                .v_flex()
                .w(px(320.))
                .gap(px(8.))
                .children(rows),
        )
        .into_any_element()
}

/// 面板「+」菜单(组件库 Popover 内容;点项即开 tab 并收起菜单)。
/// 项与空态菜单同份 [`PanelTab::ALL`]
fn plus_menu_card(
    store: &Entity<AppStore>,
    pop: Entity<gpui_kit::component::popover::PopoverState>,
    shortcut: String,
    cx: &App,
) -> impl IntoElement {
    let items: Vec<gpui_kit::AnyElement> = PanelTab::ALL
        .iter()
        .map(|tab| {
            let s = store.clone();
            let shortcut = shortcut.clone();
            let pop = pop.clone();
            let open = tab.clone();
            div()
                .id(SharedString::from(format!("panel-plus-item-{}", tab.key())))
                .debug_selector(move || format!("panel-plus-item-{}", tab.key()))
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(26.))
                .px(px(8.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_color(theme::label_2(cx))
                .hover(|st| st.bg(theme::dock(cx)))
                .child(tab.icon(13.))
                .child(div().text_size(px(12.)).child(tab.title(store.read(cx))))
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme::caption(cx))
                        .child(shortcut.to_string()),
                )
                .on_click(move |_, window, cx| {
                    let pop = pop.clone();
                    s.update(cx, |st, cx| st.open_panel_tab(open.clone(), cx));
                    pop.update(cx, |state, cx| state.dismiss(window, cx));
                })
                .into_any_element()
        })
        .collect();
    div()
        .id("panel-plus-menu")
        .debug_selector(|| "panel-plus-menu".to_string())
        .v_flex()
        .w(px(176.))
        .gap(px(2.))
        .rounded(px(10.))
        .border_1()
        .border_color(theme::border(cx))
        .bg(theme::layer(cx))
        .p(px(4.))
        .shadow_md()
        .children(items)
}

/// 计划 tab 内容:最新计划全文 + 状态徽标;无计划 = 空态
fn panel_plan_content(latest: Option<(String, PlanStatus)>, cx: &App) -> impl IntoElement {
    let mut col = div().v_flex().gap(px(8.));
    match latest {
        None => {
            col = col.child(
                div()
                    .py(px(20.))
                    .text_size(px(12.))
                    .text_color(theme::caption(cx))
                    .child(crate::kits::i18n::t!("shell.plan_empty")),
            );
        }
        Some((plan, status)) => {
            use crate::kits::i18n::t;
            let (status_text, status_color) = match status {
                PlanStatus::Pending => (t!("shell.plan_pending"), theme::warning(cx)),
                PlanStatus::Approved => (t!("shell.plan_approved"), theme::success(cx)),
                PlanStatus::Declined => (t!("shell.plan_declined"), theme::caption(cx)),
                PlanStatus::Cancelled => (t!("shell.plan_cancelled"), theme::caption(cx)),
            };
            col = col
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .text_color(theme::label_2(cx))
                                .child(crate::kits::i18n::t!("shell.plan_tab")),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(status_color)
                                .child(status_text),
                        ),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme::label_2(cx))
                        .child(crate::kits::markdown_tv::tv_static("panel-plan", &plan, cx)),
                );
        }
    }
    col
}
