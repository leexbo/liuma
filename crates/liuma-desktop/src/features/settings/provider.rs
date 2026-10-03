//! 模型设置页:提供方编辑/目录/模型拉取/计费方法与列表卡、编辑卡、弹层视图(含用量倒计时测试)。

use super::*;

/// 从端点获取模型的弹层态(候选清单 + 逐项勾选)
pub(crate) struct ModelFetch {
    /// 端点返回的候选模型
    pub candidates: Vec<String>,
    /// 与 candidates 等长的勾选态(默认全勾)
    pub picked: Vec<bool>,
}

impl AppStore {
    /// 设置页 provider 表单三输入懒建(Enter 提交;同 id = 更新;挂窗态由
    /// store::attach_window_state 调用)
    pub(crate) fn ensure_provider_form_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 设置页 provider 表单三输入(Enter 提交;同 id = 更新)
        if self.settings.set_form_id.is_none() {
            self.settings.set_form_id =
                Some(cx.new(|cx| {
                    InputState::new(window, cx).placeholder(t!("settings.id_placeholder"))
                }));
        }
        if self.settings.set_form_url.is_none() {
            self.settings.set_form_url =
                Some(cx.new(|cx| {
                    InputState::new(window, cx).placeholder("https://gateway.example/v1")
                }));
        }
        if self.settings.set_form_model.is_none() {
            self.settings.set_form_model = Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("settings.optional_placeholder"))
            }));
        }
        if self.settings.set_form_name.is_none() {
            self.settings.set_form_name = Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("settings.name_placeholder"))
            }));
        }
        if self.settings.set_form_model_input.is_none() {
            self.settings.set_form_model_input = Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("settings.model_id_placeholder"))
            }));
        }
        if self.settings.context_window_input.is_none() {
            self.settings.context_window_input =
                Some(cx.new(|cx| {
                    InputState::new(window, cx).placeholder(t!("settings.ctx_placeholder"))
                }));
        }
        if self.settings.set_form_billing_url.is_none() {
            self.settings.set_form_billing_url =
                Some(cx.new(|cx| {
                    InputState::new(window, cx).placeholder(t!("settings.url_placeholder"))
                }));
        }
        for (slot, ph) in [
            (
                &mut self.settings.set_form_path_balance,
                t!("settings.balance_path_placeholder"),
            ),
            (
                &mut self.settings.set_form_path_currency,
                t!("settings.currency_path_placeholder"),
            ),
            (
                &mut self.settings.set_form_path_5h,
                t!("settings.usage5h_path_placeholder"),
            ),
            (
                &mut self.settings.set_form_path_7d,
                t!("settings.usage7d_path_placeholder"),
            ),
            (
                &mut self.settings.set_form_path_resets,
                t!("settings.reset_path_placeholder"),
            ),
        ] {
            if slot.is_none() {
                *slot = Some(cx.new(|cx| InputState::new(window, cx).placeholder(ph)));
            }
        }
        self.ensure_pref_selects(window, cx);
        self.ensure_builtin_select(window, cx);
        self.ensure_dialect_select(window, cx);
        for input in [
            &self.settings.set_form_id,
            &self.settings.set_form_url,
            &self.settings.set_form_model,
        ]
        .into_iter()
        .flatten()
        {
            cx.subscribe(input, |this, _i, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    this.apply_provider_editor(cx);
                }
            })
            .detach();
        }
        // 窗口行内输入:Enter = 提交该行(不解锁整卡,避免误保存半填表单)
        if let Some(input) = &self.settings.context_window_input {
            cx.subscribe(input, |this, _i, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    this.commit_context_window_edit(cx);
                }
            })
            .detach();
        }
    }

    /// 内置卡提供方下拉构建(目录五家;Confirm → pick_builtin_provider)
    pub(crate) fn ensure_builtin_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.builtin_select.is_some() {
            self.sync_builtin_select(window, cx);
            return;
        }
        let options: Vec<(String, String)> = liuma_core::settings::provider_catalog()
            .into_iter()
            .map(|e| (e.id.clone(), e.display_name.clone()))
            .collect();
        let labels: Vec<gpui_kit::SharedString> = options
            .iter()
            .map(|(_, l)| gpui_kit::SharedString::from(l.clone()))
            .collect();
        let index = options
            .iter()
            .position(|(id, _)| *id == self.settings.builtin_picked)
            .map(|ix| IndexPath::default().row(ix));
        let state = cx.new(|cx| SelectState::new(labels, index, window, cx));
        cx.subscribe(
            &state,
            move |this, _s, event: &SelectEvent<Vec<gpui_kit::SharedString>>, cx| {
                if let SelectEvent::Confirm(Some(label)) = event {
                    let label_s = label.to_string();
                    if let Some((id, _)) = options.iter().find(|(_, l)| *l == label_s) {
                        this.pick_builtin_provider(id, cx);
                    }
                }
            },
        )
        .detach();
        self.settings.builtin_select = Some(state);
    }

    /// 自定义卡 API 格式下拉构建(通用三面;deepseek-×/glm-responses 是
    /// 厂商预设,由「提供方」下拉承载,不在此混列。upsert 校验域仍 =
    /// DIALECT_NAMES 全集,此处只是 UI 便利项)
    pub(crate) fn ensure_dialect_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // (方言 id, 显示名):下拉展示显示名,Confirm 经显示名反查 id
        // 回写。当前值是目录外方言(如 glm-responses)时原值追加显示——
        // 否则下拉错误定位到首项,确认即静默改写方言
        let dialects = [
            (
                "openai-completions",
                t!("settings.dialect_display_chat").into_owned(),
            ),
            (
                "openai-responses",
                t!("settings.dialect_display_responses").into_owned(),
            ),
            (
                "anthropic-messages",
                t!("settings.dialect_display_anthropic").into_owned(),
            ),
        ];
        let cur = self.settings.set_form_dialect.clone();
        let mut pairs: Vec<(gpui_kit::SharedString, gpui_kit::SharedString)> = dialects
            .iter()
            .map(|(id, l)| {
                (
                    gpui_kit::SharedString::from(*id),
                    gpui_kit::SharedString::from(l.clone()),
                )
            })
            .collect();
        if !pairs.iter().any(|(id, _)| id.as_ref() == cur) {
            let same: gpui_kit::SharedString = cur.clone().into();
            pairs.push((same.clone(), same));
        }
        let labels: Vec<gpui_kit::SharedString> = pairs.iter().map(|(_, l)| l.clone()).collect();
        if let Some(select) = &self.settings.dialect_select {
            let ix = pairs
                .iter()
                .position(|(id, _)| id.as_ref() == cur)
                .unwrap_or(0);
            select.update(cx, |s, cx| {
                s.set_selected_index(Some(IndexPath::default().row(ix)), window, cx);
            });
            return;
        }
        let index = pairs
            .iter()
            .position(|(id, _)| id.as_ref() == cur)
            .map(|ix| IndexPath::default().row(ix));
        let state = cx.new(|cx| SelectState::new(labels, index, window, cx));
        cx.subscribe(
            &state,
            move |this, _s, event: &SelectEvent<Vec<gpui_kit::SharedString>>, cx| {
                if let SelectEvent::Confirm(Some(label)) = event {
                    // 显示名反查方言 id;追加的目录外现值 label 即 id
                    this.settings.set_form_dialect = pairs
                        .iter()
                        .find(|(_, l)| l.as_ref() == label.as_str())
                        .map(|(id, _)| id.to_string())
                        .unwrap_or_else(|| label.to_string());
                    cx.notify();
                }
            },
        )
        .detach();
        self.settings.dialect_select = Some(state);
    }

    /// 自定义卡 API 格式下拉同步(按表单方言定位)
    pub(crate) fn sync_dialect_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_dialect_select(window, cx);
    }

    /// 该 provider 是否处于首运行 setup 姿态:尚无可服务 provider 且
    /// 默认 provider 未配置凭据(setup 卡即其在页面上
    /// 的存在形式,直到用户关闭)
    pub(crate) fn provider_needs_setup(&self, id: &str) -> bool {
        let rows = self.settings.settings_snapshot["providers"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let any_ready = rows
            .iter()
            .any(|p| p["credentialReady"].as_bool().unwrap_or(false));
        let me_ready = rows
            .iter()
            .find(|p| p["id"].as_str() == Some(id))
            .and_then(|p| p["credentialReady"].as_bool())
            .unwrap_or(true);
        !any_ready
            && !me_ready
            && self.settings.settings_snapshot["defaultProvider"].as_str() == Some(id)
    }

    /// 渲染期 setup 姿态判定(需 setup 且未被手动关闭)
    pub fn provider_setup_posture(&self, id: &str) -> bool {
        self.provider_needs_setup(id) && !self.settings.dismissed_setup.contains(id)
    }

    /// 打开行内编辑卡(编辑卡在行卡内展开;预填自定义字段,key 清空)
    pub fn open_provider_editor(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.fill_provider_form(Some(id), window, cx);
        self.ensure_key_input(t!("settings.key_keep_placeholder"), window, cx);
        // 目录内厂商 = 内置卡(适配器与目录绑定);其余 = 自定义卡
        let builtin = liuma_core::settings::provider_catalog()
            .iter()
            .any(|e| e.id == id);
        if builtin {
            self.settings.builtin_picked = id.to_string();
            self.sync_builtin_select(window, cx);
            // 旧条目模型清单为空(预设前保存)→ 目录官方清单打底,
            // 折叠区可「从端点获取」继续更新
            if self.settings.set_form_models.is_empty()
                && let Some(entry) = liuma_core::settings::provider_catalog()
                    .into_iter()
                    .find(|e| e.id == id)
            {
                self.settings.set_form_models = entry.models.clone();
            }
        }
        self.settings.builtin_mode = builtin;
        self.settings.editing_provider = Some(id.to_string());
        self.settings.adding_provider = false;
        self.settings.saved_provider_notice = None;
        cx.notify();
    }

    /// 内置卡提供方下拉同步(按 builtin_picked 定位)
    pub(crate) fn sync_builtin_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(select) = &self.settings.builtin_select else {
            return;
        };
        let ix = liuma_core::settings::provider_catalog()
            .iter()
            .position(|e| e.id == self.settings.builtin_picked);
        if let Some(ix) = ix {
            select.update(cx, |s, cx| {
                s.set_selected_index(Some(IndexPath::default().row(ix)), window, cx);
            });
        }
    }

    /// 内置卡提供方切换:仅切目录键;卡片字段在渲染期从目录条目派生
    /// (无 window 依赖),模型草稿清单按新条目重预填,apply 时整体落盘
    pub fn pick_builtin_provider(&mut self, id: &str, cx: &mut Context<Self>) {
        self.settings.builtin_picked = id.to_string();
        if let Some(entry) = liuma_core::settings::provider_catalog()
            .into_iter()
            .find(|e| e.id == id)
        {
            self.settings.set_form_models = entry.models.clone();
        }
        // 模型清单整体更换:窗口覆盖随旧清单作废(回落默认)
        self.settings.set_form_context_windows.clear();
        self.close_context_window_edit();
        cx.notify();
    }

    /// 添加卡内切换模式 Tab(第三方目录 / 自定义 API):仅添加态可切,
    /// 各模式重建其下拉与键占位
    pub fn switch_provider_add_mode(
        &mut self,
        builtin: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings.builtin_mode == builtin {
            return;
        }
        self.settings.builtin_mode = builtin;
        self.settings.builtin_advanced_open = false;
        if builtin {
            self.settings.builtin_picked = "deepseek".into();
            if let Some(entry) = liuma_core::settings::provider_catalog()
                .into_iter()
                .find(|e| e.id == "deepseek")
            {
                self.settings.set_form_models = entry.models.clone();
            }
            self.sync_builtin_select(window, cx);
        } else {
            // 新建自定义 provider:模型目录从空开始,不携带目录预填
            self.settings.set_form_models.clear();
            self.sync_dialect_select(window, cx);
        }
        cx.notify();
    }

    /// 打开添加卡(默认第三方目录 Tab;卡顶 Tab 可切自定义)
    pub fn open_provider_add_builtin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fill_provider_form(None, window, cx);
        self.ensure_key_input(t!("settings.key_env_placeholder"), window, cx);
        self.settings.editing_provider = None;
        self.settings.adding_provider = true;
        self.settings.builtin_mode = true;
        self.settings.builtin_advanced_open = false;
        self.settings.builtin_picked = "deepseek".into();
        if let Some(entry) = liuma_core::settings::provider_catalog()
            .into_iter()
            .find(|e| e.id == "deepseek")
        {
            self.settings.set_form_models = entry.models.clone();
        }
        self.sync_builtin_select(window, cx);
        self.settings.saved_provider_notice = None;
        cx.notify();
    }

    /// API key 输入惰建(编辑卡主字段;write-only;占位随卡片模式,
    /// 已建且占位一致则保留原输入)
    pub(crate) fn ensure_key_input(
        &mut self,
        placeholder: impl Into<gpui_kit::SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder: gpui_kit::SharedString = placeholder.into();
        let matches = self.settings.key_input_placeholder == placeholder.as_ref();
        if !matches {
            self.settings.key_input =
                Some(cx.new(|cx| InputState::new(window, cx).placeholder(placeholder.clone())));
            self.settings.key_input_placeholder = placeholder.to_string();
        }
        if let Some(input) = &self.settings.key_input {
            input.update(cx, |s, cx| s.set_value("", window, cx));
        }
    }

    /// 关闭编辑/添加卡(setup 姿态的关闭 = dismiss,本会话回退普通行;
    /// setup 卡渲染不经 editing_provider,故须显式携带目标 id)
    pub fn close_provider_editor(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.settings.editing_provider.as_deref() == Some(id) {
            self.settings.editing_provider = None;
        }
        if !id.is_empty() && self.provider_needs_setup(id) {
            self.settings.dismissed_setup.insert(id.to_string());
        }
        self.settings.adding_provider = false;
        cx.notify();
    }

    /// 应用编辑/添加卡:key 非空随条目直存 settings(provider `api_key`);
    /// 留空 = 保留已存值。成功 → 保存通告 + 静默探测 + 刷新 + 关卡
    pub fn apply_provider_editor(&mut self, cx: &mut Context<Self>) {
        // 展开中的窗口编辑先提交;非法则拒绝保存(不静默丢弃输入)
        if self.settings.context_window_edit.is_some() && !self.commit_context_window_edit(cx) {
            self.push_local_notice(t!("settings.ctx_invalid_notice"), cx);
            return;
        }
        let id = if let Some(id) = &self.settings.editing_provider {
            id.clone()
        } else if self.settings.adding_provider {
            if self.settings.builtin_mode {
                // 内置添加卡不渲染 id 输入:路由键 = 所选目录条目
                self.settings.builtin_picked.clone()
            } else {
                self.settings
                    .set_form_id
                    .as_ref()
                    .map(|i| i.read(cx).value().trim().to_string())
                    .unwrap_or_default()
            }
        } else {
            return;
        };
        if id.is_empty() {
            self.push_local_notice(t!("settings.provider_id_empty"), cx);
            return;
        }
        // 新增时查重:ID 是路由键,遮蔽既有条目
        // 只会静默覆盖其配置。内置卡例外:对已存在厂商保存 = 编辑语义
        if self.settings.editing_provider.is_none() && !self.settings.builtin_mode {
            let taken = self.settings.settings_snapshot["providers"]
                .as_array()
                .is_some_and(|ps| ps.iter().any(|p| p["id"].as_str() == Some(id.as_str())));
            if taken {
                self.push_local_notice(t!("settings.provider_id_dup"), cx);
                return;
            }
        }
        let key = self
            .settings
            .key_input
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
            .unwrap_or_default();
        let existing = self.settings.settings_snapshot["providers"]
            .as_array()
            .and_then(|ps| {
                ps.iter()
                    .find(|p| p["id"].as_str() == Some(id.as_str()))
                    .cloned()
            });
        let existing_ref = existing
            .as_ref()
            .and_then(|p| p["credential_ref"].as_str().map(String::from));
        // 输入留空 = 不改已存 key(None = 保留,write-only 语义;
        // 视图不回明文,不存在「从快照回填」)
        let api_key = (!key.is_empty()).then_some(key);
        let model = self
            .settings
            .set_form_model
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
            .unwrap_or_default();
        // 内置模式:目录条目为基底(适配器/URL/计费预设按厂商绑定),
        // 模型列表取表单(端点拉取/手动增删会更新它)
        let catalog_entry = if self.settings.builtin_mode {
            liuma_core::settings::provider_catalog()
                .into_iter()
                .find(|e| e.id == self.settings.builtin_picked)
        } else {
            None
        };
        let mut base_url = self
            .settings
            .set_form_url
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
            .unwrap_or_default();
        let mut dialect = self.settings.set_form_dialect.clone();
        let mut display = self
            .settings
            .set_form_name
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
            .filter(|v| !v.is_empty());
        let mut billing = self.form_billing_config(cx);
        if let Some(entry) = &catalog_entry {
            // API 地址:输入非空**且用户改动过**(≠ 存储值)= 覆写;
            // 未改动(= 旧预设残留)或空 = 跟随目录默认——目录预设
            // 变更(如切 anthropic 面)时旧 URL 自动迁移,不打死循环
            let stored_url = existing
                .as_ref()
                .and_then(|p| p["base_url"].as_str())
                .unwrap_or_default();
            let edited = !base_url.is_empty() && base_url != stored_url;
            if base_url.is_empty() || !edited {
                base_url = entry.base_url.clone();
            }
            dialect = entry.dialect.clone();
            display = Some(entry.display_name.clone());
            billing = entry.billing.clone();
        }
        // 已存条目的 billing_cache 不被内置保存清掉;内置默认模型 = 表单
        // (编辑态 fill 已预填快照值)→ 目录首选;自定义保持清空即删语义
        let saved = existing.as_ref();
        // 生效方言(内置 = 目录覆写后的值;hosted 写回门控用它,与
        // 编辑卡开关的显示门控同源)
        let effective_dialect = dialect.clone();
        let default_model =
            if model.is_empty() {
                match &catalog_entry {
                    Some(entry) => entry.models.first().cloned().or_else(|| {
                        saved.and_then(|p| p["default_model"].as_str().map(String::from))
                    }),
                    None => None,
                }
            } else {
                Some(model)
            };
        let entry = liuma_core::settings::ProviderEntry {
            id: id.clone(),
            base_url,
            dialect,
            credential_ref: existing_ref,
            api_key,
            default_model,
            display_name: display,
            models: self.settings.set_form_models.clone(),
            billing,
            billing_cache: self.settings.settings_snapshot["providers"]
                .as_array()
                .and_then(|ps| {
                    ps.iter()
                        .find(|p| p["id"].as_str() == Some(id.as_str()))
                        .and_then(|p| serde_json::from_value(p["billing_cache"].clone()).ok())
                }),
            // 每模型上下文窗口:表单已提交草稿(空输入 = 不覆盖 → 缺省 1M)
            model_context_windows: self.form_context_windows(),
            // 厂商托管工具:按方言门控落盘(该面无声明时保持原值——
            // 模型能力面,不是用户在该格式下可改的项)
            hosted_tools: if matches!(
                effective_dialect.as_str(),
                "anthropic-messages" | "openai-responses"
            ) {
                self.settings
                    .set_form_hosted_web_search
                    .then(|| "web_search".to_string())
                    .map(|t| vec![t])
                    .unwrap_or_default()
            } else {
                self.settings.settings_snapshot["providers"]
                    .as_array()
                    .and_then(|ps| {
                        ps.iter()
                            .find(|p| p["id"].as_str() == Some(id.as_str()))
                            .and_then(|p| serde_json::from_value(p["hosted_tools"].clone()).ok())
                    })
                    .unwrap_or_default()
            },
        };
        match self.bridge.host().upsert_provider(entry) {
            Ok(()) => {
                self.settings.editing_provider = None;
                self.settings.adding_provider = false;
                self.settings.saved_provider_notice = Some(id.clone());
                let saved = id.clone();
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(4000))
                        .await;
                    this.update(cx, |s, cx| {
                        if s.settings.saved_provider_notice.as_deref() == Some(saved.as_str()) {
                            s.settings.saved_provider_notice = None;
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
                self.settings_refresh(cx);
                self.probe_models_quietly(&id, cx);
            }
            Err(e) => {
                self.set_settings_notice(false, t!("settings.save_failed", msg = &e.message), cx);
            }
        }
    }

    /// 表单计费区 → BillingConfig(未启用 = None;URL 空 = None)
    pub(crate) fn form_billing_config(
        &self,
        cx: &Context<Self>,
    ) -> Option<liuma_core::settings::BillingConfig> {
        if !self.settings.set_form_billing_enabled {
            return None;
        }
        let val = |slot: &Option<Entity<InputState>>| -> String {
            slot.as_ref()
                .map(|i| i.read(cx).value().trim().to_string())
                .unwrap_or_default()
        };
        let url = val(&self.settings.set_form_billing_url);
        if url.is_empty() {
            return None;
        }
        let opt_path = |slot: &Option<Entity<InputState>>| -> Option<String> {
            let v = val(slot);
            (!v.is_empty()).then_some(v)
        };
        Some(liuma_core::settings::BillingConfig {
            kind: if self.settings.set_form_billing_kind == "usage" {
                liuma_core::settings::BillingKind::Usage
            } else {
                liuma_core::settings::BillingKind::Balance
            },
            url,
            paths: liuma_core::settings::BillingPaths {
                balance: opt_path(&self.settings.set_form_path_balance),
                currency: opt_path(&self.settings.set_form_path_currency),
                usage_5h: opt_path(&self.settings.set_form_path_5h),
                usage_7d: opt_path(&self.settings.set_form_path_7d),
                resets: opt_path(&self.settings.set_form_path_resets),
                resets_7d: None,
            },
            auth_style: self.settings.set_form_billing_auth_style.clone(),
        })
    }

    /// 计费端点启用开关
    pub fn toggle_billing_enabled(&mut self, cx: &mut Context<Self>) {
        self.settings.set_form_billing_enabled = !self.settings.set_form_billing_enabled;
        cx.notify();
    }

    /// 计费形态切换(balance / usage)
    pub fn set_billing_kind(&mut self, kind: &str, cx: &mut Context<Self>) {
        self.settings.set_form_billing_kind = kind.to_string();
        cx.notify();
    }

    /// 手动添加模型(输入非空且不重复才入草稿;成功清输入)
    pub fn add_model_manual(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = &self.settings.set_form_model_input else {
            return;
        };
        let v = input.read(cx).value().trim().to_string();
        if v.is_empty() || self.settings.set_form_models.contains(&v) {
            return;
        }
        self.settings.set_form_models.push(v);
        if let Some(input) = &self.settings.set_form_model_input {
            input.update(cx, |s, cx| s.set_value("", window, cx));
        }
        cx.notify();
    }

    /// 移除草稿清单中的模型
    pub fn remove_form_model(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.settings.set_form_models.len() {
            let removed = self.settings.set_form_models.remove(ix);
            self.settings.set_form_context_windows.remove(&removed);
            if self.settings.context_window_edit.as_deref() == Some(removed.as_str()) {
                self.close_context_window_edit();
            }
            cx.notify();
        }
    }

    /// 展开某模型的上下文窗口行内编辑(预填当前覆盖;无覆盖 = 空输入)。
    /// 已展开同一行 = no-op(不覆盖正在输入的内容);已展开其他行 = 先
    /// 提交当前输入,当前输入非法则拒绝切换(不静默丢弃)。
    pub fn begin_context_window_edit(
        &mut self,
        model: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(open) = self.settings.context_window_edit.clone() {
            if open == model {
                return; // 同一行:不覆盖正在输入的内容
            }
            // 已展开其他行:先提交当前输入;非法则拒绝切换(不静默丢弃)
            if !self.commit_context_window_edit(cx) {
                return;
            }
        }
        let Some(input) = &self.settings.context_window_input else {
            return;
        };
        let text = self
            .settings
            .set_form_context_windows
            .get(model)
            .map(|v| v.to_string())
            .unwrap_or_default();
        input.update(cx, |s, cx| s.set_value(&text, window, cx));
        self.settings.context_window_edit = Some(model.to_string());
        self.settings.context_window_error = false;
        cx.notify();
    }

    /// 提交窗口编辑:`true` = 已提交并收起;`false` = 非法(行内提示,
    /// 保持展开)。空输入 = 清除覆盖(回落默认)。
    pub fn commit_context_window_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(model) = self.settings.context_window_edit.clone() else {
            return true;
        };
        let raw = self
            .settings
            .context_window_input
            .as_ref()
            .map(|i| i.read(cx).value())
            .unwrap_or_default();
        match parse_context_window_tokens(&raw) {
            Ok(Some(v)) => {
                self.settings.set_form_context_windows.insert(model, v);
            }
            Ok(None) => {
                self.settings.set_form_context_windows.remove(&model);
            }
            Err(()) => {
                self.settings.context_window_error = true;
                cx.notify();
                return false;
            }
        }
        self.close_context_window_edit();
        cx.notify();
        true
    }

    /// 放弃窗口编辑
    pub fn cancel_context_window_edit(&mut self, cx: &mut Context<Self>) {
        self.close_context_window_edit();
        cx.notify();
    }

    /// 收起窗口编辑行(不触碰草稿值)
    pub(crate) fn close_context_window_edit(&mut self) {
        self.settings.context_window_edit = None;
        self.settings.context_window_error = false;
    }

    /// 表单每模型上下文窗口草稿(传入保存路径;模型清单驱动,已提交值)
    pub(crate) fn form_context_windows(&self) -> std::collections::BTreeMap<String, u64> {
        self.settings.set_form_context_windows.clone()
    }

    /// 从端点拉取可用模型:base_url/方言取表单;key = 表单值(空 = 交给
    /// host 按既有凭据链解析,仅已保存 provider 生效)。结果进弹层多选
    pub fn open_fetch_models(&mut self, provider_id: &str, cx: &mut Context<Self>) {
        // 内置模式:URL/方言按目录条目(卡片不渲染这两个输入)
        let catalog = if self.settings.builtin_mode {
            liuma_core::settings::provider_catalog()
                .into_iter()
                .find(|e| e.id == self.settings.builtin_picked)
        } else {
            None
        };
        let form_url = self
            .settings
            .set_form_url
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
            .unwrap_or_default();
        let base_url = match &catalog {
            Some(e) => e.base_url.clone(),
            None => form_url,
        };
        if base_url.is_empty() {
            self.set_settings_notice(false, t!("settings.billing_need_url"), cx);
            return;
        }
        let dialect = match &catalog {
            Some(e) => e.dialect.clone(),
            None => self.settings.set_form_dialect.clone(),
        };
        let key = self
            .settings
            .key_input
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
            .filter(|v| !v.is_empty());
        let pid = provider_id.to_string();
        self.settings.model_fetch_loading = true;
        self.settings.model_fetch = Some(ModelFetch {
            candidates: Vec::new(),
            picked: Vec::new(),
        });
        let dialog_store = cx.entity().clone();
        self.with_window_deferred(cx, move |window, cx| {
            crate::features::settings::open_fetch_models_dialog(&dialog_store, window, cx);
        });
        cx.notify();
        let store = cx.entity().clone();
        let host = self.bridge.host().clone();
        let rx = self.bridge.call(async move {
            host.discover_models(base_url, dialect, key, Some(pid.clone()))
                .await
        });
        cx.spawn(async move |_this, cx| {
            let candidates = rx.await.unwrap_or_default();
            store.update(cx, |s, _cx| {
                if let Some(mf) = &mut s.settings.model_fetch {
                    mf.candidates = candidates.clone();
                    mf.picked = vec![true; candidates.len()];
                }
                s.settings.model_fetch_loading = false;
            });
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// 勾选/取消候选模型
    pub fn toggle_fetch_pick(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(mf) = &mut self.settings.model_fetch
            && ix < mf.picked.len()
        {
            mf.picked[ix] = !mf.picked[ix];
            cx.notify();
        }
    }

    /// 采纳勾选候选入草稿清单(去重),收弹层
    pub fn adopt_fetched_models(&mut self, cx: &mut Context<Self>) {
        if let Some(mf) = self.settings.model_fetch.take() {
            for (c, pick) in mf.candidates.iter().zip(&mf.picked) {
                if *pick && !self.settings.set_form_models.contains(c) {
                    self.settings.set_form_models.push(c.clone());
                }
            }
        }
        cx.notify();
    }

    /// 关闭获取弹层
    pub fn close_fetch_modal(&mut self, cx: &mut Context<Self>) {
        self.settings.model_fetch = None;
        self.settings.model_fetch_loading = false;
        cx.notify();
    }

    /// 额度自动刷新触发(turn/end;60s 防抖——用量刚消耗完就近补一查,
    /// 猝发 turn 不追打)
    pub fn auto_refresh_billing(&mut self, cx: &mut Context<Self>) {
        self.refresh_billing_auto_inner(false, cx);
    }

    /// 额度静默自动刷新(自定时机:启动一次 + 5min 节拍 force + turn/end
    /// 防抖)。只刷**当前工作区生效 provider**(绑定 > 宿主默认;徽标
    /// 唯一数据源,显示端同源取数);无计费端点整轮跳过。静默纪律:不落
    /// 设置页通告、不点亮手动刷新钮 spinner;手动(billing_refreshing)
    /// 进行中跳过本轮。失败不提示,记尝试时刻防抖
    pub(crate) fn refresh_billing_auto_inner(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.settings.billing_refreshing.is_some() || self.settings.billing_auto_running {
            return;
        }
        if !force
            && self
                .settings
                .billing_auto_last
                .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(60))
        {
            return;
        }
        let snap = &self.settings.settings_snapshot;
        let ws = self.effective_workspace();
        let ws_pid = snap["workspaceProviders"][&ws].as_str().map(str::to_string);
        let Some(pid) = ws_pid.or_else(|| snap["defaultProvider"].as_str().map(str::to_string))
        else {
            return;
        };
        // 计费预设默认生效:条目显式配置,或目录内厂商的内置端点回落
        let catalog_preset = liuma_core::settings::provider_catalog()
            .into_iter()
            .any(|e| e.id == pid && e.billing.is_some());
        let configured = snap["providers"].as_array().is_some_and(|ps| {
            ps.iter()
                .any(|p| p["id"].as_str() == Some(pid.as_str()) && p["billing"].is_object())
        }) || catalog_preset;
        if !configured {
            return;
        }
        self.settings.billing_auto_running = true;
        self.settings.billing_auto_last = Some(std::time::Instant::now());
        cx.notify();
        let store = cx.entity().clone();
        let host = self.bridge.host().clone();
        let rx = self
            .bridge
            .call(async move { host.fetch_billing(&pid).await });
        cx.spawn(async move |_this, cx| {
            let result = rx.await.unwrap_or_else(|e| Err(format!("{e}")));
            store.update(cx, |s, cx| {
                s.settings.billing_auto_running = false;
                if result.is_ok() {
                    s.settings_refresh(cx);
                }
            });
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// 额度自动刷新节拍(挂窗一次):启动即查一次,此后每 5min 一轮
    pub fn start_billing_tick(&mut self, cx: &mut Context<Self>) {
        if self.billing_tick.is_some() {
            return;
        }
        self.refresh_billing_auto_inner(true, cx);
        self.billing_tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(300))
                    .await;
                let _ = this.update(cx, |s, cx| s.refresh_billing_auto_inner(true, cx));
            }
        }));
    }

    /// 立即刷新计费。use_form = 编辑器内按钮(**表单当前值**试查,未
    /// 应用也能刷;成功写回该 provider 的 billing_cache);false = 卡片
    /// 刷新钮走已保存配置。结果均刷新快照
    pub fn refresh_billing_now(
        &mut self,
        provider_id: &str,
        use_form: bool,
        cx: &mut Context<Self>,
    ) {
        if self.settings.billing_refreshing.is_some() {
            return;
        }
        let pid = provider_id.to_string();
        let cfg = if use_form {
            self.form_billing_config(cx)
        } else {
            None
        };
        let form_key = self
            .settings
            .key_input
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
            .filter(|v| !v.is_empty());
        let dialect = self.settings.set_form_dialect.clone();
        self.settings.billing_refreshing = Some(pid.clone());
        cx.notify();
        let store = cx.entity().clone();
        let host = self.bridge.host().clone();
        let rx = self.bridge.call(async move {
            match cfg {
                Some(cfg) => {
                    let snap = host
                        .test_billing(cfg, dialect, form_key, Some(pid.clone()))
                        .await?;
                    host.set_billing_cache(&pid, snap).map_err(|e| e.message)?;
                    Ok(())
                }
                None => host.fetch_billing(&pid).await,
            }
        });
        cx.spawn(async move |_this, cx| {
            let result = rx.await.unwrap_or_else(|e| Err(format!("{e}")));
            store.update(cx, |s, cx| {
                s.settings.billing_refreshing = None;
                match result {
                    // 成功不弹条目:卡片时间戳行已随刷新更新(release 4s
                    // 通告与「刚刚」语义重复);失败仍需通告(无别处可见)
                    Ok(()) => {
                        s.settings_refresh(cx);
                    }
                    Err(msg) => s.set_settings_notice(
                        false,
                        t!("settings.billing_query_failed", msg = msg),
                        cx,
                    ),
                }
            });
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// 静默后台探测 provider 模型清单(upsert 清缓存后无自动重探路径;
    /// 应用编辑卡后触发,结果随下次设置刷新可见,失败不提示)
    pub(crate) fn probe_models_quietly(&mut self, provider_id: &str, cx: &mut Context<Self>) {
        let store = cx.entity().clone();
        let host = self.bridge.host().clone();
        let pid = provider_id.to_string();
        let rx = self
            .bridge
            .call(async move { host.refresh_models(&pid).await });
        cx.spawn(async move |_this, cx| {
            let _ = rx.await;
            store.update(cx, |s, cx| s.settings_refresh(cx));
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// 挂窗模型探测兜底:有凭据但模型清单缺席(条目未配 + 探测缓存空;
    /// 缓存不持久,重启即失)的 provider 静默拉 /models——模型菜单按
    /// 清单分组,缺席即整组消失。每个缺席者一次,失败静默(保存时
    /// probe / 菜单「从端点获取」可再探)
    pub fn ensure_models_probed(&mut self, cx: &mut Context<Self>) {
        let missing: Vec<String> = self.settings.settings_snapshot["providers"]
            .as_array()
            .map(|ps| {
                ps.iter()
                    .filter(|p| {
                        p["credentialReady"].as_bool() == Some(true)
                            && p["modelsCached"].as_bool() != Some(true)
                    })
                    .filter_map(|p| p["id"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if missing.is_empty() {
            return;
        }
        let host = self.bridge.host().clone();
        let store = cx.entity().clone();
        let rxs: Vec<_> = missing
            .into_iter()
            .map(|pid| {
                let h = host.clone();
                self.bridge
                    .call(async move { h.refresh_models(&pid).await })
            })
            .collect();
        cx.spawn(async move |_this, cx| {
            for rx in rxs {
                let _ = rx.await;
            }
            store.update(cx, |s, cx| s.settings_refresh(cx));
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// 打开 provider 删除确认(组件库 Dialog 层;取消即纯关闭,无旗标)
    pub fn ask_delete_provider(&mut self, id: &str, cx: &mut Context<Self>) {
        self.settings.saved_provider_notice = None;
        let pid = id.to_string();
        let store = cx.entity().clone();
        self.with_window_deferred(cx, move |window, cx| {
            crate::features::settings::open_delete_provider_dialog(&store, &pid, window, cx);
        });
        cx.notify();
    }

    /// 确认删除 provider(凭据记录是用户资产,不随删;id 由弹层确认
    /// 钮直派,不再经旗标中转)
    pub fn confirm_delete_provider(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.settings.editing_provider.as_deref() == Some(id) {
            self.settings.editing_provider = None;
        }
        match self.bridge.host().remove_provider(id) {
            Ok(()) => {
                self.settings.saved_provider_notice = None;
                self.settings_refresh(cx);
            }
            Err(e) => self.push_local_notice(t!("settings.delete_failed", msg = &e.message), cx),
        }
    }

    /// 表单预填(编辑态取快照字段;添加态清空 + 方言回默认)
    pub(crate) fn fill_provider_form(
        &mut self,
        id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entry = id.and_then(|id| {
            self.settings.settings_snapshot["providers"]
                .as_array()
                .and_then(|ps| {
                    ps.iter().find(|p| p["id"].as_str() == Some(id)).map(|p| {
                        (
                            p["base_url"].as_str().unwrap_or_default().to_string(),
                            p["dialect"]
                                .as_str()
                                .unwrap_or("openai-completions")
                                .to_string(),
                            p["default_model"].as_str().unwrap_or_default().to_string(),
                            p["display_name"].as_str().unwrap_or_default().to_string(),
                            p["models"].as_array().cloned().unwrap_or_default(),
                            p["billing"].clone(),
                            p["model_context_windows"].clone(),
                            p["hosted_tools"].clone(),
                        )
                    })
                })
        });
        let (url, dialect, model, name, models, billing, context_windows, hosted) = entry
            .unwrap_or_else(|| {
                (
                    String::new(),
                    "openai-completions".into(),
                    String::new(),
                    String::new(),
                    Vec::new(),
                    serde_json::Value::Null,
                    serde_json::Value::Null,
                    serde_json::Value::Null,
                )
            });
        self.settings.set_form_hosted_web_search = hosted
            .as_array()
            .map(|a| a.iter().any(|t| t.as_str() == Some("web_search")))
            .unwrap_or(false);
        self.settings.set_form_dialect = dialect;
        self.settings.set_form_models = models
            .iter()
            .filter_map(|m| m.as_str().map(String::from))
            .collect();
        // 每模型上下文窗口回填(JSON 对象;非法值忽略 = 回落默认)
        let prefill: std::collections::BTreeMap<String, u64> =
            serde_json::from_value(context_windows).unwrap_or_default();
        // 计费表单回填(快照字段 = ProviderEntry serde 直出)
        let billing_kind = billing["kind"].as_str().unwrap_or("balance").to_string();
        let billing_url = billing["url"].as_str().unwrap_or_default().to_string();
        let paths = |k: &str| billing["paths"][k].as_str().unwrap_or_default().to_string();
        self.settings.set_form_billing_enabled = billing.is_object();
        self.settings.set_form_billing_kind = billing_kind;
        self.settings.set_form_billing_auth_style =
            billing["auth_style"].as_str().map(String::from);
        let put = |slot: &mut Option<Entity<InputState>>,
                   v: String,
                   window: &mut Window,
                   cx: &mut Context<Self>| {
            if let Some(input) = slot {
                input.update(cx, |s, cx| s.set_value(&v, window, cx));
            }
        };
        let id_value = id.unwrap_or_default().to_string();
        put(&mut self.settings.set_form_id, id_value, window, cx);
        put(&mut self.settings.set_form_url, url, window, cx);
        put(&mut self.settings.set_form_model, model, window, cx);
        put(&mut self.settings.set_form_name, name, window, cx);
        put(
            &mut self.settings.set_form_model_input,
            String::new(),
            window,
            cx,
        );
        put(
            &mut self.settings.set_form_billing_url,
            billing_url,
            window,
            cx,
        );
        put(
            &mut self.settings.set_form_path_balance,
            paths("balance"),
            window,
            cx,
        );
        put(
            &mut self.settings.set_form_path_currency,
            paths("currency"),
            window,
            cx,
        );
        put(
            &mut self.settings.set_form_path_5h,
            paths("usage_5h"),
            window,
            cx,
        );
        put(
            &mut self.settings.set_form_path_7d,
            paths("usage_7d"),
            window,
            cx,
        );
        put(
            &mut self.settings.set_form_path_resets,
            paths("resets"),
            window,
            cx,
        );
        self.settings.set_form_context_windows = prefill;
        self.close_context_window_edit();
    }
}

/// 表单行(标签 + 输入实体;InputState 必须挂树才能聚焦输入)
/// Models 区:标题+intro+通告 / 行卡列表(编辑内嵌 / setup 姿态)/ 添加块
pub(crate) fn models_section(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let mut col = div()
        .v_flex()
        .gap(px(12.))
        .child(section_title(t!("settings.models_section")))
        .children(
            st.settings
                .saved_provider_notice
                .as_ref()
                .map(|name| saved_notice(name)),
        )
        // 设置动作页内通告(单槽覆盖;不走聊天区)
        .children(st.settings.settings_notice.as_ref().map(|(ok, msg)| {
            div()
                .id("settings-notice")
                .debug_selector(|| "settings-notice".to_string())
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(if *ok {
                    theme::SUCCESS()
                } else {
                    theme::DANGER()
                })
                .child(format!("{} {msg}", if *ok { "✓" } else { "⚠" }))
        }));
    let providers: Vec<serde_json::Value> = st.settings.settings_snapshot["providers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    // 行卡列表(与标题块之间 extra 12 空气,gap 8)
    let mut rows = div().v_flex().gap(px(8.)).mt(px(12.));
    if providers.is_empty() {
        rows = rows.child(caption_line(t!("settings.registry_empty")));
    }
    for p in &providers {
        let id = p["id"].as_str().unwrap_or_default();
        // 首运行 setup 姿态:未配置的默认 provider 直接是打开的设置卡
        if st.provider_setup_posture(id) {
            rows = rows.child(setup_card(store, cx, id));
            continue;
        }
        rows = rows.child(provider_row_card(store, cx, p));
    }
    col = col.child(rows).child(add_block(store, cx));
    col
}

/// 保存通告行(12/success)
pub(crate) fn saved_notice(name: &str) -> impl IntoElement {
    div()
        .id("provider-saved-notice")
        .text_size(px(12.))
        .text_color(theme::SUCCESS())
        .child(t!("settings.saved", name = name))
}

/// 单个 provider 卡片:圆标 avatar + 名称 + URL 链接行;
/// 右侧 = 上次刷新「N 小时前」+ 刷新钮 + 计费行;当前 defaultProvider
/// = BRAND 蓝描边。点卡片展开编辑器
pub(crate) fn provider_row_card(
    store: &Entity<AppStore>,
    cx: &App,
    p: &serde_json::Value,
) -> impl IntoElement {
    let st = store.read(cx);
    let id = p["id"].as_str().unwrap_or_default().to_string();
    let name = p["display_name"]
        .as_str()
        .filter(|v| !v.is_empty())
        .unwrap_or(id.as_str())
        .to_string();
    let base_url = p["base_url"].as_str().unwrap_or_default().to_string();
    let cred_ready = p["credentialReady"].as_bool().unwrap_or(false);
    let open = st.settings.editing_provider.as_deref() == Some(id.as_str());
    let refreshing = st.settings.billing_refreshing.as_deref() == Some(id.as_str());
    let cache = p["billing_cache"].clone();
    let has_billing = p["billing"].is_object();
    let row_sel = sid("provider-row", &id);
    let mut card = div()
        .id(row_sel.clone())
        .debug_selector(move || row_sel.to_string())
        .v_flex()
        .gap(px(10.))
        .rounded(px(12.))
        .border_1()
        // 编辑中的卡 = 选中态(图1 蓝框);其余中性
        .border_color(if open {
            theme::BRAND()
        } else {
            theme::BORDER()
        })
        .p(px(12.))
        .pr(px(14.))
        // 两行结构(行1 = 名称+刷新时间/钮;行2 = URL+计费值):
        // 左右两列的行基线对齐,右缘信息贴各自行尾
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(avatar(&name))
                .child(
                    div()
                        .v_flex()
                        .min_w(px(0.))
                        .flex_1()
                        .gap(px(3.))
                        // 行1:名称 + 凭据点 | 刷新时间 + 刷新钮
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .text_size(px(15.))
                                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                        .text_color(theme::LABEL())
                                        .child(name.clone()),
                                )
                                .child(credential_dot(cred_ready))
                                .child(div().flex_1())
                                .when(has_billing, |el| {
                                    el.child(billing_refresh_line(store, &id, &cache, refreshing))
                                }),
                        )
                        // 行2:URL | 计费值(配置了计费端点才有)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .text_size(px(12.))
                                        .text_color(theme::ONGOING())
                                        .truncate()
                                        .child(base_url.clone()),
                                )
                                .when(has_billing, |el| el.children(billing_value_line(&cache))),
                        ),
                )
                // 动作列:编辑/移除(纵排居中,不占内容行)
                .child(
                    div()
                        .v_flex()
                        .flex_shrink_0()
                        .gap(px(2.))
                        .child(row_edit_button(store, &id))
                        .child(row_remove_button(store, &id)),
                ),
        );
    if open {
        card = card.child(provider_editor(store, cx, &id, false));
    }
    card
}

/// 圆标 avatar(显示名前两词首字母;圆形 32px,LAYER 底)
pub(crate) fn avatar(name: &str) -> impl IntoElement {
    let words: Vec<String> = name
        .split_whitespace()
        .map(|w| w.chars().next().unwrap_or('?').to_string())
        .take(2)
        .collect();
    let ch = if words.is_empty() {
        "?".into()
    } else {
        words.concat()
    };
    div()
        .flex()
        .size(px(32.))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme::LAYER())
        .text_size(px(13.))
        .font_weight(gpui_kit::FontWeight::MEDIUM)
        .text_color(theme::LABEL_2())
        .child(ch)
}

/// 计费刷新行:「N 小时前」+ 刷新钮(拖拽刷新中禁用)
pub(crate) fn billing_refresh_line(
    store: &Entity<AppStore>,
    id: &str,
    cache: &serde_json::Value,
    refreshing: bool,
) -> impl IntoElement {
    let s = store.clone();
    let pid = id.to_string();
    let fetched_at = cache["fetched_at_ms"].as_u64();
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .text_size(px(11.))
        .text_color(theme::CAPTION())
        .children(fetched_at.map(|ms| {
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(fixed(LiumaIcon::Clock, 11.))
                .child(relative_time(ms))
        }))
        .child(
            div()
                .id(sid("billing-refresh", id))
                .flex_shrink_0()
                .cursor_pointer()
                .text_color(theme::LABEL_3())
                .hover(|s| s.text_color(theme::LABEL()))
                .child(fixed(IconName::LoaderCircle, 12.))
                .when(refreshing, |el| el.text_color(theme::ONGOING()))
                .on_click(move |_, _, cx| {
                    let pid = pid.clone();
                    s.update(cx, |st, cx| {
                        if !refreshing {
                            st.refresh_billing_now(&pid, false, cx);
                        }
                    });
                }),
        )
}

/// 计费数值行:余额「剩余: 9.52 CNY」/ 用量「5小时: 6% 7天: 6% 4d22h」
pub(crate) fn billing_value_line(cache: &serde_json::Value) -> Option<impl IntoElement> {
    let row = div().flex().items_center().gap(px(6.)).text_size(px(12.));
    match cache["kind"].as_str() {
        Some("balance") => {
            let amount = cache["amount"].as_str()?;
            let currency = cache["currency"].as_str().unwrap_or("");
            Some(
                row.child(t!("settings.remaining"))
                    .child(
                        div()
                            .text_color(theme::SUCCESS())
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .child(amount.to_string()),
                    )
                    .child(
                        div()
                            .text_color(theme::LABEL_3())
                            .child(currency.to_string()),
                    )
                    .into_any_element(),
            )
        }
        Some("usage") => {
            // 紧凑单行(右缘与 URL 行基线对齐,不撑卡高):
            // 5小时: 12%  7天: 16%  🕐重置倒计时
            let resets = cache["resets"].as_str().and_then(resets_countdown);
            let row = div().flex().items_center().gap(px(8.));
            let mut out = row;
            let mut any = false;
            for (label, pct) in [
                (t!("settings.quota_5h"), cache["pct_5h"].as_u64()),
                (t!("settings.quota_7d"), cache["pct_7d"].as_u64()),
            ] {
                let Some(v) = pct else { continue };
                any = true;
                out = out
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme::CAPTION())
                            .child(label),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .text_color(theme::LABEL())
                            .child(format!("{v}%")),
                    );
            }
            if let Some(cd) = resets {
                any = true;
                out = out.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .text_size(px(11.))
                        .text_color(theme::CAPTION())
                        .child(fixed(LiumaIcon::Clock, 11.))
                        .child(cd),
                );
            }
            any.then(|| out.into_any_element())
        }
        _ => None,
    }
}

/// 用量迷你进度条(width px、4px 高;填充 <70% 正常绿,≥70% 接近限额红)
pub(crate) fn usage_bar(pct: u64, width: f32) -> gpui_kit::AnyElement {
    let pct = pct.min(100);
    div()
        .w(px(width))
        .h(px(4.))
        .rounded(px(2.))
        .bg(theme::BORDER_2())
        .overflow_hidden()
        .child(
            div()
                .w(px(width * pct as f32 / 100.))
                .h_full()
                .rounded(px(2.))
                .bg(if pct >= 70 {
                    theme::DANGER()
                } else {
                    theme::SUCCESS()
                }),
        )
        .into_any_element()
}

/// 重置倒计时:毫秒戳 → 剩余「4天22时 / 3时12分 / 45分」;缺席/非时间戳/已过 = None
pub(crate) fn resets_countdown(resets: &str) -> Option<String> {
    let ts = resets.parse::<u64>().ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    let mins = ts.saturating_sub(now) / 60_000;
    if mins >= 60 * 24 {
        Some(
            t!(
                "settings.quota_expiry",
                days = mins / (60 * 24),
                hours = (mins % (60 * 24)) / 60
            )
            .into_owned(),
        )
    } else if mins >= 60 {
        Some(
            t!(
                "settings.quota_expiry_hm",
                hours = mins / 60,
                mins = mins % 60
            )
            .into_owned(),
        )
    } else if mins > 0 {
        Some(t!("settings.quota_expiry_m", mins = mins).into_owned())
    } else {
        None
    }
}

/// 毫秒时间戳 → 「N 分钟前 / N 小时前 / N 天前」
pub(crate) fn relative_time(ms: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mins = now.saturating_sub(ms) / 60_000;
    if mins < 1 {
        // 1 分钟内 = 刚刚(与 shell relative_time_l 同语义;刷新后
        // 立即显示「0 分钟前」是缺档表现)
        t!("time.rel_just_now").into_owned()
    } else if mins < 60 {
        t!("time.rel_mins_ago", n = mins).into_owned()
    } else if mins < 60 * 24 {
        t!("time.rel_hours_ago", n = mins / 60).into_owned()
    } else {
        t!("time.rel_days_ago", n = mins / (60 * 24)).into_owned()
    }
}

/// 凭据状态圆点(8px 实心,success/error)
pub(crate) fn credential_dot(configured: bool) -> impl IntoElement {
    div()
        .flex()
        .size(px(8.))
        .flex_shrink_0()
        .rounded_full()
        .bg(if configured {
            theme::SUCCESS()
        } else {
            theme::DANGER()
        })
}

/// 行头「编辑」钮(28h r14 边框胶囊;再点收起)
pub(crate) fn row_edit_button(store: &Entity<AppStore>, id: &str) -> impl IntoElement {
    let s = store.clone();
    let pid = id.to_string();
    let sel = sid("provider-edit", id);
    div()
        .id(sel.clone())
        .debug_selector(move || sel.to_string())
        .flex()
        .flex_shrink_0()
        .h(px(28.))
        .items_center()
        .px(px(10.))
        .rounded(px(14.))
        .border_1()
        .border_color(theme::BORDER())
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(theme::LABEL_2())
        .hover(|s| s.bg(theme::DOCK()))
        .child(t!("common.edit"))
        .on_click(move |_, window, cx| {
            let pid = pid.clone();
            s.update(cx, |st, cx| {
                if st.settings.editing_provider.as_deref() == Some(&pid) {
                    st.close_provider_editor(&pid, cx);
                } else {
                    st.open_provider_editor(&pid, window, cx);
                }
            });
        })
}

/// 行头「移除」钮(28h 胶囊,危险色文字)
pub(crate) fn row_remove_button(store: &Entity<AppStore>, id: &str) -> impl IntoElement {
    let s = store.clone();
    let pid = id.to_string();
    let sel = sid("provider-remove", id);
    div()
        .id(sel.clone())
        .debug_selector(move || sel.to_string())
        .flex()
        .flex_shrink_0()
        .h(px(28.))
        .items_center()
        .px(px(10.))
        .rounded(px(14.))
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(theme::DANGER())
        .hover(|s| s.bg(theme::DOCK()))
        .child(t!("common.remove"))
        .on_click(move |_, _, cx| {
            let pid = pid.clone();
            s.update(cx, |st, cx| st.ask_delete_provider(&pid, cx));
        })
}

/// 首运行 setup 卡(填充模块 = 该 provider 在页面上的
/// 存在形式,内嵌编辑卡且凭据必填)
pub(crate) fn setup_card(store: &Entity<AppStore>, cx: &App, id: &str) -> impl IntoElement {
    let sel = sid("provider-setup", id);
    div()
        .id(sel.clone())
        .debug_selector(move || sel.to_string())
        .v_flex()
        .rounded(px(12.))
        .bg(theme::SIDEBAR())
        .p(px(14.))
        .pr(px(16.))
        .child(provider_editor(store, cx, id, true))
}

/// 添加块(两入口):目录选择卡 / 自定义表单卡 /
/// 闭态 = 两个 dashed 添加钮(「添加提供方」= 目录流;「添加自定义
/// 提供方」= 自由表单)
pub(crate) fn add_block(store: &Entity<AppStore>, cx: &App) -> gpui_kit::AnyElement {
    let st = store.read(cx);
    if st.settings.adding_provider {
        return div()
            .id("provider-add-card")
            .debug_selector(|| "provider-add-card".to_string())
            .v_flex()
            .gap(px(14.))
            .rounded(px(12.))
            .bg(theme::SIDEBAR())
            .p(px(14.))
            .pr(px(16.))
            .child(provider_editor(store, cx, "", false))
            .into_any_element();
    }
    let s_catalog = store.clone();
    div()
        .id("provider-add")
        .debug_selector(|| "provider-add".to_string())
        .w_full()
        .h(px(44.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .rounded(px(12.))
        .border_1()
        .border_color(theme::BORDER_2())
        .border_dashed()
        .cursor_pointer()
        .text_size(px(14.))
        .text_color(theme::LABEL_3())
        .hover(|s| s.bg(theme::LAYER()).text_color(theme::LABEL_2()))
        .child(fixed(IconName::Plus, 14.))
        .child(t!("settings.add_provider"))
        .on_click(move |_, window, cx| {
            s_catalog.update(cx, |st, cx| st.open_provider_add_builtin(window, cx));
        })
        .into_any_element()
}

/// 编辑卡:内置模式(提供方下拉 + API 密钥 + 「自定义设置」折叠;
/// 适配器与目录绑定)/ 自定义模式(名称 / Base URL / API Key / API 格式
/// 三选 / 模型列表 / 计费端点,图3 字段序)。页脚右对齐 取消/应用。
/// setup/添加卡共用(无标题行)
pub(crate) fn provider_editor(
    store: &Entity<AppStore>,
    cx: &App,
    id: &str,
    setup: bool,
) -> impl IntoElement {
    let st = store.read(cx);
    let (s_add_model, s_fetch, s_billing, s_kind) =
        (store.clone(), store.clone(), store.clone(), store.clone());
    let close_id = id.to_string();
    let fetch_pid = id.to_string();
    let billing_pid = id.to_string();
    let editor_sel = if id.is_empty() {
        "provider-editor".to_string()
    } else {
        format!("provider-editor-{id}")
    };
    let editor_id = if id.is_empty() {
        gpui_kit::SharedString::from("provider-editor")
    } else {
        sid("provider-editor", id)
    };
    let input_row =
        |label: &str, slot: &Option<Entity<InputState>>, id_fmt: String| -> gpui_kit::AnyElement {
            div()
                .v_flex()
                .gap(px(6.))
                .child(field_label(label))
                .children(slot.as_ref().map(|e| {
                    div()
                        .debug_selector(move || id_fmt.clone())
                        .child(Input::new(e))
                }))
                .into_any_element()
        };
    let builtin = st.settings.builtin_mode;
    let adding = st.settings.adding_provider && id.is_empty();
    let picked_id = st.settings.builtin_picked.clone();
    let picked = liuma_core::settings::provider_catalog()
        .into_iter()
        .find(|e| e.id == picked_id);
    // 高级段(模型列表 + 计费端点):自定义模式平铺;内置模式收进
    // 「自定义设置」折叠
    let advanced = || -> Vec<gpui_kit::AnyElement> {
        vec![
            section_divider(),
            editor_models_block(store, cx, fetch_pid.clone()).into_any_element(),
            section_divider(),
            editor_billing_block(store, cx, id.to_string(), setup, billing_pid.clone())
                .into_any_element(),
        ]
    };
    let mut card = div()
        .id(editor_id)
        .debug_selector(move || editor_sel.clone())
        .v_flex()
        .gap(px(14.))
        .rounded(px(12.))
        .when(!setup && !id.is_empty(), |el| {
            el.bg(theme::SIDEBAR()).p(px(14.)).pr(px(16.))
        })
        // 标题与列表卡一致:显示名(非 id)
        .when(!setup && !id.is_empty(), |el| {
            let display = if builtin {
                picked.as_ref().map(|e| e.display_name.clone())
            } else {
                st.settings
                    .set_form_name
                    .as_ref()
                    .map(|i| i.read(cx).value().trim().to_string())
                    .filter(|v| !v.is_empty())
            };
            let title = display.unwrap_or_else(|| id.to_string());
            el.child(
                div()
                    .text_size(px(15.))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(theme::LABEL())
                    .child(title),
            )
        })
        // 添加态:分段模式 Tab(第三方目录 / 自定义 API)+ 模式导语
        .when(adding, |el| {
            let (s_tab_b, s_tab_c) = (store.clone(), store.clone());
            el.child(
                div()
                    .flex()
                    .self_start()
                    .gap(px(4.))
                    .p(px(2.))
                    .rounded(px(10.))
                    .bg(theme::LAYER())
                    .child(
                        div()
                            .id("provider-mode-builtin")
                            .debug_selector(|| "provider-mode-builtin".to_string())
                            .flex()
                            .items_center()
                            .px(px(12.))
                            .h(px(28.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .text_size(px(13.))
                            .when(builtin, |el| el.bg(theme::DOCK()))
                            .hover(|s| s.text_color(theme::LABEL_2()))
                            .text_color(if builtin {
                                theme::LABEL()
                            } else {
                                theme::CAPTION()
                            })
                            .child(t!("settings.provider_tab_builtin"))
                            .on_click(move |_, window, cx| {
                                s_tab_b.update(cx, |st, cx| {
                                    st.switch_provider_add_mode(true, window, cx)
                                });
                            }),
                    )
                    .child(
                        div()
                            .id("provider-mode-custom")
                            .debug_selector(|| "provider-mode-custom".to_string())
                            .flex()
                            .items_center()
                            .px(px(12.))
                            .h(px(28.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .text_size(px(13.))
                            .when(!builtin, |el| el.bg(theme::DOCK()))
                            .hover(|s| s.text_color(theme::LABEL_2()))
                            .text_color(if builtin {
                                theme::CAPTION()
                            } else {
                                theme::LABEL()
                            })
                            .child(t!("settings.provider_tab_custom"))
                            .on_click(move |_, window, cx| {
                                s_tab_c.update(cx, |st, cx| {
                                    st.switch_provider_add_mode(false, window, cx)
                                });
                            }),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme::CAPTION())
                    .child(if builtin {
                        t!("settings.provider_explain_builtin").into_owned()
                    } else {
                        t!("settings.provider_explain_custom").into_owned()
                    }),
            )
        })
        // 内置模式:提供方下拉仅添加态(编辑态锁定为该厂商,不可改即
        // 不展示;厂商名由卡头标题承担)
        .when(builtin && adding, |el| {
            el.child(
                div()
                    .v_flex()
                    .gap(px(6.))
                    .child(field_label(t!("settings.provider_label")))
                    .children(st.settings.builtin_select.as_ref().map(|s| {
                        div()
                            .w(px(280.))
                            .line_height(gpui_kit::relative(1.4))
                            .child(Select::new(s))
                    })),
            )
        })
        .when(!builtin, |el| {
            // Provider ID(仅添加态;编辑态 id 是路由键不可改,卡头已示)
            el.when(id.is_empty(), |el| {
                el.child(
                    div()
                        .v_flex()
                        .gap(px(6.))
                        .child(field_label(t!("settings.provider_id_label")))
                        .children(
                            st.settings
                                .set_form_id
                                .as_ref()
                                .map(|e| div().child(Input::new(e))),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme::CAPTION())
                                .child(t!("settings.provider_id_helper")),
                        ),
                )
            })
            .child(input_row(
                &t!("settings.name"),
                &st.settings.set_form_name,
                "field-name".into(),
            ))
            .child(input_row(
                &t!("settings.provider_field_base_url"),
                &st.settings.set_form_url,
                "field-url".into(),
            ))
        })
        .child(
            div()
                .v_flex()
                .gap(px(6.))
                .child(field_label(if builtin {
                    t!("settings.api_key")
                } else if setup {
                    t!("settings.api_key_required")
                } else {
                    t!("settings.api_key_plain")
                }))
                .children(st.settings.key_input.as_ref().map(|e| {
                    div()
                        .debug_selector(|| "field-key".to_string())
                        .child(Input::new(e))
                })),
        )
        .when(builtin, |el| {
            // 「自定义设置」折叠:目录
            // 契约字段只读展示——适配器与目录绑定,改写走自定义流
            el.child(
                div()
                    .id("builtin-advanced-toggle")
                    .debug_selector(|| "builtin-advanced-toggle".to_string())
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .text_color(theme::LABEL_2())
                    .hover(|s| s.text_color(theme::LABEL()))
                    .child(if st.settings.builtin_advanced_open {
                        fixed(IconName::ChevronDown, 13.)
                    } else {
                        fixed(IconName::ChevronRight, 13.)
                    })
                    .child(t!("settings.advanced_section"))
                    .on_mouse_down(gpui_kit::MouseButton::Left, {
                        let s = store.clone();
                        move |_, _, cx| {
                            s.update(cx, |st, cx| {
                                st.settings.builtin_advanced_open =
                                    !st.settings.builtin_advanced_open;
                                cx.notify();
                            });
                        }
                    }),
            )
            .when(st.settings.builtin_advanced_open, |el| {
                if let Some(entry) = &picked {
                    el // API 地址可覆盖(空 = 目录默认);适配器与目录绑定只读
                        .child(
                            div()
                                .v_flex()
                                .gap(px(6.))
                                .child(field_label(t!("settings.provider_field_base_url")))
                                .children(st.settings.set_form_url.as_ref().map(|e| {
                                    div()
                                        .debug_selector(|| "field-builtin-url".to_string())
                                        .child(Input::new(e))
                                }))
                                .child(url_protocol_hint(entry.dialect.as_str())),
                        )
                        // 模型清单可编辑草稿(目录预填打底,端点拉取更新,
                        // 随「应用」落盘——模型列表会更新,不锁目录契约)
                        .child(editor_models_block(store, cx, fetch_pid.clone()).into_any_element())
                        .child(info_line(
                            t!("settings.billing_preset"),
                            if entry.billing.is_some() {
                                t!("settings.billing_builtin").to_string()
                            } else {
                                t!("settings.billing_none").to_string()
                            },
                        ))
                        .when(
                            matches!(
                                entry.dialect.as_str(),
                                "anthropic-messages" | "openai-responses"
                            ),
                            |el| el.child(hosted_tools_row(store, st)),
                        )
                } else {
                    el.child(caption_line(t!("settings.catalog_missing")))
                }
            })
        })
        .when(!builtin, |el| {
            el.child(
                div()
                    .v_flex()
                    .gap(px(6.))
                    .child(field_label(t!("settings.api_format")))
                    .children(st.settings.dialect_select.as_ref().map(|s| {
                        div()
                            .w(px(280.))
                            .line_height(gpui_kit::relative(1.4))
                            .child(Select::new(s))
                    })),
            )
            .children(advanced())
            .when(
                matches!(
                    st.settings.set_form_dialect.as_str(),
                    "anthropic-messages" | "openai-responses"
                ),
                |el| el.child(hosted_tools_row(store, st)),
            )
        });
    let _ = (&s_add_model, &s_fetch, &s_billing, &s_kind);
    // 页脚:右对齐 取消/应用(官方 Button:Primary 变体自带 hover 降档)
    let s_cancel = store.clone();
    let s_apply = store.clone();
    let cancel_label: gpui_kit::SharedString = t!("common.cancel").into_owned().into();
    let apply_label: gpui_kit::SharedString = if st.settings.adding_provider {
        t!("settings.create_provider").into_owned().into()
    } else {
        t!("common.apply").into_owned().into()
    };
    card = card.child(
        div()
            .flex()
            .justify_end()
            .items_center()
            .gap(px(8.))
            .child(
                Button::new("provider-editor-cancel")
                    .debug_selector(|| "provider-editor-cancel".to_string())
                    .small()
                    .label(cancel_label)
                    .on_click(move |_, _, cx| {
                        let id = close_id.clone();
                        s_cancel.update(cx, |st, cx| st.close_provider_editor(&id, cx));
                    }),
            )
            .child(
                Button::new("provider-editor-apply")
                    .debug_selector(|| "provider-editor-apply".to_string())
                    .small()
                    .primary()
                    .label(apply_label)
                    .on_click(move |_, _, cx| {
                        s_apply.update(cx, |st, cx| st.apply_provider_editor(cx));
                    }),
            ),
    );
    card.into_any_element()
}

/// 模型列表块(图2:空态虚线框;行删除;端点拉取 + 手动添加)
pub(crate) fn editor_models_block(
    store: &Entity<AppStore>,
    cx: &App,
    fetch_pid: String,
) -> impl IntoElement {
    let st = store.read(cx);
    let (s_add_model, s_fetch) = (store.clone(), store.clone());
    div()
        .v_flex()
        .gap(px(8.))
        // 段头:模型目录 | 获取可用模型(文字链)
        .child(
            div()
                .flex()
                .items_center()
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(theme::LABEL())
                        .child(t!("settings.model_list")),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("models-fetch")
                        .debug_selector(|| "models-fetch".to_string())
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .cursor_pointer()
                        .text_size(px(13.))
                        .text_color(theme::LABEL_2())
                        .hover(|s| s.text_color(theme::LABEL()))
                        .when(st.settings.model_fetch_loading, |el| {
                            el.text_color(theme::ONGOING())
                        })
                        .child(fixed(IconName::Globe, 13.))
                        .child(t!("settings.fetch_from_endpoint"))
                        .on_click(move |_, _, cx| {
                            let pid = fetch_pid.clone();
                            s_fetch.update(cx, |st, cx| st.open_fetch_models(&pid, cx));
                        }),
                ),
        )
        .child(if st.settings.set_form_models.is_empty() {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(10.))
                .border_1()
                .border_color(theme::BORDER_2())
                .border_dashed()
                .px(px(12.))
                .py(px(14.))
                .text_size(px(13.))
                .text_color(theme::CAPTION())
                .child(fixed(IconName::Info, 14.))
                .child(t!("settings.models_empty"))
                .into_any_element()
        } else {
            div()
                .v_flex()
                .gap(px(4.))
                .children(
                    st.settings
                        .set_form_models
                        .iter()
                        .enumerate()
                        .map(|(ix, m)| model_draft_row(store, cx, ix, m)),
                )
                .into_any_element()
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .children(
                    st.settings
                        .set_form_model_input
                        .as_ref()
                        .map(|e| div().flex_1().child(Input::new(e)).into_any_element()),
                )
                .child(
                    div()
                        .id("model-add")
                        .debug_selector(|| "model-add".to_string())
                        .flex()
                        .h(px(36.))
                        .flex_shrink_0()
                        .items_center()
                        .gap(px(4.))
                        .px(px(10.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme::BORDER())
                        .cursor_pointer()
                        .text_size(px(13.))
                        .text_color(theme::LABEL_2())
                        .hover(|s| s.bg(theme::DOCK()))
                        .child(fixed(IconName::Plus, 13.))
                        .child(t!("settings.add_model"))
                        .on_click(move |_, window, cx| {
                            s_add_model.update(cx, |st, cx| {
                                st.add_model_manual(window, cx);
                            });
                        }),
                ),
        )
        .child(caption_line(t!("settings.ctx_hint")))
}

/// 计费端点块(开关 + 形态 + URL + JSON 路径)
pub(crate) fn editor_billing_block(
    store: &Entity<AppStore>,
    cx: &App,
    id: String,
    setup: bool,
    billing_pid: String,
) -> impl IntoElement {
    let st = store.read(cx);
    let (s_billing, s_kind) = (store.clone(), store.clone());
    let input_row =
        |label: &str, slot: &Option<Entity<InputState>>, id_fmt: String| -> gpui_kit::AnyElement {
            div()
                .v_flex()
                .gap(px(6.))
                .child(field_label(label))
                .children(slot.as_ref().map(|e| {
                    div()
                        .debug_selector(move || id_fmt.clone())
                        .child(Input::new(e))
                }))
                .into_any_element()
        };
    div()
        .v_flex()
        .gap(px(8.))
        .child(
            div()
                .id("billing-toggle")
                .debug_selector(|| "billing-toggle".to_string())
                .flex()
                .items_center()
                .justify_between()
                .child(field_label(t!("settings.billing_endpoint")))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(if st.settings.set_form_billing_enabled {
                            theme::LABEL_2()
                        } else {
                            theme::CAPTION()
                        })
                        .child(if st.settings.set_form_billing_enabled {
                            t!("settings.enabled")
                        } else {
                            t!("settings.disabled")
                        })
                        .child(
                            Switch::new("billing-enabled")
                                .small()
                                .checked(st.settings.set_form_billing_enabled)
                                .color(theme::LABEL())
                                .on_click({
                                    let s = s_billing.clone();
                                    move |_, _, cx| {
                                        s.update(cx, |st, cx| st.toggle_billing_enabled(cx));
                                    }
                                }),
                        ),
                ),
        )
        .when(st.settings.set_form_billing_enabled, |el| {
            el.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(billing_kind_chip(
                        store,
                        t!("settings.billing_balance"),
                        "balance",
                        &st.settings.set_form_billing_kind,
                    ))
                    .child(billing_kind_chip(
                        store,
                        t!("settings.billing_usage"),
                        "usage",
                        &st.settings.set_form_billing_kind,
                    )),
            )
            .child(input_row(
                &t!("settings.query_url"),
                &st.settings.set_form_billing_url,
                "field-billing-url".into(),
            ))
            .children(if st.settings.set_form_billing_kind == "usage" {
                vec![
                    input_row(
                        &t!("settings.usage_path_5h"),
                        &st.settings.set_form_path_5h,
                        "field-p5h".into(),
                    ),
                    input_row(
                        &t!("settings.usage_path_7d"),
                        &st.settings.set_form_path_7d,
                        "field-p7d".into(),
                    ),
                    input_row(
                        &t!("settings.reset_path"),
                        &st.settings.set_form_path_resets,
                        "field-presets".into(),
                    ),
                ]
            } else {
                vec![
                    input_row(
                        &t!("settings.balance_path"),
                        &st.settings.set_form_path_balance,
                        "field-pbal".into(),
                    ),
                    input_row(
                        &t!("settings.currency_path"),
                        &st.settings.set_form_path_currency,
                        "field-pcur".into(),
                    ),
                ]
            })
            .when(!setup && !id.is_empty(), |el| {
                el.child(
                    div()
                        .id("billing-refresh-now")
                        .flex()
                        .h(px(30.))
                        .w(px(88.))
                        .items_center()
                        .justify_center()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme::BORDER())
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(
                            if st.settings.billing_refreshing.as_deref() == Some(id.as_str()) {
                                theme::ONGOING()
                            } else {
                                theme::LABEL_2()
                            },
                        )
                        .hover(|s| s.bg(theme::DOCK()))
                        .child(
                            if st.settings.billing_refreshing.as_deref() == Some(id.as_str()) {
                                t!("settings.billing_refreshing")
                            } else {
                                t!("settings.refresh_now")
                            },
                        )
                        .on_click(move |_, _, cx| {
                            let pid = billing_pid.clone();
                            s_kind.update(cx, |st, cx| {
                                st.refresh_billing_now(&pid, true, cx);
                            });
                        }),
                )
            })
        })
}

/// 草稿模型行:名称 + 上下文窗口 chip(展开行内编辑)+ 移除
pub(crate) fn model_draft_row(
    store: &Entity<AppStore>,
    cx: &App,
    ix: usize,
    model: &str,
) -> impl IntoElement {
    let st = store.read(cx);
    let editing = st.settings.context_window_edit.as_deref() == Some(model);
    // 窗口 chip:已覆盖 = 「窗口 128,000」;无覆盖 = 「窗口 默认」(点击展开编辑)
    let chip_text = st
        .settings
        .set_form_context_windows
        .get(model)
        .map(|v| t!("settings.ctx_window", value = grouped_tokens(*v)).into_owned())
        .unwrap_or_else(|| t!("settings.ctx_window_default").to_string());
    let s_chip = store.clone();
    let s_remove = store.clone();
    let chip_model = model.to_string();
    div()
        .v_flex()
        .gap(px(4.))
        .child(
            div()
                .id(sid("model-draft", &ix.to_string()))
                .debug_selector(|| format!("model-draft-{ix}"))
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(30.))
                .px(px(10.))
                .rounded(px(8.))
                .bg(theme::SIDEBAR())
                .text_size(px(13.))
                .text_color(theme::LABEL_2())
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .child(model.to_string()),
                )
                .child(
                    div()
                        .id(sid("model-window", &ix.to_string()))
                        .debug_selector(|| format!("model-window-{ix}"))
                        .flex()
                        .h(px(22.))
                        .flex_shrink_0()
                        .items_center()
                        .px(px(8.))
                        .rounded(px(11.))
                        .border_1()
                        .border_color(if editing {
                            theme::BRAND()
                        } else {
                            theme::BORDER()
                        })
                        .cursor_pointer()
                        .text_size(px(11.))
                        .text_color(if editing {
                            theme::LABEL()
                        } else {
                            theme::CAPTION()
                        })
                        .hover(|s| s.text_color(theme::LABEL()))
                        .child(chip_text)
                        .on_click(move |_, window, cx| {
                            let m = chip_model.clone();
                            s_chip
                                .update(cx, |st, cx| st.begin_context_window_edit(&m, window, cx));
                        }),
                )
                .child(
                    div()
                        .id(sid("model-draft-remove", &ix.to_string()))
                        .flex()
                        .size(px(20.))
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .text_color(theme::CAPTION())
                        .hover(|s| s.bg(theme::DOCK()).text_color(theme::DANGER()))
                        .child(fixed(IconName::Close, 12.))
                        .on_click(move |_, _, cx| {
                            s_remove.update(cx, |st, cx| st.remove_form_model(ix, cx));
                        }),
                ),
        )
        .when(editing, |el| el.child(context_window_edit_row(store, cx)))
}

/// 窗口行内编辑行(展开态):输入 + 应用/取消;非法时行内提示
pub(crate) fn context_window_edit_row(store: &Entity<AppStore>, cx: &App) -> gpui_kit::AnyElement {
    let st = store.read(cx);
    let (s_apply, s_cancel) = (store.clone(), store.clone());
    div()
        .v_flex()
        .gap(px(4.))
        .px(px(10.))
        .pb(px(2.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .children(st.settings.context_window_input.as_ref().map(|e| {
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .debug_selector(|| "model-window-input".to_string())
                        .child(Input::new(e))
                }))
                .child(
                    div()
                        .id("model-window-apply")
                        .debug_selector(|| "model-window-apply".to_string())
                        .flex()
                        .h(px(36.))
                        .flex_shrink_0()
                        .items_center()
                        .px(px(10.))
                        .rounded(px(8.))
                        .bg(theme::DOCK())
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(theme::LABEL())
                        .hover(|s| s.bg(theme::BUBBLE()))
                        .child(t!("common.apply"))
                        .on_click(move |_, _, cx| {
                            s_apply.update(cx, |st, cx| {
                                st.commit_context_window_edit(cx);
                            });
                        }),
                )
                .child(
                    div()
                        .id("model-window-cancel")
                        .debug_selector(|| "model-window-cancel".to_string())
                        .flex()
                        .h(px(36.))
                        .flex_shrink_0()
                        .items_center()
                        .px(px(10.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme::BORDER())
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(theme::LABEL_2())
                        .hover(|s| s.bg(theme::DOCK()))
                        .child(t!("common.cancel"))
                        .on_click(move |_, _, cx| {
                            s_cancel.update(cx, |st, cx| st.cancel_context_window_edit(cx));
                        }),
                ),
        )
        .when(st.settings.context_window_error, |el| {
            el.child(
                div()
                    .text_size(px(11.))
                    .text_color(theme::DANGER())
                    .child(t!("settings.ctx_invalid_hint")),
            )
        })
        .into_any_element()
}

/// 计费形态 chip(余额 / 用量)
pub(crate) fn billing_kind_chip(
    store: &Entity<AppStore>,
    label: impl Into<gpui_kit::SharedString>,
    kind: &'static str,
    selected: &str,
) -> impl IntoElement {
    let label = label.into();
    let s = store.clone();
    let active = kind == selected;
    div()
        .id(sid("billing-kind", kind))
        .flex()
        .h(px(28.))
        .items_center()
        .px(px(8.))
        .rounded(px(14.))
        .border_1()
        .border_color(if active {
            theme::BRAND()
        } else {
            theme::BORDER()
        })
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(if active {
            theme::LABEL()
        } else {
            theme::LABEL_3()
        })
        .hover(|s| s.bg(theme::DOCK()))
        .child(label.to_string())
        .on_click(move |_, _, cx| {
            let k = kind.to_string();
            s.update(cx, |st, cx| st.set_billing_kind(&k, cx));
        })
}

/// 字段标签(12/500 secondary)
/// 厂商托管工具勾选行(provider 编辑卡;仅声明表所在的两个面显示)。
/// 服务端执行面,非客户端工具——开关只决定请求是否声明该工具
pub(crate) fn hosted_tools_row(store: &Entity<AppStore>, st: &AppStore) -> gpui_kit::AnyElement {
    div()
        .v_flex()
        .gap(px(6.))
        .child(field_label(t!("settings.hosted_tools")))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    Switch::new("hosted-web-search")
                        .small()
                        .checked(st.settings.set_form_hosted_web_search)
                        .small()
                        .on_click({
                            let store = store.clone();
                            move |_, _, cx| {
                                store.update(cx, |st, cx| {
                                    st.settings.set_form_hosted_web_search =
                                        !st.settings.set_form_hosted_web_search;
                                    cx.notify();
                                });
                            }
                        }),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme::LABEL_2())
                        .child(t!("settings.hosted_web_search")),
                ),
        )
        .into_any_element()
}

/// 内置卡 API 地址的协议提示(按目录条目方言整句;未知方言走通用句。
/// i18n 守卫要求 t! 键为字面量,故在 match 臂内逐键调用)
pub(crate) fn url_protocol_hint(dialect: &str) -> gpui_kit::AnyElement {
    let line = || div().text_size(px(12.)).text_color(theme::CAPTION());
    match dialect {
        "anthropic-messages" => line().child(t!("settings.url_protocol_hint_anthropic")),
        "openai-responses" | "glm-responses" => {
            line().child(t!("settings.url_protocol_hint_responses"))
        }
        "deepseek-chat" | "openai-completions" => {
            line().child(t!("settings.url_protocol_hint_chat"))
        }
        _ => line().child(t!("settings.url_protocol_hint_compat")),
    }
    .into_any_element()
}

/// 从端点获取模型弹层(组件库 Dialog 层;store 经 with_window 桥打开)。
/// 候选多选 + 采纳;loading 态获取中——content builder 每帧重放实时读
/// model_fetch,loading→清单切换与「采纳」钮出现均无需手动刷新,故
/// 动态尾行一并收进 content(Dialog 静态 footer 槽不随帧重放)
pub(crate) fn open_fetch_models_dialog(
    store: &Entity<AppStore>,
    window: &mut Window,
    cx: &mut App,
) {
    use gpui_kit::component::WindowExt as _;
    let store = store.clone();
    let s_close = store.clone();
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(t!("settings.fetch_models_title"))
            .w(px(440.))
            .bg(theme::LAYER())
            .content({
                let s_rows = store.clone();
                // 取消钮 handler 的克隆源(名字区分两层:Fn 闭包体
                // 不得移出捕获变量,逐帧取新克隆)
                let s_close_src = s_close.clone();
                move |content, _, cx| {
                    let s_close = s_close_src.clone();
                    let st = s_rows.read(cx);
                    let Some(mf) = &st.settings.model_fetch else {
                        return content;
                    };
                    let loading = st.settings.model_fetch_loading;
                    let picked_count = mf.picked.iter().filter(|p| **p).count();
                    let mut card = div()
                        .id("models-fetch-card")
                        .debug_selector(|| "models-fetch-card".to_string())
                        .v_flex()
                        .gap(px(12.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme::CAPTION())
                                .child(t!("settings.picked_count", count = picked_count)),
                        );
                    if loading {
                        card = card.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .py(px(20.))
                                .justify_center()
                                .text_size(px(13.))
                                .text_color(theme::CAPTION())
                                .child(t!("settings.fetching")),
                        );
                    } else {
                        card = card.child(
                            div()
                                .id("models-fetch-list")
                                .v_flex()
                                .gap(px(2.))
                                .max_h(px(320.))
                                .overflow_y_scroll()
                                .children(mf.candidates.iter().enumerate().map(|(ix, m)| {
                                    let s_toggle = s_rows.clone();
                                    let picked = mf.picked.get(ix).copied().unwrap_or(false);
                                    div()
                                        .id(sid("fetch-cand", &ix.to_string()))
                                        .debug_selector(|| format!("fetch-cand-{ix}"))
                                        .flex()
                                        .items_center()
                                        .gap(px(8.))
                                        .h(px(30.))
                                        .px(px(8.))
                                        .rounded(px(8.))
                                        .cursor_pointer()
                                        .hover(|s| s.bg(theme::DOCK()))
                                        .child(
                                            div()
                                                .flex()
                                                .size(px(14.))
                                                .items_center()
                                                .justify_center()
                                                .rounded(px(4.))
                                                .border_1()
                                                .border_color(if picked {
                                                    theme::BRAND()
                                                } else {
                                                    theme::BORDER()
                                                })
                                                .bg(if picked {
                                                    theme::BRAND()
                                                } else {
                                                    theme::TRANSPARENT()
                                                })
                                                .text_color(theme::LABEL())
                                                .children(
                                                    picked.then(|| fixed(IconName::Check, 11.)),
                                                ),
                                        )
                                        .text_size(px(13.))
                                        .text_color(theme::LABEL_2())
                                        .child(m.clone())
                                        .on_click(move |_, _, cx| {
                                            s_toggle
                                                .update(cx, |st, cx| st.toggle_fetch_pick(ix, cx));
                                        })
                                })),
                        );
                    }
                    // 动作尾行(随 content 重放:loading 中只留取消)
                    let st = s_rows.read(cx);
                    let loading = st.settings.model_fetch_loading;
                    let picked_count = st
                        .settings
                        .model_fetch
                        .as_ref()
                        .map(|mf| mf.picked.iter().filter(|p| **p).count())
                        .unwrap_or(0);
                    card = card.child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(
                                div()
                                    .id("models-fetch-cancel")
                                    .debug_selector(|| "models-fetch-cancel".to_string())
                                    .flex()
                                    .h(px(32.))
                                    .items_center()
                                    .px(px(14.))
                                    .rounded(px(16.))
                                    .border_1()
                                    .border_color(theme::BORDER())
                                    .cursor_pointer()
                                    .text_size(px(12.))
                                    .text_color(theme::LABEL_2())
                                    .hover(|s| s.bg(theme::DOCK()))
                                    .child(t!("common.cancel"))
                                    .on_click(move |_, window, cx| {
                                        s_close.update(cx, |st, cx| st.close_fetch_modal(cx));
                                        window.close_dialog(cx);
                                    }),
                            )
                            .when(!loading, |el| {
                                let s_adopt = s_rows.clone();
                                el.child(
                                    div()
                                        .id("models-fetch-adopt")
                                        .debug_selector(|| "models-fetch-adopt".to_string())
                                        .flex()
                                        .h(px(32.))
                                        .items_center()
                                        .px(px(14.))
                                        .rounded(px(16.))
                                        .bg(theme::DOCK())
                                        .cursor_pointer()
                                        .text_size(px(12.))
                                        .text_color(theme::LABEL())
                                        .hover(|s| s.bg(theme::BUBBLE()))
                                        .child(t!("settings.adopt", count = picked_count))
                                        .on_click(move |_, window, cx| {
                                            s_adopt.update(cx, |st, cx| {
                                                st.adopt_fetched_models(cx);
                                            });
                                            window.close_dialog(cx);
                                        }),
                                )
                            }),
                    );
                    content.child(card)
                }
            })
            .on_close({
                let s_close = s_close.clone();
                move |_, _, cx| {
                    s_close.update(cx, |st, cx| st.close_fetch_modal(cx));
                }
            })
    });
}

