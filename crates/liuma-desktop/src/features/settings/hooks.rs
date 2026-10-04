//! Hooks 桥管理页:详情表单/开关/移除与视图。

use super::*;

/// Hooks 分区表单态(新增/编辑共享;M4.2)
#[derive(Clone)]
pub struct HooksDetailState {
    /// 编辑中的桥 id(None = 新增;编辑态 id 锁定)
    pub editing: Option<String>,
    /// 启用开关表单值
    pub form_enabled: bool,
    /// 方言选择(claude-code / codex)
    pub form_dialect: String,
    /// config 路径输入
    pub form_config_path: Option<Entity<InputState>>,
    /// pluginRoot 输入(CC;可选)
    pub form_plugin_root: Option<Entity<InputState>>,
    /// projectDir 输入(CC;可选,缺省 = 会话工作区)
    pub form_project_dir: Option<Entity<InputState>>,
    /// 缺省超时 MS 输入(空 = 600000)
    pub form_timeout: Option<Entity<InputState>>,
}

impl AppStore {
    /// 打开 Hooks 详情页(新增模式)
    pub fn open_hooks_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.hooks_detail = Some(HooksDetailState {
            editing: None,
            form_enabled: true,
            form_dialect: "claude-code".to_string(),
            form_config_path: Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("settings.hooks_path_placeholder"))
            })),
            form_plugin_root: Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("settings.hooks_root_placeholder"))
            })),
            form_project_dir: Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("settings.hooks_cwd_placeholder"))
            })),
            form_timeout: Some(cx.new(|cx| InputState::new(window, cx).placeholder("600000"))),
        });
        cx.notify();
    }

    /// 打开 Hooks 详情页(编辑模式:按 id 预填)
    pub fn open_hooks_edit(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.settings.settings_snapshot["hookBridges"]
            .as_array()
            .and_then(|list| list.iter().find(|e| e["id"] == *id).cloned())
            .and_then(|v| serde_json::from_value::<liuma_core::settings::HookBridgeEntry>(v).ok())
        else {
            return;
        };
        let state = HooksDetailState {
            editing: Some(entry.id.clone()),
            form_enabled: entry.enabled,
            form_dialect: entry.dialect.clone(),
            form_config_path: Some(cx.new(|cx| InputState::new(window, cx))),
            form_plugin_root: Some(cx.new(|cx| InputState::new(window, cx))),
            form_project_dir: Some(cx.new(|cx| InputState::new(window, cx))),
            form_timeout: Some(cx.new(|cx| InputState::new(window, cx))),
        };
        let Some(d) = self.settings.hooks_detail.as_mut() else {
            return;
        };
        *d = state;
        let Some(d) = self.settings.hooks_detail.as_ref() else {
            return;
        };
        if let Some(inp) = &d.form_config_path {
            inp.update(cx, |s2, cx| {
                s2.set_value(entry.config_path.clone(), window, cx)
            });
        }
        if let (Some(inp), Some(v)) = (&d.form_plugin_root, &entry.plugin_root) {
            inp.update(cx, |s2, cx| s2.set_value(v.clone(), window, cx));
        }
        if let (Some(inp), Some(v)) = (&d.form_project_dir, &entry.project_dir) {
            inp.update(cx, |s2, cx| s2.set_value(v.clone(), window, cx));
        }
        if let (Some(inp), Some(v)) = (&d.form_timeout, entry.default_timeout_ms) {
            inp.update(cx, |s2, cx| s2.set_value(v.to_string(), window, cx));
        }
        cx.notify();
    }

    /// 关闭 Hooks 详情页(回列表)
    pub fn close_hooks_detail(&mut self, cx: &mut Context<Self>) {
        self.settings.hooks_detail = None;
        cx.notify();
    }

    /// 翻转 Hooks 方言(表单)
    pub fn set_hooks_dialect(&mut self, dialect: &str, cx: &mut Context<Self>) {
        if let Some(d) = self.settings.hooks_detail.as_mut() {
            d.form_dialect = dialect.to_string();
        }
        cx.notify();
    }

    /// 翻转 Hooks 启用开关(表单)
    pub fn toggle_hooks_form_enabled(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.settings.hooks_detail.as_mut() {
            d.form_enabled = !d.form_enabled;
        }
        cx.notify();
    }

    /// 保存 Hooks 表单(upsert;变更 = 下次 attach 生效)
    pub fn save_hooks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.settings.hooks_detail.clone() else {
            return;
        };
        let read =
            |inp: &Option<Entity<InputState>>, window: &mut Window, cx: &mut Context<Self>| {
                let _ = window;
                inp.as_ref()
                    .map(|e| e.read(cx).value().trim().to_string())
                    .unwrap_or_default()
            };
        let config_path = read(&d.form_config_path, window, cx);
        let plugin_root = read(&d.form_plugin_root, window, cx);
        let project_dir = read(&d.form_project_dir, window, cx);
        let timeout_raw = read(&d.form_timeout, window, cx);
        let id = d.editing.clone().unwrap_or_else(|| {
            format!(
                "hooks-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_micros() as u64)
                    .unwrap_or(0)
            )
        });
        let entry = liuma_core::settings::HookBridgeEntry {
            id,
            dialect: d.form_dialect.clone(),
            config_path,
            enabled: d.form_enabled,
            plugin_root: (!plugin_root.is_empty()).then_some(plugin_root),
            project_dir: (!project_dir.is_empty()).then_some(project_dir),
            default_timeout_ms: timeout_raw.parse::<u64>().ok(),
            stderr_summary_max_chars: None,
        };
        match self.bridge.host().upsert_hook_bridge(entry) {
            Ok(()) => {
                self.settings.hooks_detail = None;
                self.settings_refresh(cx);
            }
            Err(e) => {
                self.settings.settings_notice = Some((false, e.message));
            }
        }
        cx.notify();
    }

    /// 启停 hooks 桥(enabled 翻转)
    pub fn toggle_hook_bridge(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(mut entry) = self.settings.settings_snapshot["hookBridges"]
            .as_array()
            .and_then(|list| list.iter().find(|e| e["id"] == *id).cloned())
            .and_then(|v| serde_json::from_value::<liuma_core::settings::HookBridgeEntry>(v).ok())
        else {
            return;
        };
        entry.enabled = !entry.enabled;
        if self.bridge.host().upsert_hook_bridge(entry).is_ok() {
            self.settings_refresh(cx);
        }
        cx.notify();
    }

    /// 卸载 hooks 桥
    pub fn remove_hook_bridge(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.bridge.host().remove_hook_bridge(id).is_ok() {
            self.settings_refresh(cx);
        }
        cx.notify();
    }
}

