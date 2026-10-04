//! 常规设置页:偏好下拉/开关/语言/外观方法与面板、关于。

use super::*;

impl AppStore {
    /// 通用区偏好下拉构建(gpui-component Select;Confirm → 按 label 映射
    /// id 落盘。full-access 经风险确认,取消时回滚显示)。构建时刻的
    /// 语言档记入 `selects_lang`,换档后由 [`AppStore::sync_locale_ui`]
    /// 据此整组重建(标签词典化,Confirm 按标签映射 id)
    pub(crate) fn ensure_pref_selects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.selects_lang = i18n::current_locale();
        let snapshot = self.settings.settings_snapshot.clone();
        let preset_options: Vec<(String, String)> = snapshot["presetOptions"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| {
                        Some((
                            p["id"].as_str()?.to_string(),
                            p["name"].as_str().unwrap_or_default().to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let permission_options: Vec<(String, String)> = snapshot["permissionOptions"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| {
                        let id = p.as_str()?;
                        let label = match id {
                            "read-only" => t!("settings.perm_read_only"),
                            "workspace-write" => t!("settings.perm_workspace_write"),
                            "full-access" => t!("settings.perm_full_access"),
                            other => other.into(),
                        };
                        Some((id.to_string(), label.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // 语言下拉:id = settings.yaml `language` 值 = locale,显示名取
        // `LOCALES` 的原文名(两档同显中文/English)——语言选项不进文案
        // 文件,显示名恒定的政策由常量表构造保证
        let language_options: Vec<(String, String)> = i18n::LOCALES
            .iter()
            .map(|(id, name)| (id.to_string(), name.to_string()))
            .collect();
        let busy_options = vec![
            ("queue".to_string(), t!("settings.busy_queue").to_string()),
            ("steer".to_string(), t!("settings.busy_steer").to_string()),
        ];
        self.settings.preset_select = Some(Self::build_pref_select(
            preset_options,
            snapshot["defaultPreset"].as_str().unwrap_or("standard"),
            PrefMenuKind::Preset,
            window,
            cx,
        ));
        self.settings.permission_select = Some(Self::build_pref_select(
            permission_options,
            snapshot["defaultPermission"]
                .as_str()
                .unwrap_or("workspace-write"),
            PrefMenuKind::Permission,
            window,
            cx,
        ));
        self.settings.language_select = Some(Self::build_pref_select(
            language_options,
            // 手改/历史配置的档位先归一(`zh` → `zh-CN`),免得下拉选不中而空显
            i18n::normalize(snapshot["language"].as_str().unwrap_or(i18n::DEFAULT)),
            PrefMenuKind::Language,
            window,
            cx,
        ));
        self.settings.busy_enter_select = Some(Self::build_pref_select(
            busy_options,
            snapshot["busyEnter"].as_str().unwrap_or("queue"),
            PrefMenuKind::BusyEnter,
            window,
            cx,
        ));
        // 主题两行:选项 = registry 内置主题名(id 与显示名同值,
        // 非词典文案,语言换档不参与重建);当前值取 theme 侧选中名
        // (启动时已从持久化装配,空串归一为 Liuma 默认——下拉必须
        // 能选中一项)。先收集选项(registry 借用与后面的 cx 可变
        // 借用不重叠)
        let theme_options: [(gpui_kit::component::ThemeMode, Vec<(String, String)>); 2] = [
            gpui_kit::component::ThemeMode::Light,
            gpui_kit::component::ThemeMode::Dark,
        ]
        .map(|m| {
            let options: Vec<(String, String)> = gpui_kit::component::ThemeRegistry::global(&*cx)
                .sorted_themes()
                .iter()
                .filter(|t| t.mode == m)
                .map(|t| (t.name.to_string(), t.name.to_string()))
                .collect();
            (m, options)
        });
        for (m, options) in theme_options {
            let current = theme::theme_name(m);
            let built = Self::build_pref_select(
                options,
                &current,
                if m.is_dark() {
                    PrefMenuKind::ThemeDark
                } else {
                    PrefMenuKind::ThemeLight
                },
                window,
                cx,
            );
            if m.is_dark() {
                self.settings.theme_dark_select = Some(built);
            } else {
                self.settings.theme_light_select = Some(built);
            }
        }
    }

    /// 单个偏好 Select 构建(labels + 当前项 + Confirm 落盘订阅)
    pub(crate) fn build_pref_select(
        options: Vec<(String, String)>,
        current: &str,
        kind: PrefMenuKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SelectState<Vec<gpui_kit::SharedString>>> {
        let labels: Vec<gpui_kit::SharedString> = options
            .iter()
            .map(|(_, l)| gpui_kit::SharedString::from(l.clone()))
            .collect();
        let index = options
            .iter()
            .position(|(id, _)| id == current)
            .map(|ix| IndexPath::default().row(ix));
        let state = cx.new(|cx| SelectState::new(labels, index, window, cx));
        cx.subscribe(
            &state,
            move |this, _s, event: &SelectEvent<Vec<gpui_kit::SharedString>>, cx| {
                if let SelectEvent::Confirm(Some(label)) = event {
                    let label_s = label.to_string();
                    if let Some((id, _)) = options.iter().find(|(_, l)| *l == label_s) {
                        let id = id.clone();
                        match kind {
                            PrefMenuKind::Preset => this.set_default_preset(&id, cx),
                            PrefMenuKind::Permission => this.set_default_permission(&id, cx),
                            PrefMenuKind::Language => this.set_language(&id, cx),
                            PrefMenuKind::BusyEnter => this.set_busy_enter(&id, cx),
                            PrefMenuKind::ThemeLight => {
                                this.set_theme_pref(false, &id, cx);
                            }
                            PrefMenuKind::ThemeDark => {
                                this.set_theme_pref(true, &id, cx);
                            }
                        }
                    }
                }
            },
        )
        .detach();
        state
    }

    /// 设置页开关(独立页路由;打开时刷新快照与 onboarding 态)
    pub fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.settings_open = !self.settings.settings_open;
        if self.settings.settings_open {
            self.settings.settings_snapshot = self.bridge.host().settings_view();
            self.recalc_onboarding();
            // 决策区是常驻表单(无「打开编辑卡」事件),开页落在该区即回填
            if self.settings.settings_nav == SettingsNav::Decision {
                self.sync_decision_form(window, cx);
            }
            // 归档区同理:落在此区开页即建控件 + 首拉清单
            if self.settings.settings_nav == SettingsNav::ArchivedChats {
                self.open_archived_section(window, cx);
            }
        }
        cx.notify();
    }

    /// 切换默认 preset(通用区 Agent 预设行;落盘)
    pub fn set_default_preset(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.bridge.host().set_default_preset(id) {
            Ok(()) => {
                self.settings_refresh(cx);
            }
            Err(e) => {
                self.set_settings_notice(false, t!("settings.save_failed", msg = &e.message), cx);
            }
        }
    }

    /// 切换默认权限预设(通用区权限行;full-access 先经风险确认)。
    /// 落盘为工作区默认权限预设,新会话 pin 时沿用。
    pub fn set_default_permission(&mut self, id: &str, cx: &mut Context<Self>) {
        if id == "full-access" {
            self.settings.full_access_confirm = Some(FullAccessAsk::Default);
            let store = cx.entity().clone();
            self.with_window_deferred(cx, move |window, cx| {
                open_full_access_dialog(&store, window, cx);
            });
            cx.notify();
            return;
        }
        match self.bridge.host().set_default_permission_preset(id) {
            Ok(()) => {
                self.settings_refresh(cx);
            }
            Err(e) => {
                self.set_settings_notice(false, t!("settings.save_failed", msg = &e.message), cx);
            }
        }
    }

    /// 切换界面语言偏好(落盘 + 语言盘即时生效:refresh_windows 让
    /// 词典取值整体换档,挂窗态由渲染期 sync_locale_ui 回写)
    pub fn set_language(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.bridge.host().set_language(id) {
            Ok(()) => {
                i18n::apply(id, cx);
                self.settings_refresh(cx);
            }
            Err(e) => {
                self.set_settings_notice(false, t!("settings.save_failed", msg = &e.message), cx);
            }
        }
    }

    /// 切换主题偏好(dark = 深盘;落盘 + registry 校验生效:当前盘
    /// 立即重装,另一盘下次翻盘时装载)
    pub fn set_theme_pref(&mut self, dark: bool, name: &str, cx: &mut Context<Self>) {
        let m = if dark {
            gpui_kit::component::ThemeMode::Dark
        } else {
            gpui_kit::component::ThemeMode::Light
        };
        let saved = if dark {
            self.bridge.host().set_theme_dark(name)
        } else {
            self.bridge.host().set_theme_light(name)
        };
        match saved {
            Ok(()) => {
                if !theme::set_theme(m, name, cx) {
                    self.set_settings_notice(false, t!("settings.theme_unknown", name = name), cx);
                }
            }
            Err(e) => {
                self.set_settings_notice(false, t!("settings.save_failed", msg = &e.message), cx);
            }
        }
    }

    /// 语言档切换后的挂窗态回写:偏好下拉标签按新语言重建。Confirm 按
    /// 标签映射 id 且订阅闭包捕获构建期选项表,故须整组重置重建(当前
    /// 项由快照保位;语言选项名两语言恒原名,重建无害)。Provider 表单
    /// 输入的占位同词典化——无编辑器在开(添加/编辑流缺席)时才清空
    /// 惰建槽再重建,防误清未保存草稿(MCP/hooks 表单每次打开即重建,
    /// 无需处理)。渲染期同步块调用(sync_composer_placeholder 同通道;
    /// 档位未变即零开销早退)
    pub(crate) fn sync_locale_ui(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lang = i18n::current_locale();
        if self.settings.selects_lang == lang {
            return;
        }
        self.settings.selects_lang = lang;
        self.settings.preset_select = None;
        self.settings.permission_select = None;
        self.settings.language_select = None;
        self.settings.busy_enter_select = None;
        // 归档区两下拉(下次进入归档区按新档重建)
        self.settings.archived_order_select = None;
        self.settings.archived_project_select = None;
        self.ensure_pref_selects(window, cx);
        if self.settings.editing_provider.is_none() && !self.settings.adding_provider {
            self.settings.set_form_id = None;
            self.settings.set_form_url = None;
            self.settings.set_form_model = None;
            self.settings.set_form_name = None;
            self.settings.set_form_model_input = None;
            self.settings.context_window_input = None;
            self.settings.set_form_billing_url = None;
            self.settings.set_form_path_balance = None;
            self.settings.set_form_path_currency = None;
            self.settings.set_form_path_5h = None;
            self.settings.set_form_path_7d = None;
            self.settings.set_form_path_resets = None;
            self.ensure_provider_form_inputs(window, cx);
        }
    }

    /// 切换外观偏好(light / dark / system;落盘)。返回是否成功——
    /// 主题切换由调用方在持 window 的点击闭包里做(theme::apply)。
    pub fn set_appearance(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        match self.bridge.host().set_appearance(id) {
            Ok(()) => {
                self.settings_refresh(cx);
                true
            }
            Err(e) => {
                self.set_settings_notice(false, t!("settings.save_failed", msg = &e.message), cx);
                false
            }
        }
    }

    /// 切换繁忙时 Enter 键行为偏好(通用区行;落盘)
    pub fn set_busy_enter(&mut self, behavior: &str, cx: &mut Context<Self>) {
        match self.bridge.host().set_busy_enter(behavior) {
            Ok(()) => {
                self.settings_refresh(cx);
            }
            Err(e) => {
                self.set_settings_notice(false, t!("settings.save_failed", msg = &e.message), cx);
            }
        }
    }

    /// 设置快照重拉 + onboarding 重估
    pub fn settings_refresh(&mut self, cx: &mut Context<Self>) {
        self.settings.settings_snapshot = self.bridge.host().settings_view();
        self.recalc_onboarding();
        cx.notify();
    }

    /// 切换设置页导航区(两栏壳:左 nav + 单区内容)
    pub fn set_settings_nav(
        &mut self,
        nav: SettingsNav,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 判「是否真的换了区」:重复点当前项不得冲掉用户未保存的输入
        let changed = self.settings.settings_nav != nav;
        self.settings.settings_nav = nav;
        if changed && nav == SettingsNav::Decision {
            self.sync_decision_form(window, cx);
        }
        // 进入归档区:惰建控件 + 首拉清单(重复进入不重置筛选态)
        if changed && nav == SettingsNav::ArchivedChats {
            self.open_archived_section(window, cx);
        }
        cx.notify();
    }

    /// 设置页内通告(单槽覆盖;ok = 绿色成功 / 否则红色失败)。
    /// **4s 自动清除**——反馈的持久形态在数据本身(计费行/卡片),
    /// 通告只是瞬态提示,不留常驻
    pub(crate) fn set_settings_notice(
        &mut self,
        ok: bool,
        msg: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.settings.settings_notice_seq = self.settings.settings_notice_seq.wrapping_add(1);
        let seq = self.settings.settings_notice_seq;
        self.settings.settings_notice = Some((ok, msg.into()));
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(4000))
                .await;
            this.update(cx, |s, cx| {
                if s.settings.settings_notice_seq == seq {
                    s.settings.settings_notice = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

/// About 区:版本与产品定位
pub(crate) fn about_section(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let info = &st.state.host_info;
    div()
        .v_flex()
        .gap(px(12.))
        .child(section_title(t!("settings.about")))
        .child(info_line(t!("settings.version"), info.version.clone(), cx))
        .child(intro_line(t!("settings.about_intro"), cx))
}

/// 通用区(行序与形态按 settings.general.item):
/// 语言行(左标题 + 右选择 pill)→ 外观组(纵向:标题 + 三 cube,
/// 图标上文字下)→ 运行中 Enter 行为行(左标题+描述 / 右选择)
pub(crate) fn general_section(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let busy = st.settings.settings_snapshot["busyEnter"]
        .as_str()
        .unwrap_or("queue");
    let language = st.settings.settings_snapshot["language"]
        .as_str()
        .unwrap_or("zh");
    let appearance = st.settings.settings_snapshot["appearance"]
        .as_str()
        .unwrap_or("dark");
    let preset = st.settings.settings_snapshot["defaultPreset"]
        .as_str()
        .unwrap_or("standard");
    let permission = st.settings.settings_snapshot["defaultPermission"]
        .as_str()
        .unwrap_or("workspace-write");
    let preset_options = snapshot_options(&st.settings.settings_snapshot["presetOptions"]);
    let permission_options: Vec<(String, String)> =
        st.settings.settings_snapshot["permissionOptions"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| {
                        let id = p.as_str()?;
                        let label = match id {
                            "read-only" => t!("settings.perm_read_only"),
                            "workspace-write" => t!("settings.perm_workspace_write"),
                            "full-access" => t!("settings.perm_full_access"),
                            other => other.into(),
                        };
                        Some((id.to_string(), label.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
    let busy_options = vec![
        ("queue".to_string(), t!("settings.busy_queue").to_string()),
        ("steer".to_string(), t!("settings.busy_steer").to_string()),
    ];
    // 语言下拉:与 store 侧构建同源(id = settings.yaml `language` =
    // locale,显示名 = `LOCALES` 原文名)
    let language_options: Vec<(String, String)> = i18n::LOCALES
        .iter()
        .map(|(id, name)| (id.to_string(), name.to_string()))
        .collect();
    div()
        .v_flex()
        .child(section_title(t!("settings.general")))
        .mt(px(12.))
        // 行序:Agent 预设 / 权限 / 语言 / 外观 / 繁忙时 Enter 键行为
        .child(selector_row(
            "agent-preset",
            t!("settings.preset_title"),
            t!("settings.preset_desc"),
            &preset_options,
            preset,
            st.settings.preset_select.as_ref(),
            cx,
        ))
        .child(selector_row(
            "permission",
            t!("settings.permission_title"),
            t!("settings.permission_desc"),
            &permission_options,
            permission,
            st.settings.permission_select.as_ref(),
            cx,
        ))
        .child(selector_row(
            "language",
            t!("settings.language"),
            "",
            &language_options,
            language,
            st.settings.language_select.as_ref(),
            cx,
        ))
        .child(appearance_group(store, appearance, cx))
        // 主题两行跟外观组(浅盘/深盘各自选一个 registry 主题)
        .child(theme_rows(st, cx))
        .child(selector_row(
            "busy-enter",
            t!("settings.busy_title"),
            t!("settings.busy_desc"),
            &busy_options,
            busy,
            st.settings.busy_enter_select.as_ref(),
            cx,
        ))
}

/// 主题行组(浅盘/深盘主题下拉;外观组之下)
fn theme_rows(st: &AppStore, cx: &App) -> impl IntoElement {
    let light = theme::theme_name(gpui_kit::component::ThemeMode::Light);
    let dark = theme::theme_name(gpui_kit::component::ThemeMode::Dark);
    div()
        .v_flex()
        .child(selector_row(
            "theme-light",
            t!("settings.theme_light"),
            t!("settings.theme_desc"),
            &[],
            &light,
            st.settings.theme_light_select.as_ref(),
            cx,
        ))
        .child(selector_row(
            "theme-dark",
            t!("settings.theme_dark"),
            t!("settings.theme_desc"),
            &[],
            &dark,
            st.settings.theme_dark_select.as_ref(),
            cx,
        ))
}

/// 快照选项数组 → (id, name)
pub(crate) fn snapshot_options(v: &serde_json::Value) -> Vec<(String, String)> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    Some((
                        p["id"].as_str()?.to_string(),
                        p["name"].as_str().unwrap_or_default().to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 选择器行(左 title+desc,右 Select 下拉[gpui-component])
pub(crate) fn selector_row(
    id: &'static str,
    title: impl Into<gpui_kit::SharedString>,
    desc: impl Into<gpui_kit::SharedString>,
    options: &[(String, String)],
    current: &str,
    select: Option<&Entity<SelectState<Vec<gpui_kit::SharedString>>>>,
    cx: &App,
) -> impl IntoElement {
    let title = title.into();
    let desc = desc.into();
    let _ = options;
    let _ = current;
    let row_sel = sid("pref-row", id);
    div()
        .id(row_sel.clone())
        .debug_selector(move || row_sel.to_string())
        .v_flex()
        .py(px(16.))
        .border_b_1()
        .border_color(theme::border(cx))
        .child(
            div()
                .flex()
                .items_center()
                .child(
                    div()
                        .v_flex()
                        .flex_1()
                        .min_w(px(0.))
                        .gap(px(4.))
                        .child(div().text_size(px(14.)).child(title.to_string()))
                        .when(!desc.is_empty(), |el| {
                            el.child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme::caption(cx))
                                    .child(desc.to_string()),
                            )
                        }),
                )
                // Select 自带 size_full:必须装进定尺寸容器,否则撑爆行高并挤塌文字列。
                // line_height 经 deferred 弹层沿元素树继承——组件项 padding 紧凑,
                // 默认行高对 CJK 偏窄(下拉选项字形相触)
                .children(select.map(|s| {
                    div()
                        .w(px(200.))
                        .h(px(36.))
                        .line_height(gpui_kit::relative(1.4))
                        .child(Select::new(s))
                })),
        )
}

/// 外观组(标题 + cube 行;cube = 图标上文字下,
/// r16,选中 = 模块填充 + 描边)
pub(crate) fn appearance_group(
    store: &Entity<AppStore>,
    current: &str,
    cx: &App,
) -> impl IntoElement {
    let cubes: [(
        &str,
        std::borrow::Cow<'static, str>,
        gpui_kit::component::Icon,
    ); 3] = [
        (
            "light",
            t!("settings.appearance_light"),
            fixed(IconName::Sun, 20.),
        ),
        (
            "dark",
            t!("settings.appearance_dark"),
            fixed(IconName::Moon, 20.),
        ),
        (
            "system",
            t!("settings.appearance_system"),
            fixed(LiumaIcon::Monitor, 20.),
        ),
    ];
    let mut row = div().flex().gap(px(8.));
    for (id, label, icon) in cubes {
        let s = store.clone();
        let active = id == current;
        let sel = sid("appearance-cube", id);
        row = row.child(
            div()
                .id(sel.clone())
                .debug_selector(move || sel.to_string())
                .flex()
                .flex_1()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .py(px(20.))
                .rounded(px(16.))
                .border_1()
                .border_color(if active {
                    theme::label_3(cx)
                } else {
                    theme::border(cx)
                })
                .when(active, |el| el.bg(theme::dock(cx)))
                .cursor_pointer()
                .text_size(px(14.))
                .text_color(if active {
                    theme::label(cx)
                } else {
                    theme::label_2(cx)
                })
                .when(!active, |el| el.hover(|s| s.bg(theme::layer(cx))))
                .child(icon)
                .child(label)
                .on_click(move |_, _window, cx| {
                    // 落盘成功即实装生效:同步切主题盘 + 组件 token
                    // (Theme::update 自动全窗刷新)
                    let ok = s.update(cx, |st, cx| st.set_appearance(id, cx));
                    if ok {
                        theme::apply(theme::Appearance::parse(id), cx);
                    }
                }),
        );
    }
    div()
        .id("appearance-group")
        .debug_selector(|| "appearance-group".to_string())
        .v_flex()
        .gap(px(8.))
        .py(px(16.))
        .border_b_1()
        .border_color(theme::border(cx))
        .child(
            div()
                .text_size(px(14.))
                .child(t!("settings.appearance_title")),
        )
        .child(row)
}
