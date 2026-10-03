//! 决策模型页:表单解析纯函数、方法与配置视图(含解析测试)。

use super::*;

/// 决策超时输入:空 = 内置默认(2000ms,与
/// `liuma_decision::thresholds::DEFAULT_TIMEOUT_MS` 同源;desktop 不依赖
/// 该 crate,此处字面量由测试钉住)。纯数字毫秒,限 [100, 60000]——
/// 下限之下询问立即超时(功能假死式 fail-open),上限之上守卫的前置
/// 询问会把每个工具调用都挂住
pub(crate) fn parse_decision_timeout(raw: &str) -> Result<u64, ()> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(2000);
    }
    let ms: u64 = trimmed.parse().map_err(|_| ())?;
    (100..=60_000).contains(&ms).then_some(ms).ok_or(())
}

/// 决策置信输入:空 = None(内置默认,由装配层展开);0–1 纯小数。
/// 控件侧 NumberInput 自带步进与失焦 clamp(min/max),但键入自由文本
/// 不受其约束,保存前的这道校验仍是门槛。NaN/∞ 落不进 [0,1],同非法
pub(crate) fn parse_decision_high(raw: &str) -> Result<Option<f64>, ()> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let value = trimmed.parse::<f64>().map_err(|_| ())?;
    if !(0.0..=1.0).contains(&value) {
        return Err(());
    }
    Ok(Some(value))
}

impl AppStore {
    /// 设置快照里的决策条目(缺失 = 默认)
    pub(crate) fn decision_entry(&self) -> liuma_core::settings::DecisionEntry {
        serde_json::from_value::<liuma_core::settings::DecisionEntry>(
            self.settings.settings_snapshot["decision"].clone(),
        )
        .unwrap_or_default()
    }

    pub(crate) fn save_decision(
        &mut self,
        mutate: impl FnOnce(&mut liuma_core::settings::DecisionEntry),
        cx: &mut Context<Self>,
    ) {
        let mut entry = self.decision_entry();
        mutate(&mut entry);
        if self.bridge.host().upsert_decision_settings(entry).is_ok() {
            self.settings_refresh(cx);
        }
        cx.notify();
    }