/// Hooks 区(Claude Code / Codex 桥;M4.2):行卡(id/方言/路径/启停/
/// 编辑/卸载)+ 添加卡 + 详情表单。视觉语言复刻 MCP Servers 区。
pub(crate) fn hooks_section(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let st_add = store.clone();
    let Some(detail) = st.settings.hooks_detail.clone() else {
        // ── 列表页 ──
        let bridges = st.settings.settings_snapshot["hookBridges"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut col = div()
            .v_flex()
            .gap(px(12.))
            .child(section_title(t!("settings.nav_hooks")))
            .child(intro_line(t!("settings.hooks_intro"), cx));
        let total = bridges.len();
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .mt(px(12.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_size(px(11.))
                        .text_color(theme::caption(cx))
                        .child(t!("settings.installed", total = total)),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("hooks-add")
                        .debug_selector(|| "hooks-add".to_string())
                        .flex()
                        .h(px(28.))
                        .items_center()
                        .px(px(12.))
                        .rounded(px(8.))
                        .bg(theme::label(cx))
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(theme::INK)
                        .hover(|s| s.opacity(0.9))
                        .on_click({
                            let st_open = st_add.clone();
                            move |_, window, cx| {
                                st_open.update(cx, |st, cx| st.open_hooks_add(window, cx));
                            }
                        })
                        .child(t!("settings.add_new")),
                ),
        );
        let mut rows = div().v_flex().gap(px(8.));
        if bridges.is_empty() {
            rows = rows.child(caption_line(t!("settings.hooks_none"), cx));
        }
        for (ix, b) in bridges.into_iter().enumerate() {
            let id = b["id"].as_str().unwrap_or_default().to_string();
            let dialect = b["dialect"].as_str().unwrap_or_default().to_string();
            let config_path = b["configPath"].as_str().unwrap_or_default().to_string();
            let enabled = b["enabled"].as_bool().unwrap_or(false);
            let (st_switch, st_edit, st_remove) = (store.clone(), store.clone(), store.clone());
            let (id_switch_click, id_edit_click, id_remove_click) =
                (id.clone(), id.clone(), id.clone());
            let (id_sw_dbg, id_sw_click) = (id_switch_click.clone(), id_switch_click.clone());
            let (id_ed_dbg, id_ed_click) = (id_edit_click.clone(), id_edit_click.clone());
            let (id_rm_dbg, id_rm_click) = (id_remove_click.clone(), id_remove_click.clone());
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(10.))
                    .bg(theme::layer(cx))
                    .px(px(12.))
                    .py(px(10.))
                    .child(
                        div()
                            .v_flex()
                            .gap(px(2.))
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                                    .text_color(theme::label(cx))
                                    .child(id_switch_click.clone()),
                            )
                            .child(
                                div().flex().items_center().gap(px(6.)).child(
                                    div()
                                        .min_w(px(0.))
                                        .text_size(px(11.))
                                        .text_color(theme::caption(cx))
                                        .child(format!("{dialect} · {config_path}")),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .id(("hooks-switch", ix))
                            .debug_selector(move || format!("hooks-switch-{id_sw_dbg}"))
                            .child(
                                Switch::new(("hooks-switch-toggle", ix))
                                    .checked(enabled)
                                    .color(theme::label(cx))
                                    .on_click({
                                        let st_switch = st_switch.clone();
                                        let id_sw = id_sw_click.clone();
                                        move |_, _, cx| {
                                            st_switch.update(cx, |st, cx| {
                                                st.toggle_hook_bridge(&id_sw, cx)
                                            });
                                        }
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .id(("hooks-edit", ix))
                            .debug_selector(move || format!("hooks-edit-{id_ed_dbg}"))
                            .flex()
                            .h(px(22.))
                            .items_center()
                            .px(px(8.))
                            .rounded(px(11.))
                            .cursor_pointer()
                            .text_size(px(11.))
                            .text_color(theme::label_2(cx))
                            .hover(|s| s.bg(theme::dock(cx)))
                            .on_click(move |_, window, cx| {
                                st_edit.update(cx, |st, cx| {
                                    st.open_hooks_edit(&id_ed_click, window, cx)
                                });
                            })
                            .child(t!("common.edit")),
                    )
                    .child(
                        div()
                            .id(("hooks-remove", ix))
                            .debug_selector(move || format!("hooks-remove-{id_rm_dbg}"))
                            .flex()
                            .h(px(22.))
                            .items_center()
                            .px(px(8.))
                            .rounded(px(11.))
                            .cursor_pointer()
                            .text_size(px(11.))
                            .text_color(theme::danger(cx))
                            .hover(|s| s.bg(theme::dock(cx)))
                            .on_click(move |_, _, cx| {
                                st_remove
                                    .update(cx, |st, cx| st.remove_hook_bridge(&id_rm_click, cx));
                            })
                            .child(t!("common.uninstall")),
                    ),
            );
        }
        return col.child(rows).into_any_element();
    };

    // ── 详情页(新增/编辑表单)──
    let st_back = store.clone();
    let st_save = store.clone();
    let st_dialect_cc = store.clone();
    let st_dialect_codex = store.clone();
    let st_toggle = store.clone();
    let dialect_sel = |d: &HooksDetailState, want: &str| d.form_dialect == want;
    let mut col = div()
        .v_flex()
        .gap(px(12.))
        .child(section_title(if detail.editing.is_some() {
            t!("settings.hooks_edit_card")
        } else {
            t!("settings.hooks_new_card")
        }))
        .children(st.settings.settings_notice.as_ref().map(|(ok, msg)| {
            div()
                .debug_selector(|| "hooks-detail-notice".to_string())
                .text_size(px(12.))
                .text_color(if *ok {
                    theme::success(cx)
                } else {
                    theme::danger(cx)
                })
                .child(format!("{} {msg}", if *ok { "✓" } else { "⚠" }))
        }))
        .child(
            div()
                .id("hooks-back")
                .flex()
                .h(px(28.))
                .w(px(72.))
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .bg(theme::layer(cx))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(theme::label_2(cx))
                .on_click({
                    let st_back = st_back.clone();
                    move |_, _, cx| {
                        st_back.update(cx, |st, cx| st.close_hooks_detail(cx));
                    }
                })
                .child(t!("settings.back_arrow")),
        );
    // 方言
    {
        let (cc_sel, codex_sel) = (
            dialect_sel(&detail, "claude-code"),
            dialect_sel(&detail, "codex"),
        );
        col = col
            .child(section_title(t!("settings.dialect_section")))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("hooks-dialect-cc")
                            .flex()
                            .h(px(28.))
                            .items_center()
                            .px(px(12.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(if cc_sel {
                                gpui_kit::white()
                            } else {
                                theme::label_2(cx).into()
                            })
                            .bg(if cc_sel {
                                theme::brand(cx)
                            } else {
                                theme::layer(cx)
                            })
                            .on_click({
                                let st_dialect_cc = st_dialect_cc.clone();
                                move |_, _, cx| {
                                    st_dialect_cc.update(cx, |st, cx| {
                                        st.set_hooks_dialect("claude-code", cx)
                                    });
                                }
                            })
                            .child("claude-code"),
                    )
                    .child(
                        div()
                            .id("hooks-dialect-codex")
                            .flex()
                            .h(px(28.))
                            .items_center()
                            .px(px(12.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(if codex_sel {
                                gpui_kit::white()
                            } else {
                                theme::label_2(cx).into()
                            })
                            .bg(if codex_sel {
                                theme::brand(cx)
                            } else {
                                theme::layer(cx)
                            })
                            .on_click({
                                let st_dialect_codex = st_dialect_codex.clone();
                                move |_, _, cx| {
                                    st_dialect_codex
                                        .update(cx, |st, cx| st.set_hooks_dialect("codex", cx));
                                }
                            })
                            .child("codex"),
                    ),
            );
    }
    // 启用开关
    col = col
        .child(section_title(t!("settings.enable_section")))
        .child(
            Switch::new("hooks-form-enabled")
                .small()
                .checked(detail.form_enabled)
                .color(theme::label(cx))
                .on_click({
                    let st_toggle = st_toggle.clone();
                    move |_, _, cx| {
                        st_toggle.update(cx, |st, cx| st.toggle_hooks_form_enabled(cx));
                    }
                }),
        );
    // 字段
    col = col
        .child(section_title(t!("settings.config_section")))
        .child(field_input(
            t!("settings.path"),
            "hooks-form-path",
            &detail.form_config_path,
            cx,
        ))
        .child(field_input(
            "pluginRoot",
            "hooks-form-plugin-root",
            &detail.form_plugin_root,
            cx,
        ))
        .child(field_input(
            "projectDir",
            "hooks-form-project-dir",
            &detail.form_project_dir,
            cx,
        ))
        .child(field_input(
            t!("settings.timeout_ms_hooks"),
            "hooks-form-timeout",
            &detail.form_timeout,
            cx,
        ))
        .child(
            div()
                .id("hooks-save")
                .debug_selector(|| "hooks-save".to_string())
                .flex()
                .h(px(32.))
                .w(px(96.))
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .bg(theme::label(cx))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(theme::INK)
                .hover(|s| s.opacity(0.9))
                .on_click({
                    let st_save = st_save.clone();
                    move |_, window, cx| {
                        st_save.update(cx, |st, cx| st.save_hooks(window, cx));
                    }
                })
                .child(t!("common.save")),
        );
    col.into_any_element()
}