/// Provider 删除确认(组件库 Dialog 层;store 经 with_window 桥打开)。
/// 取消 = 纯关闭(无旗标可清);确认携 id 直派 store 动作
pub(crate) fn open_delete_provider_dialog(
    store: &Entity<AppStore>,
    id: &str,
    window: &mut Window,
    cx: &mut App,
) {
    use gpui_kit::component::WindowExt as _;
    let s_confirm = store.clone();
    let pid = id.to_string();
    window.open_dialog(cx, move |dialog, _, _| {
        let s_confirm = s_confirm.clone();
        let pid = pid.clone();
        dialog
            .title(t!("settings.remove_provider", id = pid.clone()))
            .w(px(420.))
            .bg(theme::LAYER())
            .content(|content, _, _| {
                content.child(
                    div()
                        .debug_selector(|| "provider-delete-card".to_string())
                        .child(caption_line(t!("settings.remove_provider_desc"))),
                )
            })
            .footer(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("provider-delete-cancel")
                            .debug_selector(|| "provider-delete-cancel".to_string())
                            .flex()
                            .h(px(32.))
                            .items_center()
                            .px(px(14.))
                            .rounded(px(16.))
                            .border_1()
                            .border_color(theme::BORDER())
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(theme::LABEL_2())
                            .hover(|s| s.bg(theme::DOCK()))
                            .child(t!("common.cancel"))
                            .on_click(|_, window, cx| {
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        div()
                            .id("provider-delete-confirm")
                            .debug_selector(|| "provider-delete-confirm".to_string())
                            .flex()
                            .h(px(32.))
                            .items_center()
                            .px(px(14.))
                            .rounded(px(16.))
                            .border_1()
                            .border_color(theme::DANGER())
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(theme::DANGER())
                            .hover(|s| s.bg(theme::DOCK()))
                            .child(t!("common.remove"))
                            .on_click(move |_, window, cx| {
                                s_confirm.update(cx, |st, cx| st.confirm_delete_provider(&pid, cx));
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
}

#[cfg(test)]
/// 重置倒计时格式化:剩余窗按量级取「天/时/分」,已过或非时间戳缺席
#[cfg(test)]
mod countdown_tests {
    use super::resets_countdown;

    #[test]
    pub(crate) fn formats_remaining_window() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        // +1s 缓冲:断言按分钟取整,而 resets_countdown 内部会重新读一次
        // 时钟(晚于本行捕获的 now)。若恰好跨毫秒边界,floor 会少 1 分钟
        // (实测并行负载下「4天22时」偶发成「4天21时」)。缓冲把边界推到
        // 秒级,消除该非确定。
        let at = |mins: u64| (now + mins * 60_000 + 1_000).to_string();
        assert_eq!(
            resets_countdown(&at(4 * 1440 + 22 * 60)).as_deref(),
            Some("4天22时")
        );
        assert_eq!(
            resets_countdown(&at(3 * 60 + 12)).as_deref(),
            Some("3时12分")
        );
        assert_eq!(resets_countdown(&at(45)).as_deref(), Some("45分"));
        // 已过 / 非时间戳 → 缺席(不显示倒计时)
        let past = (now - 60 * 60_000).to_string();
        assert_eq!(resets_countdown(&past), None);
        assert_eq!(resets_countdown("soon"), None);
    }
}