    /// 决策表单七输入惰建(挂窗一次;`sync_decision_form` 亦会自足调用,
    /// 不依赖 attach 先后)
    pub(crate) fn ensure_decision_form_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings.decision_form_url.is_none() {
            self.settings.decision_form_url =
                Some(cx.new(|cx| {
                    InputState::new(window, cx).placeholder(t!("settings.url_placeholder"))
                }));
        }
        if self.settings.decision_form_model.is_none() {
            self.settings.decision_form_model =
                Some(cx.new(|cx| InputState::new(window, cx).placeholder("jev-latest")));
        }
        if self.settings.decision_form_key.is_none() {
            self.settings.decision_form_key = Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("settings.decision_key_placeholder"))
            }));
        }
        // 阈值/超时的 placeholder 直接写内置默认值:空输入 = 用默认,
        // placeholder 就是那份默认的可视形态。四行都是 NumberInput:
        // Number mask + step/min/max 给步进按钮与失焦 clamp(键入自由
        // 文本仍不受约束,保存前由 parse_* 把关)
        let number_input = |placeholder: &'static str,
                            step: f64,
                            min: f64,
                            max: f64,
                            window: &mut Window,
                            cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .mask_pattern(MaskPattern::Number {
                        separator: None,
                        fraction: None,
                    })
                    .step(step)
                    .min(min)
                    .max(max)
            })
        };
        if self.settings.decision_form_timeout.is_none() {
            self.settings.decision_form_timeout =
                Some(number_input("2000", 500., 100., 60000., window, cx));
        }
        if self.settings.decision_form_guard_high.is_none() {
            self.settings.decision_form_guard_high =
                Some(number_input("0.9", 0.05, 0., 1., window, cx));
        }
        if self.settings.decision_form_context_high.is_none() {
            self.settings.decision_form_context_high =
                Some(number_input("0.85", 0.05, 0., 1., window, cx));
        }
        if self.settings.decision_form_fold_high.is_none() {
            self.settings.decision_form_fold_high =
                Some(number_input("0.15", 0.05, 0., 1., window, cx));
        }
    }

    /// 决策表单回填(唯一入口:切到 Decision 分区,或开设置页时当前
    /// 已在该区)。**不得在渲染期或 [`Self::settings_refresh`] 内调用**:
    /// 那会覆盖用户正在输入的内容(同 `ask_custom_input_not_rewritten_each_frame`
    /// 锁住的契约)。key 恒不回填、只清空(write-only;明文不回显)。
    /// 阈值/超时回填现值:None(未配置)回空串 = 用内置默认。
    pub(crate) fn sync_decision_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_decision_form_inputs(window, cx);
        let entry = self.decision_entry();
        if let Some(input) = &self.settings.decision_form_url {
            input.update(cx, |s, cx| s.set_value(entry.base_url.as_str(), window, cx));
        }
        if let Some(input) = &self.settings.decision_form_model {
            input.update(cx, |s, cx| s.set_value(entry.model.as_str(), window, cx));
        }
        if let Some(input) = &self.settings.decision_form_key {
            input.update(cx, |s, cx| s.set_value("", window, cx));
        }
        if let Some(input) = &self.settings.decision_form_timeout {
            let v = entry.timeout_ms.to_string();
            input.update(cx, |s, cx| s.set_value(&v, window, cx));
        }
        for (input, high, default_text) in [
            (
                &self.settings.decision_form_guard_high,
                entry.guard_high,
                "0.9",
            ),
            (
                &self.settings.decision_form_context_high,
                entry.context_high,
                "0.85",
            ),
            (
                &self.settings.decision_form_fold_high,
                entry.fold_high,
                "0.15",
            ),
        ] {
            if let Some(input) = input {
                // 回填生效值:配置过 = 原样小数字面量(会话内保存往返不
                // 丢精度);未配置 = 内置默认字面量——NumberInput 的步进
                // 从当前文本起算,placeholder 不参与运算,空框按 + 会从
                // 区间下限起步而非默认值。手动清空仍是「回默认」入口
                // (保存时空串解析为 None)
                let v = high
                    .map(|h| h.to_string())
                    .unwrap_or_else(|| default_text.to_string());
                input.update(cx, |s, cx| s.set_value(&v, window, cx));
            }
        }
    }

    /// 决策配置保存(端点 / 模型 / 密钥 / 超时 / 置信阈值)。key 留空 =
    /// 不改已存(后端 `api_key == None` 保留原值);端点或模型为空、端点
    /// 无 http(s) 前缀、超时或阈值非法一律拒绝并内联通告——空端点会让
    /// `build_decision_port` 返回 `None`,整个功能无声失效。成功即清空
    /// key 明文并刷快照(圆点转「已配置」)。
    pub fn apply_decision_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base_url = self
            .settings
            .decision_form_url
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
            .unwrap_or_default();
        let model = self
            .settings
            .decision_form_model
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
            .unwrap_or_default();
        let key = self
            .settings
            .decision_form_key
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
            .unwrap_or_default();
        let timeout_raw = self
            .settings
            .decision_form_timeout
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
            .unwrap_or_default();
        let high_raw = |field: &Option<Entity<InputState>>| {
            field
                .as_ref()
                .map(|i| i.read(cx).value().trim().to_string())
                .unwrap_or_default()
        };
        let guard_high_raw = high_raw(&self.settings.decision_form_guard_high);
        let context_high_raw = high_raw(&self.settings.decision_form_context_high);
        let fold_high_raw = high_raw(&self.settings.decision_form_fold_high);
        if base_url.is_empty() {
            self.set_settings_notice(false, t!("settings.decision_url_empty"), cx);
            return;
        }
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            self.set_settings_notice(false, t!("settings.decision_url_invalid"), cx);
            return;
        }
        if model.is_empty() {
            self.set_settings_notice(false, t!("settings.decision_model_empty"), cx);
            return;
        }
        let Ok(timeout_ms) = parse_decision_timeout(&timeout_raw) else {
            self.set_settings_notice(false, t!("settings.decision_timeout_invalid"), cx);
            return;
        };
        let (Ok(guard_high), Ok(context_high), Ok(fold_high)) = (
            parse_decision_high(&guard_high_raw),
            parse_decision_high(&context_high_raw),
            parse_decision_high(&fold_high_raw),
        ) else {
            self.set_settings_notice(false, t!("settings.decision_high_invalid"), cx);
            return;
        };
        let mut entry = self.decision_entry();
        entry.base_url = base_url;
        entry.model = model;
        entry.timeout_ms = timeout_ms;
        entry.guard_high = guard_high;
        entry.context_high = context_high;
        entry.fold_high = fold_high;
        // 空 key = 保持 None,交给 host 保留原值(与 provider upsert 同惯例)。
        // 阈值/超时的「空」与此不同:不是「不改」而是「内置默认」——key 是
        // write-only 特例,其余字段以表单为准
        if !key.is_empty() {
            entry.api_key = Some(key);
        }
        if let Err(e) = self.bridge.host().upsert_decision_settings(entry) {
            self.set_settings_notice(false, t!("settings.save_failed", msg = &e.message), cx);
            return;
        }
        // 明文不留在控件里(也避免下次保存把同一 key 再提交一遍)
        if let Some(input) = &self.settings.decision_form_key {
            input.update(cx, |s, cx| s.set_value("", window, cx));
        }
        self.set_settings_notice(true, t!("settings.decision_saved"), cx);
        self.settings_refresh(cx);
    }

    /// 决策总开关
    pub fn toggle_decision_enabled(&mut self, cx: &mut Context<Self>) {
        self.save_decision(|e| e.enabled = !e.enabled, cx);
    }

    /// 场景状态(kind: approvals / stop / guard / context / fold)。`mode`
    /// 是分段控件的下标:0 = 关闭,1 = 仅记录(启用但 shadow),2 = 拦截
    /// (enforce)。approvals / stop 无 enforce 位,只用 0 / 1。
    ///
    /// 关闭(0)时**不动** enforce 位:重新开启能回到用户上次选的模式。
    /// 选中态由 enabled / enforce 共同推出,关闭态下标恒为 0——残留的
    /// enforce 不可见,也不会让控件读出第四种状态。
    pub fn set_decision_scenario(
        &mut self,
        kind: &'static str,
        mode: usize,
        cx: &mut Context<Self>,
    ) {
        self.save_decision(
            |e| {
                let on = mode > 0;
                let enforce = mode == 2;
                match kind {
                    "approvals" => e.approvals = on,
                    "stop" => e.stop = on,
                    "guard" => {
                        e.guard = on;
                        if on {
                            e.guard_enforce = enforce;
                        }
                    }
                    "context" => {
                        e.context = on;
                        if on {
                            e.context_enforce = enforce;
                        }
                    }
                    "fold" => {
                        e.fold = on;
                        if on {
                            e.fold_enforce = enforce;
                        }
                    }
                    // 未知 kind:不动配置(旧客户端读到新场景行时静默)
                    _ => {}
                }
            },
            cx,
        );
    }
}

