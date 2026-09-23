//! Hero 空态:居中输入卡 + 工作区/模式 chip 行
//! (列宽由根布局按 metrics 策略给定)。空会话时由根布局挂载。
//! 两 chip 均为实装下拉:工作区 = 切换/添加(标题栏下拉同源行),
//! 模式 = preset 选择(describe presets)。

use gpui_kit::component::Icon;
use gpui_kit::component::IconName;
use gpui_kit::component::StyledExt;
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, App, Entity, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::features::chat::composer;
use crate::kits::icons::{self, fixed};
use crate::kits::modals::{overlay_card, workspace_menu_rows};
use crate::kits::popup::PopTrigger;
use crate::kits::theme;
use crate::shell::store::AppStore;

/// Hero 整体(col_w:统一对话列宽,由根布局给定)
pub fn render(
    store: &Entity<AppStore>,
    col_w: gpui_kit::Pixels,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let st = store.read(cx);
    let ws = st
        .state
        .active_workspace
        .clone()
        .unwrap_or_else(|| st.default_workspace());
    let cfg = st.current_cfg_or_default();
    let preset_label = st.preset_label(&cfg.preset);

    div()
        .relative()
        .flex()
        .min_h(px(0.))
        .flex_1()
        .flex_col()
        .items_center()
        .justify_center()
        // 列对齐容器同款槽 padding(左锚点槽/右滚动条槽)——hero 主卡
        // 与 composer/消息列同中心线
        .pl(px(
            crate::shell::metrics::H_PAD + crate::shell::metrics::NAV_GUTTER_W
        ))
        .pr(px(
            crate::shell::metrics::H_PAD + crate::shell::metrics::SCROLLBAR_GUTTER_W
        ))
        .pb(px(64.))
        .child(
            div()
                .v_flex()
                .relative()
                .w(col_w)
                .gap(px(12.))
                // 品牌流马 logo + 标语(logo.svg)
                .child(
                    div()
                        .v_flex()
                        .items_center()
                        .gap(px(10.))
                        .child(
                            div().flex().justify_center().child(
                                fixed(crate::kits::icons::LiumaIcon::Logo, 64.)
                                    .text_color(theme::LABEL()),
                            ),
                        )
                        .child(
                            div()
                                .text_size(px(15.))
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .text_color(theme::LABEL())
                                .child(crate::kits::i18n::dict::shell::hero_tagline()),
                        ),
                )
                // 首运行引导:凭据未配置时给出去设置的入口
                .when(st.settings.needs_onboarding, |el| {
                    let s = store.clone();
                    el.child(
                        div()
                            .id("hero-onboarding")
                            .flex()
                            .h(px(28.))
                            .items_center()
                            .gap(px(6.))
                            .px(px(10.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(theme::BORDER())
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(theme::WARN())
                            .hover(|s| s.bg(theme::LAYER()))
                            .child(crate::kits::i18n::dict::shell::hero_no_key())
                            .on_click(move |_, _, cx| {
                                s.update(cx, |st, cx| st.toggle_settings(cx));
                            }),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(hero_ws_popover(
                            store,
                            &ws,
                            fixed(IconName::FolderOpen, 14.),
                        ))
                        .child(hero_preset_popover(
                            store,
                            "hero-preset",
                            &preset_label,
                            fixed(icons::LiumaIcon::AgentPreset, 14.),
                        )),
                )
                .child(composer::render(store, window, cx)), // 工作区/模式两 chip 各自弹组件库 Popover(内容闭包捕
                                                             // 获自身行组;开态/外点关闭/定位由库托管,不再有
                                                             // hero_menu 旗标与硬编码 top 偏移)
        )
}

/// 模式(preset)菜单(组件库 Popover 内容;选中即切换并收起菜单):
/// describe presets(id/name/description;当前勾选)
fn preset_card(
    store: &Entity<AppStore>,
    pop: Entity<PopoverState>,
    cx: &App,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let (current, presets) = {
        let st = store.read(cx);
        (
            st.current_cfg_or_default().preset,
            st.state.host_info.presets.clone(),
        )
    };
    let mut rows: Vec<gpui_kit::AnyElement> = vec![];
    for (ix, p) in presets.iter().enumerate() {
        let Some(id) = p["id"].as_str() else { continue };
        let name = p["name"].as_str().unwrap_or(id);
        let desc = p["description"].as_str().unwrap_or_default();
        let s = store.clone();
        let v = id.to_string();
        let pop = pop.clone();
        let checked = id == current;
        let sel = format!("preset-item-{id}");
        rows.push(
            div()
                .id(("preset-item", ix))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(10.))
                .px(px(10.))
                .py(px(8.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::DOCK()))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .v_flex()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .line_height(gpui_kit::relative(1.4))
                                .text_color(theme::LABEL())
                                .child(name.to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .line_height(gpui_kit::relative(1.3))
                                .text_color(theme::CAPTION())
                                .child(desc.to_string()),
                        ),
                )
                .when(checked, |el| {
                    el.child(fixed(IconName::Check, 14.).text_color(theme::LABEL()))
                })
                .debug_selector(move || sel.clone())
                .on_click(move |_, window, cx| {
                    pop.update(cx, |state, cx| state.dismiss(window, cx));
                    let v = v.clone();
                    s.update(cx, |st, cx| st.set_session_preset(&v, cx));
                })
                .into_any_element(),
        );
    }
    overlay_card("hero-preset-card", 320., rows)
        // 布局回归锁锚点(layout_tests hero_preset_select 按 bounds
        // 断言卡片不叠 chip 行;debug_bounds 只认 selector 不认 id)
        .debug_selector(|| "hero-preset-card".to_string())
}

/// Hero 态 chip(工作区/模式触发钮;图标 + 文字,点击弹下拉)
fn hero_chip(id: &'static str, label: &str, icon: Icon) -> gpui_kit::Stateful<gpui_kit::Div> {
    let sel = id;
    let label = label.to_string();
    div()
        .id(id)
        .flex()
        .h(px(28.))
        .items_center()
        .gap(px(4.))
        .rounded(px(16.))
        .px(px(8.))
        .text_size(px(13.))
        .font_weight(gpui_kit::FontWeight::MEDIUM)
        .text_color(theme::LABEL())
        .cursor_pointer()
        // 素底 hover 才显灰(触发钮语言,同 composer chip)
        .hover(|s| s.bg(theme::DOCK()))
        .child(icon)
        .child(label)
        .child(fixed(IconName::ChevronDown, 12.).text_color(theme::CAPTION()))
        .debug_selector(move || sel.to_string())
}

/// 工作区 chip 弹层(组件库 Popover;行组与标题栏下拉共用)
fn hero_ws_popover(store: &Entity<AppStore>, ws: &str, icon: Icon) -> impl IntoElement {
    let s_card = store.clone();
    Popover::new("hero-ws-pop")
        .appearance(false)
        .anchor(Anchor::TopLeft)
        .trigger(PopTrigger(hero_chip("hero-ws", ws, icon)))
        .content(move |_, _, cx| {
            let pop = cx.entity();
            overlay_card("hero-ws-card", 320., workspace_menu_rows(&s_card, pop, cx))
                .into_any_element()
        })
}

/// 模式 chip 弹层(组件库 Popover)
fn hero_preset_popover(
    store: &Entity<AppStore>,
    id: &'static str,
    label: &str,
    icon: Icon,
) -> impl IntoElement {
    let s_card = store.clone();
    Popover::new("hero-preset-pop")
        .appearance(false)
        .anchor(Anchor::TopLeft)
        .trigger(PopTrigger(hero_chip(id, label, icon)))
        .content(move |_, _, cx| {
            let pop = cx.entity();
            preset_card(&s_card, pop, cx).into_any_element()
        })
}
