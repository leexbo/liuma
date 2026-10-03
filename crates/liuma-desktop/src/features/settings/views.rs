//! 设置页骨架:render 两栏壳与侧栏菜单。各页视图在各自 *_views 文件。

use super::*;

/// 设置页内容(右列整列:顶部窄拖拽条 + 滚动内容;导航在左侧栏的
/// 设置菜单里)
pub fn render(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    div()
        .id("settings-page")
        .v_flex()
        .size_full()
        // 画布透 Root 毛玻璃涂层(同聊天区,不再自铺 base 叠涂)
        .debug_selector(|| "settings-page".to_string())
        // 顶部拖拽条(交通灯在左列;右列拖拽由此接手,无可见 chrome;
        // 高度与主标题行/右栏面板头 40 同高对齐)
        .child(
            div()
                .id("settings-drag")
                .flex()
                .flex_shrink_0()
                .h(px(40.))
                .cursor(gpui_kit::CursorStyle::Arrow)
                .on_mouse_down(gpui_kit::MouseButton::Left, |_, window, _| {
                    window.start_window_move();
                })
                .on_double_click(|_, window, _| {
                    window.titlebar_double_click();
                }),
        )
        // 归档页:自带内滚(flex_1 约束高度 + 列表内滚),不套外层滚动;
        // 其余页:block 布局 + overflow_y_scroll,内容溢出外层滚
        .child(
            div()
                .id("settings-options")
                .flex_1()
                .min_h(px(0.))
                .px(px(16.))
                .py(px(16.))
                .when(
                    store.read(cx).settings.settings_nav == SettingsNav::ArchivedChats,
                    |el| {
                        el.child(
                            div()
                                .v_flex()
                                .w(px(720.))
                                .h_full()
                                .min_h(px(0.))
                                .mx_auto()
                                .child(archived_section(store, cx).into_any_element()),
                        )
                    },
                )
                .when(
                    store.read(cx).settings.settings_nav != SettingsNav::ArchivedChats,
                    |el| {
                        el.overflow_y_scroll().child(
                            div()
                                .id("settings-scroll")
                                .v_flex()
                                .w(px(720.))
                                .mx_auto()
                                .child(match store.read(cx).settings.settings_nav {
                                    SettingsNav::Models => {
                                        models_section(store, cx).into_any_element()
                                    }
                                    SettingsNav::Mcp => mcp_section(store, cx).into_any_element(),
                                    SettingsNav::Hooks => {
                                        hooks_section(store, cx).into_any_element()
                                    }
                                    SettingsNav::Decision => {
                                        decision_section(store, cx).into_any_element()
                                    }
                                    SettingsNav::General => {
                                        general_section(store, cx).into_any_element()
                                    }
                                    SettingsNav::ArchivedChats => div().into_any_element(),
                                    SettingsNav::About => {
                                        about_section(store, cx).into_any_element()
                                    }
                                }),
                        )
                    },
                ),
        )
}