/// 决策区配置块(端点 / 模型 / 密钥 + 保存)。输入是常驻表单:回填由
/// store 的 `sync_decision_form` 在「进入该区」时驱动,本函数只读不写。
pub(crate) fn decision_config_block(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let s_save = store.clone();
    // 明文 key 不出快照(与 providers 同惯例):「已配置」读 apiKeySet 布尔,
    // 不读 api_key——后者在快照里恒缺席,读它等于恒报「未配置」
    let key_configured = st.settings.settings_snapshot["decision"]["apiKeySet"]
        .as_bool()
        .unwrap_or(false);
    div()
        .id("decision-endpoint-block")
        .debug_selector(|| "decision-endpoint-block".to_string())
        .v_flex()
        .gap(px(10.))
        .child(field_input(
            t!("settings.provider_field_base_url"),
            "decision-url-input",
            &st.settings.decision_form_url,
        ))
        .child(field_input(
            t!("settings.decision_model_label"),
            "decision-model-input",
            &st.settings.decision_form_model,
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                // 弹性层持 flex_1:并排时若无此层,输入框会塌成小方块
                .child(div().flex_1().min_w(px(0.)).child(field_input(
                    t!("settings.api_key_plain"),
                    "decision-key-input",
                    &st.settings.decision_form_key,
                )))
                .child(
                    div()
                        .id("decision-key-state")
                        .debug_selector(|| "decision-key-state".to_string())
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap(px(5.))
                        .child(credential_dot(key_configured))
                        .child(div().text_size(px(11.)).text_color(theme::CAPTION()).child(
                            if key_configured {
                                t!("settings.decision_key_set")
                            } else {
                                t!("settings.decision_key_missing")
                            },
                        )),
                ),
        )
        // 端点/模型/密钥附着时才烘进端口,如实交代生效时机
        .child(
            div()
                .id("decision-attach-hint")
                .debug_selector(|| "decision-attach-hint".to_string())
                .text_size(px(11.))
                .text_color(theme::CAPTION())
                .child(t!("settings.decision_attach_hint")),
        )
        .child(field_number_hinted(
            t!("settings.decision_timeout_label"),
            "decision-timeout-input",
            &st.settings.decision_form_timeout,
            t!("settings.decision_timeout_hint"),
        ))
        // 阈值组说明:极性写明(守卫/裁判「≥ 才动作」,折叠反向「≤ 才裁」)
        .child(
            div()
                .id("decision-threshold-intro")
                .debug_selector(|| "decision-threshold-intro".to_string())
                .text_size(px(11.))
                .text_color(theme::CAPTION())
                .child(t!("settings.decision_threshold_intro")),
        )
        .child(field_number_hinted(
            t!("settings.decision_guard_high_label"),
            "decision-guard-high-input",
            &st.settings.decision_form_guard_high,
            t!("settings.decision_guard_high_hint"),
        ))
        .child(field_number_hinted(
            t!("settings.decision_context_high_label"),
            "decision-context-high-input",
            &st.settings.decision_form_context_high,
            t!("settings.decision_context_high_hint"),
        ))
        .child(field_number_hinted(
            t!("settings.decision_fold_high_label"),
            "decision-fold-high-input",
            &st.settings.decision_form_fold_high,
            t!("settings.decision_fold_high_hint"),
        ))
        .children(st.settings.settings_notice.as_ref().map(|(ok, msg)| {
            div()
                .debug_selector(|| "decision-settings-notice".to_string())
                .text_size(px(12.))
                .text_color(if *ok {
                    theme::SUCCESS()
                } else {
                    theme::DANGER()
                })
                .child(format!("{} {msg}", if *ok { "✓" } else { "⚠" }))
        }))
        .child(
            div().flex().justify_end().child(
                div()
                    .id("decision-save")
                    .debug_selector(|| "decision-save".to_string())
                    .flex()
                    .h(px(32.))
                    .items_center()
                    .px(px(14.))
                    .rounded(px(16.))
                    .border_1()
                    .border_color(theme::BORDER())
                    .cursor_pointer()
                    .text_size(px(13.))
                    .text_color(theme::LABEL_2())
                    .hover(|s| s.bg(theme::DOCK()))
                    .child(t!("common.save"))
                    .on_click(move |_, window, cx| {
                        s_save.update(cx, |st, cx| st.apply_decision_form(window, cx));
                    }),
            ),
        )
}

/// 决策模型区(System One 协议):端点/模型/密钥配置块 + 总开关 + 四场景
/// 开关(+ guard/context 的 enforce 位)。
pub(crate) fn decision_section(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let entry = serde_json::from_value::<liuma_core::settings::DecisionEntry>(
        st.settings.settings_snapshot["decision"].clone(),
    )
    .unwrap_or_default();
    let mut col = div()
        .v_flex()
        .gap(px(12.))
        .child(section_title(t!("settings.nav_decision")))
        .child(intro_line(t!("settings.decision_intro")))
        .child(decision_config_block(store, cx));

    // 场景行通用形态(标题 + 说明 + 单选组;主开关独立置顶)。
    // 三态一一对应底层两位:关闭(enabled=false)/ 仅记录(enabled, shadow)/
    // 拦截(enabled + enforce);approvals / stop 没有 enforce 位,只有两态。
    // 用库 `RadioGroup`(受控单选):语义就是「从 N 个里选一个」,选中点是
    // 品牌色实心 + 勾——形状信号,不靠底色深浅。不选 `TabBar::segmented`:
    // 那是切视图的标签页控件,且它的选中药丸被库硬编码成画布色,在本主题
    // 下与页面同色(深盘差 2/255)。
    let scenario_row = |store: &Entity<AppStore>,
                        id: &'static str,
                        label: String,
                        desc: &str,
                        mode: usize,
                        has_enforce: bool|
     -> gpui_kit::AnyElement {
        let st_row = store.clone();
        // 每项挂独立 selector,测试才能点到具体那一项。`.small()`:圆点
        // 14px + 标签 14px——Medium 档(16px 圆点 + 16px 标签)比行主文
        // (13px)还大,与字号纪律不符;XSmall 的 12px 标签又低于行主文,
        // 模式词是行内主控不该更小
        let item = |kind: &'static str, label: gpui_kit::SharedString| {
            let sel = format!("decision-mode-{id}-{kind}");
            Radio::new(gpui_kit::SharedString::from(sel.clone()))
                .debug_selector(move || sel.clone())
                .small()
                .label(label)
        };
        let mut items = vec![item("off", t!("settings.decision_mode_off").into())];
        if has_enforce {
            items.push(item("shadow", t!("settings.decision_mode_shadow").into()));
            items.push(item("block", t!("settings.decision_mode_block").into()));
        } else {
            items.push(item("on", t!("settings.decision_mode_on").into()));
        }
        div()
            .id(gpui_kit::SharedString::from(format!("decision-row-{id}")))
            .debug_selector(move || format!("decision-row-{id}"))
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .v_flex()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(theme::LABEL())
                            .child(label),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme::CAPTION())
                            .child(desc.to_string()),
                    ),
            )
            .child(
                div()
                    .id(gpui_kit::SharedString::from(format!("decision-mode-{id}")))
                    .debug_selector(move || format!("decision-mode-{id}"))
                    .flex_shrink_0()
                    .child(
                        RadioGroup::horizontal(gpui_kit::SharedString::from(format!(
                            "decision-mode-group-{id}"
                        )))
                        .selected_index(Some(mode))
                        .on_click(move |ix: &usize, _, cx| {
                            st_row.update(cx, |st, cx| st.set_decision_scenario(id, *ix, cx));
                        })
                        .children(items),
                    ),
            )
            .into_any_element()
    };

    // 三态下标:关闭 0 / 仅记录 1 / 拦截 2(enabled=false 时恒 0)
    let scenario_mode = |on: bool, enforce: bool| {
        if !on {
            0
        } else if enforce {
            2
        } else {
            1
        }
    };

    let st_master = store.clone();
    col = col.child(
        div()
            .id("decision-master")
            .debug_selector(|| "decision-master".to_string())
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(theme::LABEL())
                    .child(t!("settings.decision_master")),
            )
            .child(
                // 包装层只为布局回归测试持有 selector:库 Switch 的内部
                // track 不注册 debug bounds,层高即开关高(Small 16/Medium 20)
                div()
                    .id("decision-master-switch")
                    .debug_selector(|| "decision-master-switch".to_string())
                    .flex_shrink_0()
                    .child(
                        Switch::new("decision-master-toggle")
                            .small()
                            .checked(entry.enabled)
                            .small()
                            .color(theme::LABEL())
                            .on_click(move |_, _, cx| {
                                st_master.update(cx, |st, cx| st.toggle_decision_enabled(cx));
                            }),
                    ),
            ),
    );
    // 主开关关闭时场景行整体退灰(仍可查看,不可交互的语义由分段选中态承担)
    let _ = &entry;
    col = col
        .child(scenario_row(
            store,
            "approvals",
            t!("settings.decision_approvals").to_string(),
            &t!("settings.decision_approvals_desc"),
            scenario_mode(entry.approvals, false),
            false,
        ))
        .child(scenario_row(
            store,
            "stop",
            t!("settings.decision_stop").to_string(),
            &t!("settings.decision_stop_desc"),
            scenario_mode(entry.stop, false),
            false,
        ))
        .child(scenario_row(
            store,
            "guard",
            t!("settings.decision_guard").to_string(),
            &t!("settings.decision_guard_desc"),
            scenario_mode(entry.guard, entry.guard_enforce),
            true,
        ))
        .child(scenario_row(
            store,
            "context",
            t!("settings.decision_context").to_string(),
            &t!("settings.decision_context_desc"),
            scenario_mode(entry.context, entry.context_enforce),
            true,
        ))
        .child(scenario_row(
            store,
            "fold",
            t!("settings.decision_fold").to_string(),
            &t!("settings.decision_fold_desc"),
            scenario_mode(entry.fold, entry.fold_enforce),
            true,
        ));
    col
}

