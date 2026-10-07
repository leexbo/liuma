//! 浏览器标签正文:工具栏(后退/前进/刷新|停止/地址栏/视口切换/
//! 在系统浏览器打开)+ 错误条 + 正文。正文当前为占位空态(原生
//! webview 挂载见 native.rs,仅 macOS;非 macOS 恒占位)。

use gpui_kit::component::input::Input;
use gpui_kit::component::{IconName, StyledExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Entity, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::kits::icons::{LiumaIcon, fixed};
use crate::kits::theme;
use crate::shell::panel::BrowserTabId;
use crate::shell::store::AppStore;

use super::store::{NavCommand, ViewportPreset};

/// 浏览器标签正文(面板 tab_body 分发臂;根 size_full 填满)
pub(crate) fn render(
    store: &Entity<AppStore>,
    id: BrowserTabId,
    window: &mut Window,
    cx: &mut App,
) -> gpui_kit::AnyElement {
    // —— 突变相位(terminal render 同款模式)——
    // ensure:懒建档 + 地址栏输入/订阅;sync:地址栏回填。命令
    // (pending)由正文 mount canvas 的 paint 期平台层取走执行
    // (macOS 派发 WKWebView;其余平台丢弃落位)
    store.update(cx, |st, cx| {
        st.browser_ensure(id, window, cx);
        st.browser_addr_sync(id, window, cx);
    });
    let st = store.read(cx);
    let Some(tab) = st.browser.tab(id) else {
        return div().into_any_element();
    };
    let nav = tab.nav.clone();
    let viewport = tab.viewport;
    let addr_input = tab.addr_input.clone();

    div()
        .debug_selector(|| "panel-browser-view".to_string())
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .v_flex()
        .child(toolbar(store, id, &nav, viewport, addr_input, cx))
        .when_some(nav.error.clone(), |c, err| {
            c.child(
                div()
                    .id("browser-error")
                    .debug_selector(|| "browser-error".to_string())
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(12.))
                    .py(px(6.))
                    .text_size(px(12.))
                    .text_color(theme::warning(cx))
                    .border_b_1()
                    .border_color(theme::border(cx))
                    .child(fixed(IconName::TriangleAlert, 12.))
                    .child(div().min_w(px(0.)).child(err)),
            )
        })
        // 正文:有 URL = mount canvas(平台层挂 webview/派发命令);
        // 无 URL = 空态引导
        .child(if nav.url.is_empty() {
            empty_hint(cx).into_any_element()
        } else {
            let s_mount = store.clone();
            gpui_kit::canvas(
                // prepaint:无自定义绘制
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    super::native::paint_mount(id, &s_mount, bounds, window, cx);
                },
            )
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .into_any_element()
        })
        .into_any_element()
}

