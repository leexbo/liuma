//! 「在编辑器中打开」视图:顶栏右缘分体钮(主钮 = 选中应用直开 +
//! chevron = 应用菜单)+ 菜单行 + 启动失败 toast。弹层走组件库
//! Popover + overlay_card 惯例(见 workspace_trigger 同款形态)。

use gpui_kit::component::IconName;
use gpui_kit::component::popover::Popover;
use gpui_kit::{
    Anchor, App, Entity, ImageSource, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, img, px,
};

use crate::features::opener::store::OpenerStore;
use crate::kits::i18n::t;
use crate::kits::icons::fixed;
use crate::kits::modals::overlay_card;
use crate::kits::popup::PopTrigger;
use crate::kits::theme;
use crate::shell::store::AppStore;

/// 应用显示名:访达/终端走翻译真值,品牌名走双语同形键(登记在
/// verify-desktop-i18n 的 SAME_VALUE_KEYS);清单封闭,兜底回原始 id
pub(crate) fn display_name(id: &str) -> String {
    match id {
        "finder" => t!("opener.app_finder").to_string(),
        "terminal" => t!("opener.app_terminal").to_string(),
        "cursor" => t!("opener.app_cursor").to_string(),
        "vscode" => t!("opener.app_vscode").to_string(),
        "vscodeinsiders" => t!("opener.app_vscodeinsiders").to_string(),
        "windsurf" => t!("opener.app_windsurf").to_string(),
        "zed" => t!("opener.app_zed").to_string(),
        "sublimetext" => t!("opener.app_sublimetext").to_string(),
        "xcode" => t!("opener.app_xcode").to_string(),
        "androidstudio" => t!("opener.app_androidstudio").to_string(),
        "intellij" => t!("opener.app_intellij").to_string(),
        "pycharm" => t!("opener.app_pycharm").to_string(),
        "webstorm" => t!("opener.app_webstorm").to_string(),
        "phpstorm" => t!("opener.app_phpstorm").to_string(),
        "goland" => t!("opener.app_goland").to_string(),
        "rider" => t!("opener.app_rider").to_string(),
        "rustrover" => t!("opener.app_rustrover").to_string(),
        "fork" => t!("opener.app_fork").to_string(),
        "sourcetree" => t!("opener.app_sourcetree").to_string(),
        "tower" => t!("opener.app_tower").to_string(),
        "sublimemerge" => t!("opener.app_sublimemerge").to_string(),
        "ghostty" => t!("opener.app_ghostty").to_string(),
        "warp" => t!("opener.app_warp").to_string(),
        "iterm" => t!("opener.app_iterm").to_string(),
        "kitty" => t!("opener.app_kitty").to_string(),
        other => other.to_string(),
    }
}

/// 可见性三条件收敛:macOS + 当前会话有工作区目录 + 清单已就绪。
/// 非 macOS apps 恒空,三重保险之一。
pub(crate) fn split_button_visible(st: &AppStore) -> bool {
    cfg!(target_os = "macos") && st.current_workspace_dir().is_some() && !st.opener.apps.is_empty()
}

/// 应用真身图标(提取未到/失败回落通用外链图标)
fn app_icon(icons: &OpenerStore, id: Option<&'static str>, size: f32) -> impl IntoElement {
    match id.and_then(|id| icons.icons.get(&id)).cloned().flatten() {
        // img 默认 object_fit = Contain,真身图标比例无忧
        Some(render) => div()
            .child(img(ImageSource::Render(render)).size(px(size)))
            .into_any_element(),
        None => fixed(IconName::ExternalLink, size).into_any_element(),
    }
}