#[cfg(test)]
#[cfg(test)]
mod decision_form_parse_tests {
    use super::{parse_decision_high, parse_decision_timeout};

    /// 空 = 内置默认(2000,与 liuma_decision thresholds 同源的字面量);
    /// 纯数字毫秒限 [100, 60000];负数/小数/非数字/越界均非法
    #[test]
    pub(crate) fn parses_decision_timeout_draft() {
        assert_eq!(parse_decision_timeout(""), Ok(2000));
        assert_eq!(parse_decision_timeout("   "), Ok(2000));
        assert_eq!(parse_decision_timeout("2000"), Ok(2000));
        assert_eq!(parse_decision_timeout(" 1500 "), Ok(1500));
        assert_eq!(parse_decision_timeout("100"), Ok(100), "下边界含");
        assert_eq!(parse_decision_timeout("60000"), Ok(60000), "上边界含");
        assert_eq!(parse_decision_timeout("99"), Err(()));
        assert_eq!(parse_decision_timeout("0"), Err(()));
        assert_eq!(parse_decision_timeout("60001"), Err(()));
        assert_eq!(parse_decision_timeout("-1"), Err(()));
        assert_eq!(parse_decision_timeout("1.5"), Err(()));
        assert_eq!(parse_decision_timeout("2000ms"), Err(()));
        assert_eq!(parse_decision_timeout("abc"), Err(()));
    }

