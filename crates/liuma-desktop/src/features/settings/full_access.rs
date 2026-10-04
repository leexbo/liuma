//! 全权模式:确认/取消动作与风险确认弹层。

use super::*;

impl AppStore {
    /// 确认 full-access(风险确认后按来源分流:默认预设落盘 /
    /// composer 会话权限切换)
    pub fn confirm_full_access(&mut self, cx: &mut Context<Self>) {
        let ask = self.settings.full_access_confirm.take();
        if ask == Some(FullAccessAsk::Session) {
            self.set_session_permission("full-access", cx);
            return;
        }
        if let Err(e) = self
            .bridge
            .host()
            .set_default_permission_preset("full-access")
        {
            self.push_local_notice(t!("settings.save_failed", msg = &e.message), cx);
            return;
        }
        self.settings_refresh(cx);
    }

    /// 取消 full-access 确认(设置页来源时 Select 显示回滚到实际值;
    /// composer 来源无 Select,仅关弹窗)
    pub fn cancel_full_access(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.full_access_confirm = None;
        let actual = self.settings.settings_snapshot["defaultPermission"]
            .as_str()
            .unwrap_or("workspace-write");
        let label = match actual {
            "read-only" => t!("settings.perm_read_only"),
            "workspace-write" => t!("settings.perm_workspace_write"),
            "full-access" => t!("settings.perm_full_access"),
            other => other.into(),
        };
        if let Some(select) = &self.settings.permission_select {
            let v = gpui_kit::SharedString::from(label.to_string());
            select.update(cx, |s, cx| s.set_selected_value(&v, window, cx));
        }
        cx.notify();
    }
}

/// full-access 风险确认(组件库 Dialog 层;store 经 with_window 桥打开,
/// 设置页与 composer 会话权限两来源共用)。警示标题 + 后果段落 +
/// 能力清单盒 + 风险脚注 + 取消/红色确认;Esc/遮罩/X 走 on_close 统一
/// 回滚(旗标归位 + 设置页 Select 显示回实际值)
pub(crate) fn open_full_access_dialog(store: &Entity<AppStore>, window: &mut Window, cx: &mut App) {
    use gpui_kit::component::WindowExt as _;
    let (s_cancel, s_confirm, s_close) = (store.clone(), store.clone(), store.clone());
    window.open_dialog(cx, move |dialog, _, cx| {
        let (s_cancel, s_confirm, s_close) = (s_cancel.clone(), s_confirm.clone(), s_close.clone());
        dialog
            .title(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(fixed(IconName::TriangleAlert, 18.).text_color(theme::label(cx)))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(theme::label(cx))
                            .child(t!("settings.fa_title")),
                    ),
            )
            .w(px(440.))
            .bg(theme::layer(cx))
            .content(|content, _, cx| {
                content.child(
                    div()
                        .id("full-access-card")
                        .debug_selector(|| "full-access-card".to_string())
                        .v_flex()
                        .gap(px(14.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(theme::label_2(cx))
                                .child(t!("settings.fa_body")),
                        )
                        .child(
                            div()
                                .id("full-access-list")
                                .debug_selector(|| "full-access-list".to_string())
                                .v_flex()
                                .rounded(px(10.))
                                .bg(theme::dock(cx))
                                .child(risk_row(
                                    fixed(IconName::Folder, 16.),
                                    t!("settings.fa_files"),
                                    t!("settings.fa_files_desc"),
                                    cx,
                                ))
                                .child(div().w_full().h(px(1.)).bg(theme::border(cx)))
                                .child(risk_row(
                                    fixed(IconName::SquareTerminal, 16.),
                                    t!("settings.fa_terminal"),
                                    t!("settings.fa_terminal_desc"),
                                    cx,
                                ))
                                .child(div().w_full().h(px(1.)).bg(theme::border(cx)))
                                .child(risk_row(
                                    fixed(IconName::Globe, 16.),
                                    t!("settings.fa_internet"),
                                    t!("settings.fa_internet_desc"),
                                    cx,
                                )),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme::caption(cx))
                                .child(t!("settings.fa_risk")),
                        ),
                )
            })
            .footer(
                div()
                    .flex()
                    .justify_end()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("full-access-cancel")
                            .debug_selector(|| "full-access-cancel".to_string())
                            .flex()
                            .h(px(32.))
                            .items_center()
                            .px(px(14.))
                            .rounded(px(16.))
                            .border_1()
                            .border_color(theme::border(cx))
                            .cursor_pointer()
                            .text_size(px(13.))
                            .text_color(theme::label_2(cx))
                            .hover(|s| s.bg(theme::dock(cx)))
                            .child(t!("common.cancel"))
                            .on_click(move |_, window, cx| {
                                s_cancel.update(cx, |st, cx| st.cancel_full_access(window, cx));
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        div()
                            .id("full-access-confirm")
                            .debug_selector(|| "full-access-confirm".to_string())
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .h(px(32.))
                            .px(px(14.))
                            .rounded(px(16.))
                            .bg(gpui_kit::Rgba {
                                a: 0.14,
                                ..theme::danger(cx)
                            })
                            .cursor_pointer()
                            .text_size(px(13.))
                            .text_color(theme::danger(cx))
                            .hover(|s| {
                                s.bg(gpui_kit::Rgba {
                                    a: 0.22,
                                    ..theme::danger(cx)
                                })
                            })
                            .child(fixed(IconName::TriangleAlert, 13.))
                            .child(t!("common.confirm"))
                            .on_click(move |_, window, cx| {
                                s_confirm.update(cx, |st, cx| st.confirm_full_access(cx));
                                window.close_dialog(cx);
                            }),
                    ),
            )
            .on_close(move |_, window, cx| {
                s_close.update(cx, |st, cx| st.cancel_full_access(window, cx));
            })
    });
}

/// 风险确认弹窗能力行(图标 + 标题 + 灰描述;文本列 flex_1 换行,
/// 图标顶对齐标题线)
pub(crate) fn risk_row(
    icon: gpui_kit::component::Icon,
    title: impl Into<gpui_kit::SharedString>,
    desc: impl Into<gpui_kit::SharedString>,
    cx: &App,
) -> impl IntoElement {
    let title = title.into();
    let desc = desc.into();
    div()
        .flex()
        .items_start()
        .gap(px(10.))
        .px(px(12.))
        .py(px(10.))
        .child(icon.text_color(theme::label_2(cx)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .v_flex()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme::label(cx))
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme::caption(cx))
                        .child(desc.to_string()),
                ),
        )
}