/// 工具栏行 36px:导航键(不可用态压暗)| 地址栏(flex_1)| 视口
/// 切换 pill | 系统浏览器外开
fn toolbar(
    store: &Entity<AppStore>,
    id: BrowserTabId,
    nav: &super::store::NavState,
    viewport: ViewportPreset,
    addr_input: Option<Entity<gpui_kit::component::input::InputState>>,
    cx: &App,
) -> impl IntoElement {
    let s_viewport = store.clone();
    let url = nav.url.clone();
    div()
        .id("browser-toolbar")
        .debug_selector(|| "browser-toolbar".to_string())
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(4.))
        .h(px(36.))
        .px(px(8.))
        .border_b_1()
        .border_color(theme::border(cx))
        .child(nav_button(
            "browser-back",
            IconName::ArrowLeft,
            nav.can_back,
            t_back(),
            {
                let s = store.clone();
                move |_, _, cx| {
                    s.update(cx, |st, cx| st.browser_navigate(id, NavCommand::Back, cx));
                }
            },
            cx,
        ))
        .child(nav_button(
            "browser-forward",
            IconName::ArrowRight,
            nav.can_fwd,
            t_forward(),
            {
                let s = store.clone();
                move |_, _, cx| {
                    s.update(cx, |st, cx| {
                        st.browser_navigate(id, NavCommand::Forward, cx)
                    });
                }
            },
            cx,
        ))
        .child(if nav.loading {
            nav_button(
                "browser-stop",
                IconName::CircleX,
                true,
                t_stop(),
                {
                    let s = store.clone();
                    move |_, _, cx| {
                        s.update(cx, |st, cx| st.browser_navigate(id, NavCommand::Stop, cx));
                    }
                },
                cx,
            )
            .into_any_element()
        } else {
            nav_button(
                "browser-reload",
                LiumaIcon::RefreshCw,
                !nav.url.is_empty(),
                t_reload(),
                {
                    let s = store.clone();
                    move |_, _, cx| {
                        s.update(cx, |st, cx| st.browser_navigate(id, NavCommand::Reload, cx));
                    }
                },
                cx,
            )
            .into_any_element()
        })
        // 地址栏:Enter 提交(订阅在 store 侧);InputState 未建(首帧
        // ensure 在本 render 突变相位,闭包读旧快照 None)时给占位空层,
        // 下帧起就位
        .child(
            div()
                .id("browser-addr")
                .debug_selector(|| "browser-addr".to_string())
                .flex_1()
                .min_w(px(0.))
                .when_some(addr_input, |c, input| {
                    c.child(Input::new(&input).h(px(26.)))
                }),
        )
        // 视口切换:文字 pill 显示当前档,点击循环(桌面 → 375 → 390 → 428)
        .child(
            div()
                .id("browser-viewport")
                .debug_selector(|| "browser-viewport".to_string())
                .flex()
                .flex_shrink_0()
                .items_center()
                .h(px(24.))
                .px(px(8.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_size(px(11.))
                .text_color(theme::caption(cx))
                .hover(|s| s.bg(theme::dock(cx)).text_color(theme::label(cx)))
                .child(viewport.label())
                .on_click(move |_, _, cx| {
                    s_viewport.update(cx, |st, cx| {
                        if let Some(tab) = st.browser.tab_mut(id) {
                            tab.viewport = tab.viewport.next();
                            cx.notify();
                        }
                    });
                }),
        )
        // 在系统浏览器打开(无 URL 时不可用)
        .when(!url.is_empty(), |c| {
            c.child(nav_button(
                "browser-external",
                IconName::ExternalLink,
                true,
                t_external(),
                move |_, _, cx| cx.open_url(&url),
                cx,
            ))
        })
}

/// 导航键方钮(不可用 = 压暗 + 点击无效)
fn nav_button(
    key: &'static str,
    icon: impl Into<gpui_kit::component::Icon>,
    enabled: bool,
    tooltip: String,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    div()
        .id(SharedString::from(key))
        .debug_selector(move || key.to_string())
        .flex()
        .flex_shrink_0()
        .size(px(26.))
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .text_color(if enabled {
            theme::label_2(cx)
        } else {
            theme::caption(cx)
        })
        .hover(|s| s.bg(theme::dock(cx)))
        .child(fixed(icon, 14.))
        .when(enabled, |c| {
            c.on_click(move |ev, w, cx| on_click(ev, w, cx))
        })
        .when(!tooltip.is_empty(), |c| {
            c.tooltip(crate::shell::tip(tooltip))
        })
}

/// 空态引导(未导航;加载/错误态由 webview 自渲染 + 错误条承担)
fn empty_hint(cx: &App) -> impl IntoElement {
    div()
        .debug_selector(|| "browser-body-placeholder".to_string())
        .flex_1()
        .min_h(px(0.))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.))
                .text_color(theme::caption(cx))
                .child(fixed(IconName::Globe, 14.))
                .child(crate::kits::i18n::t!("browser.empty_hint")),
        )
}

fn t_back() -> String {
    crate::kits::i18n::t!("browser.back").to_string()
}
fn t_forward() -> String {
    crate::kits::i18n::t!("browser.forward").to_string()
}
fn t_reload() -> String {
    crate::kits::i18n::t!("browser.reload").to_string()
}
fn t_stop() -> String {
    crate::kits::i18n::t!("browser.stop").to_string()
}
fn t_external() -> String {
    crate::kits::i18n::t!("browser.open_external").to_string()
}