/// 顶栏右缘分体钮:主钮(真身图标,点击 = 选中应用直开)+ 中缝 +
/// chevron(受控 Popover 应用菜单)。26px 与顶栏控件同高;两半共享
/// hover 底;标题栏内可点元素必须 occlude(Windows 拖拽豁免)。
pub(crate) fn open_with_split(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let selected = st
        .opener
        .selected
        .and_then(|id| st.opener.apps.iter().find(|a| a.id == id).map(|a| a.id));
    let tip_open = t!(
        "opener.tip_open_with",
        app = display_name(selected.unwrap_or("finder"))
    )
    .to_string();
    let s_main = store.clone();
    let s_menu = store.clone();
    div()
        .debug_selector(|| "open-with".to_string())
        .flex()
        .h(px(26.))
        .flex_shrink_0()
        .items_center()
        .rounded(px(8.))
        // 两半共享的 hover 高亮(外壳画底,半区自身不再变色)
        .hover(|s| s.bg(theme::layer(cx)))
        .text_color(theme::label_3(cx))
        // 主钮半区:非 Popover,直接 on_click
        .child(
            div()
                .id("open-with-main")
                .debug_selector(|| "open-with-main".to_string())
                .flex()
                .items_center()
                .justify_center()
                .h_full()
                .px(px(6.))
                .cursor_pointer()
                .occlude()
                .tooltip(crate::shell::tip(tip_open))
                .child(app_icon(&st.opener, selected, 16.))
                .on_click(move |_, _, cx| {
                    s_main.update(cx, |st, cx| st.opener_open_current(cx));
                }),
        )
        // 中缝分隔线
        .child(
            div()
                .w(px(1.))
                .h(px(14.))
                .flex_shrink_0()
                .bg(theme::border(cx)),
        )
        // chevron 半区:受控 Popover。开合全走 store 单一事实源:
        // on_click toggle;外点关闭由菜单卡自持 on_mouse_down_out
        // (gpui 捕获相先于 bubble 相,会与本钮 on_click 竞态——关掉
        // 库的 overlay_closable,捕获相自关并留 gesture_dismissed 标记,
        // on_click 见标记不再重开,见 OpenerStore 字段注)
        .child({
            let open = st.opener.menu_open;
            Popover::new("open-with-pop")
                .appearance(false)
                .anchor(Anchor::TopRight)
                .overlay_closable(false)
                .open(open)
                .on_open_change({
                    let s_open = store.clone();
                    move |open, _, cx| {
                        s_open.update(cx, |st, _| st.opener.menu_open = *open);
                    }
                })
                .trigger(PopTrigger(
                    div()
                        .id("open-with-chevron")
                        .debug_selector(|| "open-with-chevron".to_string())
                        .flex()
                        .items_center()
                        .justify_center()
                        .h_full()
                        .px(px(4.))
                        .cursor_pointer()
                        .occlude()
                        .tooltip(crate::shell::tip(t!("opener.tip_choose_app").to_string()))
                        .child(fixed(IconName::ChevronDown, 12.))
                        .on_click({
                            let s_toggle = store.clone();
                            move |_, _, cx| {
                                s_toggle.update(cx, |st, cx| {
                                    if st.opener.gesture_dismissed {
                                        st.opener.gesture_dismissed = false;
                                    } else {
                                        st.opener.menu_open = !st.opener.menu_open;
                                    }
                                    cx.notify();
                                });
                            }
                        }),
                ))
                .content({
                    let s_out = store.clone();
                    move |_, _, cx| {
                        let pop = cx.entity();
                        overlay_card("open-with-card", 240., app_menu_rows(&s_menu, pop, cx), cx)
                            // 外点关闭(捕获相):关菜单并留手势标记
                            .on_mouse_down_out({
                                let s_out = s_out.clone();
                                move |_, _, cx| {
                                    s_out.update(cx, |st, cx| {
                                        if st.opener.menu_open {
                                            st.opener.menu_open = false;
                                            st.opener.gesture_dismissed = true;
                                            cx.notify();
                                        }
                                    });
                                }
                            })
                            .into_any_element()
                    }
                })
        })
}

/// 应用菜单行(照 workspace_menu_rows 语言):32px 行、图标 + 名称、
/// 选中项带「（默认）」后缀、hover dock 底;点击 = 收起 + 记住 + 打开
fn app_menu_rows(
    store: &Entity<AppStore>,
    pop: Entity<gpui_kit::component::popover::PopoverState>,
    cx: &App,
) -> Vec<gpui_kit::AnyElement> {
    let st = store.read(cx);
    st.opener
        .apps
        .iter()
        .map(|app| {
            let is_selected = st.opener.selected == Some(app.id);
            let label = if is_selected {
                t!("opener.app_default_label", name = display_name(app.id)).to_string()
            } else {
                display_name(app.id)
            };
            let sel = format!("open-with-row-{}", app.id);
            let s = store.clone();
            let pop = pop.clone();
            let id = app.id;
            let icon = app_icon(&st.opener, Some(id), 16.);
            div()
                .id(SharedString::from(sel.clone()))
                .debug_selector(move || sel.clone())
                .flex()
                .h(px(32.))
                .items_center()
                .gap(px(8.))
                .rounded(px(6.))
                .px(px(8.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::dock(cx)))
                .text_size(px(13.))
                .text_color(if is_selected {
                    theme::label(cx)
                } else {
                    theme::label_2(cx)
                })
                .child(icon)
                .child(div().min_w(px(0.)).flex_1().truncate().child(label))
                .on_click(move |_, window, cx| {
                    pop.update(cx, |state, cx| state.dismiss(window, cx));
                    s.update(cx, |st, cx| st.opener_open_app(id, cx));
                })
                .into_any_element()
        })
        .collect()
}

/// 启动失败 toast(照 attachment_toast_card 形态:全屏点关层 + 底部
/// 居中 pill;3s 定时自清由 store 兜底)
pub(crate) fn launch_error_toast(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let Some(text) = st.opener.launch_error.clone() else {
        return div().into_any_element();
    };
    let s = store.clone();
    div()
        .id("opener-toast")
        .debug_selector(|| "opener-toast".to_string())
        .absolute()
        .size_full()
        .top_0()
        .left_0()
        .flex()
        .items_end()
        .justify_center()
        .pb(px(120.))
        .on_click(move |_, _, cx| {
            s.update(cx, |st, _| st.opener_dismiss_launch_error());
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(12.))
                .bg(gpui_kit::Rgba {
                    a: 0.95,
                    ..theme::layer(cx)
                })
                .border_1()
                .border_color(theme::border(cx))
                .px(px(14.))
                .py(px(10.))
                .text_size(px(13.))
                .text_color(theme::label(cx))
                .child(fixed(IconName::TriangleAlert, 14.))
                .child(text),
        )
        .into_any_element()
}