/// 设置模式侧栏:顶部「返回工作区」+ 标题「设置」+ **分组导航**(
/// 基础设置/Agent 能力/数据与统计三组头,
/// 仅渲染已有实现的项——组内无实装项则不显示空组头)+ 底部「关于」。
/// 图标行形态:左图标右标签、激活整行 pill(DOCK),组头为
/// 小号说明字。插件/MCP/技能待实装后进各自组——入口迁移优于新增。
pub(crate) fn menu(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    use super::SettingsNav;
    let st = store.read(cx);
    let back = store.clone();
    // 「基础设置」组:常规 / 模型设置(模型行名为「模型」);
    // 「数据与统计」组承载归档会话管理,见 basic 数组之后
    let basic: [(
        SettingsNav,
        std::borrow::Cow<'static, str>,
        gpui_kit::component::Icon,
    ); 5] = [
        (
            SettingsNav::General,
            t!("settings.general"),
            fixed(LiumaIcon::General, 15.),
        ),
        (
            SettingsNav::Models,
            t!("settings.nav_models"),
            fixed(LiumaIcon::Models, 15.),
        ),
        (
            SettingsNav::Mcp,
            t!("settings.nav_mcp"),
            fixed(LiumaIcon::Mcp, 15.),
        ),
        (
            SettingsNav::Hooks,
            t!("settings.nav_hooks"),
            fixed(LiumaIcon::Hook, 15.),
        ),
        (
            SettingsNav::Decision,
            t!("settings.nav_decision"),
            fixed(LiumaIcon::Decision, 15.),
        ),
    ];
    let mut list = div().v_flex().gap(px(4.));
    list = list.child(nav_group_header(t!("settings.nav_basics")));
    for (nav, label, icon) in basic {
        list = list.child(nav_item(store, nav, label, icon, st.settings.settings_nav));
    }
    // 「数据与统计」组:归档会话管理
    list = list.child(nav_group_header(t!("settings.nav_data")));
    list = list.child(nav_item(
        store,
        SettingsNav::ArchivedChats,
        t!("settings.nav_archived"),
        fixed(LiumaIcon::Archive, 15.),
        st.settings.settings_nav,
    ));
    div()
        .id("settings-menu")
        .debug_selector(|| "settings-menu".to_string())
        .v_flex()
        .h_full()
        .w(crate::shell::metrics::sidebar_width_for(
            false,
            st.sidebar_px,
        ))
        .flex_shrink_0()
        .bg(theme::SIDEBAR())
        .border_r_1()
        .border_color(theme::BORDER())
        .px(px(12.))
        .pt(px(36.))
        .pb(px(10.))
        .gap(px(8.))
        .child(crate::features::sessions::drag_strip())
        // 返回工作区按钮(整行显式按钮;标题「设置」独立一行在下)
        .child(
            div()
                .id("settings-back")
                .debug_selector(|| "settings-back".to_string())
                .flex()
                .h(px(32.))
                .items_center()
                .gap(px(6.))
                .rounded(px(8.))
                .px(px(8.))
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(theme::LABEL_2())
                .hover(|s| s.bg(theme::LAYER()))
                .child(fixed(IconName::ArrowLeft, 15.))
                .child(t!("settings.back_workspace"))
                .on_click(move |_, window, cx| {
                    back.update(cx, |st, cx| st.toggle_settings(window, cx));
                }),
        )
        .child(list)
        // 底部「关于」(RS 专有页;独立于三组,置底兜底)
        .child(div().v_flex().gap(px(4.)).child(nav_item(
            store,
            SettingsNav::About,
            t!("settings.about"),
            fixed(IconName::Info, 15.),
            st.settings.settings_nav,
        )))
}

/// 分组导航组头(小号说明字)
fn nav_group_header(text: impl Into<gpui_kit::SharedString>) -> impl IntoElement {
    let text = text.into();
    div()
        .px(px(8.))
        .pt(px(8.))
        .pb(px(4.))
        .text_size(px(12.))
        .text_color(theme::CAPTION())
        .child(text.to_string())
}

/// 导航项行:左图标右标签,激活整行 pill(DOCK)高亮
fn nav_item(
    store: &Entity<AppStore>,
    nav: SettingsNav,
    label: impl Into<gpui_kit::SharedString>,
    icon: gpui_kit::component::Icon,
    current: SettingsNav,
) -> impl IntoElement {
    let label = label.into();
    let s = store.clone();
    let active = current == nav;
    let sel = format!("settings-nav-{}", nav.slug());
    div()
        .id(gpui_kit::SharedString::from(sel.clone()))
        .debug_selector(move || sel.clone())
        .flex()
        .h(px(36.))
        .items_center()
        .gap(px(8.))
        .rounded(px(8.))
        .px(px(8.))
        .cursor_pointer()
        .when(active, |el| el.bg(theme::DOCK()))
        .when(!active, |el| {
            el.hover(|s| s.bg(theme::LAYER()))
                .text_color(theme::LABEL_3())
        })
        .text_size(px(13.))
        .text_color(if active {
            theme::LABEL()
        } else {
            theme::LABEL_3()
        })
        .when(active, |el| el.font_weight(gpui_kit::FontWeight::MEDIUM))
        .child(icon)
        .child(label)
        .on_click(move |_, window, cx| {
            s.update(cx, |st, cx| st.set_settings_nav(nav, window, cx));
        })
}

/// 底部设置行(打开设置页;折叠 rail 的展开态对应物)
pub(crate) fn settings_row(store: &Entity<AppStore>) -> impl IntoElement {
    let s = store.clone();
    div()
        .id("settings")
        .debug_selector(|| "settings-row".to_string())
        .flex()
        .h(px(32.))
        .flex_shrink_0()
        .items_center()
        .rounded(px(8.))
        .px(px(8.))
        .gap(px(8.))
        .cursor_pointer()
        .hover(|s| s.bg(theme::LAYER()))
        .text_size(px(13.))
        .text_color(theme::LABEL_3())
        .child(fixed(IconName::Settings, 16.))
        .child(t!("settings.settings_title"))
        .on_click(move |_, window, cx| {
            s.update(cx, |st, cx| st.toggle_settings(window, cx));
        })
}