    /// 空 = None(内置默认);0–1 纯小数;越界、非数字、NaN/∞ 均非法
    /// (控件侧 NumberInput 的失焦 clamp 会先把可解析的越界值收敛,
    /// 这里是键入自由文本与手改路径的最后一道门)
    #[test]
    pub(crate) fn parses_decision_high_draft() {
        assert_eq!(parse_decision_high(""), Ok(None));
        assert_eq!(parse_decision_high("   "), Ok(None));
        assert_eq!(parse_decision_high("0.9"), Ok(Some(0.9)));
        assert_eq!(parse_decision_high(" 0.85 "), Ok(Some(0.85)));
        assert_eq!(parse_decision_high("0"), Ok(Some(0.0)));
        assert_eq!(parse_decision_high("1"), Ok(Some(1.0)));
        assert_eq!(parse_decision_high("90"), Err(()), "越上界");
        assert_eq!(parse_decision_high("1.5"), Err(()));
        assert_eq!(parse_decision_high("-0.1"), Err(()));
        assert_eq!(parse_decision_high("90%"), Err(()), "% 格式不再收");
        assert_eq!(parse_decision_high("abc"), Err(()));
        assert_eq!(parse_decision_high("NaN"), Err(()));
        assert_eq!(parse_decision_high("inf"), Err(()));
    }
}
