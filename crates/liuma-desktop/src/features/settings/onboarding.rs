//! 首次运行引导:动作(save/later)与密钥弹层视图。

use super::*;

impl AppStore {
    /// onboarding 重估:任一 provider 凭据可用 = 引导完成
    /// (全部缺席才弹「添加一个 API Key」模态)
    pub fn recalc_onboarding(&mut self) {
        let onboarded = self.settings.settings_snapshot["onboarded"]
            .as_bool()
            .unwrap_or(false);
        let any_ready = self.settings.settings_snapshot["providers"]
            .as_array()
            .map(|ps| {
                ps.iter()
                    .any(|p| p["credentialReady"].as_bool().unwrap_or(false))
            })
            .unwrap_or(false);
        self.settings.needs_onboarding = !onboarded && !any_ready;
        if !self.settings.needs_onboarding && !onboarded {
            let _ = self.bridge.host().set_onboarded();
        }
    }

    /// onboarding 模态「保存并继续」:key 写入默认 provider(deepseek)
    /// 并完成引导;空 key = 内联错误
    pub fn onboarding_save(&mut self, cx: &mut Context<Self>) {
        let key = self
            .settings
            .onboarding_key_input
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
            .unwrap_or_default();
        if key.is_empty() {
            self.settings.onboarding_key_error = Some(t!("settings.onboarding_key_empty").into());
            cx.notify();
            return;
        }
        self.settings.onboarding_key_error = None;
        let mut entry = self
            .bridge
            .host()
            .providers()
            .into_iter()
            .find(|p| p.id == "deepseek")
            .unwrap_or_else(liuma_core::settings::builtin_provider);
        entry.api_key = Some(key);
        if let Err(e) = self.bridge.host().upsert_provider(entry) {
            self.settings.onboarding_key_error = Some(e.message);
            cx.notify();
            return;
        }
        let _ = self.bridge.host().set_onboarded();
        self.settings_refresh(cx);
        cx.notify();
    }

    /// onboarding 模态「稍后配置」:完成引导,不再弹
    pub fn onboarding_later(&mut self, cx: &mut Context<Self>) {
        self.settings.onboarding_key_error = None;
        let _ = self.bridge.host().set_onboarded();
        self.settings_refresh(cx);
    }
}

/// 首运行 onboarding 模态:无任何可用
/// 凭据时弹出,默认 deepseek provider,只填 key。稍后配置 = 完成引导
/// (不再弹);保存并继续 = key 写入 deepseek 并完成
pub(crate) fn onboarding_modal(store: &Entity<AppStore>, cx: &App) -> gpui_kit::AnyElement {
    let st = store.read(cx);
    let (s_save, s_later) = (store.clone(), store.clone());
    let mut card = div()
        .id("onboarding-card")
        .debug_selector(|| "onboarding-card".to_string())
        .v_flex()
        .w(px(460.))
        .gap(px(14.))
        .rounded(px(14.))
        .border_1()
        .border_color(theme::border(cx))
        .bg(theme::layer(cx))
        .p(px(24.))
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation()
        })
        .child(
            div()
                .text_size(px(17.))
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(theme::label(cx))
                .child(t!("settings.onboarding_title")),
        )
        .child(
            div()
                .text_size(px(13.))
                .text_color(theme::label_2(cx))
                .child(t!("settings.onboarding_desc")),
        )
        .child(
            div()
                .v_flex()
                .gap(px(6.))
                .child(field_label(t!("settings.api_key"), cx))
                .children(st.settings.onboarding_key_input.as_ref().map(|e| {
                    div()
                        .id("onboarding-key")
                        .debug_selector(|| "onboarding-key".to_string())
                        .child(Input::new(e))
                })),
        );
    if let Some(err) = &st.settings.onboarding_key_error {
        card = card.child(
            div()
                .id("onboarding-error")
                .debug_selector(|| "onboarding-error".to_string())
                .text_size(px(12.))
                .text_color(theme::danger(cx))
                .child(err.clone()),
        );
    }
    card = card.child(
        div()
            .flex()
            .justify_end()
            .gap(px(10.))
            .child(
                div()
                    .id("onboarding-later")
                    .debug_selector(|| "onboarding-later".to_string())
                    .flex()
                    .h(px(36.))
                    .items_center()
                    .px(px(16.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(theme::border(cx))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .text_color(theme::label_2(cx))
                    .hover(|s| s.bg(theme::dock(cx)))
                    .child(t!("settings.onboarding_later"))
                    .on_click(move |_, _, cx| {
                        s_later.update(cx, |st, cx| st.onboarding_later(cx));
                    }),
            )
            .child(
                div()
                    .id("onboarding-save")
                    .debug_selector(|| "onboarding-save".to_string())
                    .flex()
                    .h(px(36.))
                    .items_center()
                    .px(px(16.))
                    .rounded(px(10.))
                    .bg(theme::brand(cx))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .text_color(theme::label(cx))
                    .hover(|s| s.opacity(0.9))
                    .child(t!("settings.onboarding_save"))
                    .on_click(move |_, _, cx| {
                        s_save.update(cx, |st, cx| st.onboarding_save(cx));
                    }),
            ),
    );
    div()
        .id("onboarding-overlay")
        .debug_selector(|| "onboarding-overlay".to_string())
        .absolute()
        .size_full()
        .top_0()
        .left_0()
        .flex()
        .items_start()
        .justify_center()
        .pt(px(140.))
        .bg(gpui_kit::Rgba {
            a: 0.6,
            ..theme::base(cx)
        })
        .child(card)
        .into_any_element()
}
