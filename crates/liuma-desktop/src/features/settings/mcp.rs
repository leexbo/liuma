//! MCP Server 管理页:详情表单/导入提交/列表开关与视图。

use super::*;

/// MCP 详情页状态(新增 / 编辑 / JSON 导入三态共用载体)
#[derive(Clone)]
pub struct McpDetailState {
    /// 编辑中的 server id(None = 新增;编辑态 id 锁定)
    pub editing: Option<String>,
    /// 页签:表单 / JSON 粘贴
    pub mode: McpDetailMode,
    /// 启用开关表单值
    pub form_enabled: bool,
    /// 表单:id(仅新增可输入;编辑态身份锁定)
    pub form_id: Option<Entity<InputState>>,
    pub form_command: Option<Entity<InputState>>,
    pub form_cwd: Option<Entity<InputState>>,
    /// 参数(每参数一条;空格分隔单行会吞含空格的参数)
    pub form_args: Vec<Entity<InputState>>,
    /// 单次调用超时 MS(空 = 默认 60000)
    pub form_timeout: Option<Entity<InputState>>,
    /// 动态环境变量键值对列表
    pub form_env: Vec<(Entity<InputState>, Entity<InputState>)>,
    /// 传输形态(true = streamable-http,false = stdio)
    pub form_http: bool,
    /// http endpoint URL
    pub form_url: Option<Entity<InputState>>,
    /// http 附加请求头键值对列表(原样透传,如 Authorization)
    pub form_headers: Vec<(Entity<InputState>, Entity<InputState>)>,
    /// JSON 粘贴区输入
    pub json_input: Option<Entity<EditorState>>,
    /// JSON 页签预填文本(编辑模式 = 当前配置;新建 = None 显示占位示例)
    pub json_draft: Option<String>,
    /// JSON 实时解析预览(None = 未解析;Err = 错误文案)
    pub json_preview: Option<Result<Vec<liuma_core::settings::McpServerEntry>, String>>,
}

/// 详情页页签
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpDetailMode {
    /// 表单(逐字段)
    Form,
    /// JSON 粘贴导入
    Json,
}

impl AppStore {
    /// 打开详情页(新增模式:id 可输入;JSON 页签不激活)
    pub fn open_mcp_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let form_id = Some(cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("settings.mcp_name_placeholder"))
        }));
        let form_command = Some(cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("settings.mcp_command_placeholder"))
        }));
        let form_cwd =
            Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder(t!("settings.mcp_cwd_placeholder"))
            }));
        let form_timeout = Some(cx.new(|cx| InputState::new(window, cx).placeholder("60000")));
        self.settings.mcp_detail = Some(McpDetailState {
            editing: None,
            mode: McpDetailMode::Form,
            form_enabled: true,
            form_id,
            form_command,
            form_cwd,
            form_args: Vec::new(),
            form_timeout,
            form_env: Vec::new(),
            form_http: false,
            form_url: Some(
                cx.new(|cx| InputState::new(window, cx).placeholder("https://host/mcp")),
            ),
            form_headers: Vec::new(),
            json_input: None,
            json_draft: None,
            json_preview: None,
        });
        cx.notify();
    }

    /// 打开详情页(编辑模式:按 id 从快照预填;id 锁定只读)
    pub fn open_mcp_edit(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.settings.settings_snapshot["mcpServers"]
            .as_array()
            .and_then(|list| list.iter().find(|e| e["id"] == *id).cloned())
            .and_then(|v| serde_json::from_value::<liuma_core::settings::McpServerEntry>(v).ok())
        else {
            return;
        };
        let form_id = cx
            .new(|cx| InputState::new(window, cx).placeholder(t!("settings.mcp_name_placeholder")));
        let form_command = cx.new(|cx| InputState::new(window, cx));
        form_command.update(cx, |s, cx| {
            s.set_value(entry.command.clone(), window, cx);
        });
        let form_cwd = cx
            .new(|cx| InputState::new(window, cx).placeholder(t!("settings.mcp_cwd_placeholder")));
        if let Some(cwd) = &entry.cwd {
            form_cwd.update(cx, |s, cx| {
                s.set_value(cwd.clone(), window, cx);
            });
        }
        let mut form_args = Vec::new();
        for a in &entry.args {
            let input = cx.new(|cx| InputState::new(window, cx));
            input.update(cx, |s, cx| s.set_value(a.clone(), window, cx));
            form_args.push(input);
        }
        let form_timeout = cx.new(|cx| InputState::new(window, cx).placeholder("60000"));
        if let Some(ms) = entry.tool_call_timeout_ms {
            form_timeout.update(cx, |s, cx| {
                s.set_value(ms.to_string(), window, cx);
            });
        }
        let mut form_env = Vec::new();
        for (k, v) in &entry.env {
            let k_in = cx.new(|cx| InputState::new(window, cx));
            k_in.update(cx, |s, cx| s.set_value(k.clone(), window, cx));
            let v_in = cx.new(|cx| InputState::new(window, cx));
            v_in.update(cx, |s, cx| s.set_value(v.clone(), window, cx));
            form_env.push((k_in, v_in));
        }
        let form_http = entry.is_http();
        let form_url = cx.new(|cx| InputState::new(window, cx).placeholder("https://host/mcp"));
        if let Some(url) = &entry.url {
            form_url.update(cx, |s, cx| {
                s.set_value(url.clone(), window, cx);
            });
        }
        let mut form_headers = Vec::new();
        for (k, v) in &entry.headers {
            let k_in = cx.new(|cx| InputState::new(window, cx));
            k_in.update(cx, |s, cx| s.set_value(k.clone(), window, cx));
            let v_in = cx.new(|cx| InputState::new(window, cx));
            v_in.update(cx, |s, cx| s.set_value(v.clone(), window, cx));
            form_headers.push((k_in, v_in));
        }
        // JSON 页签预填:当前配置回显为 mcpServers 形态(去 id/enabled——
        // 编辑态身份在标题锁定,启停在表单)
        let mut body = serde_json::to_value(&entry).unwrap_or(serde_json::json!({}));
        if let Some(map) = body.as_object_mut() {
            map.remove("id");
            map.remove("enabled");
        }
        let json_draft = serde_json::to_string_pretty(&serde_json::json!({
            "mcpServers": { entry.id.clone(): body }
        }))
        .ok();
        self.settings.mcp_detail = Some(McpDetailState {
            editing: Some(id.to_string()),
            mode: McpDetailMode::Form,
            form_enabled: entry.enabled,
            form_id: Some(form_id),
            form_command: Some(form_command),
            form_cwd: Some(form_cwd),
            form_args,
            form_timeout: Some(form_timeout),
            form_env,
            form_http,
            form_url: Some(form_url),
            form_headers,
            json_input: None,
            json_draft,
            json_preview: None,
        });
        cx.notify();
    }

    /// 返回列表页(弃草稿)
    pub fn close_mcp_detail(&mut self, cx: &mut Context<Self>) {
        self.settings.mcp_detail = None;
        cx.notify();
    }

    /// 添加一条参数输入
    pub fn add_mcp_arg(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self.settings.mcp_detail.as_mut() else {
            return;
        };
        detail
            .form_args
            .push(cx.new(|cx| InputState::new(window, cx)));
        cx.notify();
    }

    /// 移除一条参数输入
    pub fn remove_mcp_arg(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(detail) = self.settings.mcp_detail.as_mut() else {
            return;
        };
        if ix < detail.form_args.len() {
            detail.form_args.remove(ix);
            cx.notify();
        }
    }

    /// 添加一组环境变量键值输入
    pub fn add_mcp_env(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self.settings.mcp_detail.as_mut() else {
            return;
        };
        let k = cx.new(|cx| InputState::new(window, cx).placeholder("KEY"));
        let v = cx.new(|cx| InputState::new(window, cx).placeholder("VALUE"));
        detail.form_env.push((k, v));
        cx.notify();
    }

    /// 移除一组环境变量键值输入
    pub fn remove_mcp_env(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(detail) = self.settings.mcp_detail.as_mut() else {
            return;
        };
        if ix < detail.form_env.len() {
            detail.form_env.remove(ix);
            cx.notify();
        }
    }

    /// 切换传输形态(stdio ↔ streamable-http;字段集随形态显隐)
    pub fn toggle_mcp_transport(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.settings.mcp_detail.as_mut() else {
            return;
        };
        detail.form_http = !detail.form_http;
        cx.notify();
    }

    /// 添加一组 http 请求头键值输入
    pub fn add_mcp_header(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self.settings.mcp_detail.as_mut() else {
            return;
        };
        let k = cx.new(|cx| InputState::new(window, cx).placeholder(t!("settings.mcp_header_key")));
        let v = cx.new(|cx| InputState::new(window, cx).placeholder("VALUE"));
        detail.form_headers.push((k, v));
        cx.notify();
    }

    /// 移除一组 http 请求头键值输入
    pub fn remove_mcp_header(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(detail) = self.settings.mcp_detail.as_mut() else {
            return;
        };
        if ix < detail.form_headers.len() {
            detail.form_headers.remove(ix);
            cx.notify();
        }
    }

    /// 翻转详情页启用开关
    pub fn toggle_mcp_form_enabled(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.settings.mcp_detail.as_mut() else {
            return;
        };
        detail.form_enabled = !detail.form_enabled;
        cx.notify();
    }

    /// 切换详情页页签(表单 ↔ JSON;切到 JSON 时懒建粘贴区)
    pub fn switch_mcp_mode(
        &mut self,
        mode: McpDetailMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = self.settings.mcp_detail.as_mut() else {
            return;
        };
        detail.mode = mode;
        if mode == McpDetailMode::Json && detail.json_input.is_none() {
            // 文档标准用法:创建时 default_value 预填(编辑模式 = 当前
            // 配置);新增模式无草稿 → placeholder 显示示例
            let draft = detail.json_draft.clone();
            let input = cx.new(|cx| {
                let state = EditorState::new(window, cx).language("json");
                match draft {
                    Some(text) => state.default_value(text),
                    None => state.placeholder(
                        "{\n  \"mcpServers\": {\n    \"filesystem\": {\n      \"command\": \"npx\",\n      \"args\": [\"-y\", \"@modelcontextprotocol/server-filesystem\", \"~/dir\"]\n    },\n    \"remote\": {\n      \"transport\": \"http\",\n      \"url\": \"https://host/mcp\",\n      \"headers\": { \"Authorization\": \"Bearer <token>\" }\n    }\n  }\n}",
                    ),
                }
            });
            cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    let text = input.read(cx).value().to_string();
                    let preview = liuma_core::settings::parse_mcp_servers_json(&text);
                    let Some(d) = this.settings.mcp_detail.as_mut() else {
                        return;
                    };
                    d.json_preview = Some(preview);
                }
                cx.notify();
            })
            .detach();
            detail.json_input = Some(input);
        }
        if mode == McpDetailMode::Form {
            detail.json_input = None;
            detail.json_preview = None;
        }
        cx.notify();
    }

    /// 导入 JSON(逐条 upsert;任一非法整体拒绝,错误进页内通告)
    pub fn import_mcp_json(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.settings.mcp_detail.as_ref() else {
            return;
        };
        let Some(input) = &detail.json_input else {
            return;
        };
        let text = input.read(cx).value().to_string();
        match self.bridge.host().import_mcp_servers_json(&text) {
            Ok(n) => {
                self.settings.mcp_detail = None;
                self.settings.settings_notice =
                    Some((true, t!("settings.mcp_imported", n = n).into_owned()));
                self.settings_refresh(cx);
            }
            Err(e) => {
                self.settings.settings_notice = Some((false, e.message));
                cx.notify();
            }
        }
    }

    /// 统一保存:表单态提交字段,JSON 态解析导入(同一个保存动作)
    pub fn save_mcp_detail(&mut self, cx: &mut Context<Self>) {
        let mode = self
            .settings
            .mcp_detail
            .as_ref()
            .map(|d| d.mode)
            .unwrap_or(McpDetailMode::Form);
        match mode {
            McpDetailMode::Form => self.submit_mcp_server(cx),
            McpDetailMode::Json => self.import_mcp_json(cx),
        }
    }

    /// 编辑态卸载:删条目并返回列表
    pub fn uninstall_mcp_detail(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self
            .settings
            .mcp_detail
            .as_ref()
            .and_then(|d| d.editing.clone())
        else {
            return;
        };
        if self.bridge.host().remove_mcp_server(&id).is_ok() {
            self.settings.mcp_detail = None;
            self.settings_refresh(cx);
        }
        cx.notify();
    }

    /// 提交 MCP server(新增 = 新 id upsert;编辑 = 同 id 覆盖)。
    /// http 形态:url 必填 + headers 键值对(键空跳过);stdio 形态:
    /// command 必填 + args/env;超时两态共用(空 = 60000)
    pub fn submit_mcp_server(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.settings.mcp_detail.as_ref() else {
            return;
        };
        let id = match &detail.editing {
            Some(id) => id.clone(),
            None => detail
                .form_id
                .as_ref()
                .map(|i| i.read(cx).value().trim().to_string())
                .unwrap_or_default(),
        };
        if id.is_empty() {
            self.push_mcp_form_notice(t!("settings.mcp_id_empty"), cx);
            return;
        }
        let timeout = match detail
            .form_timeout
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
        {
            Some(t) if t.is_empty() => None,
            Some(t) => match t.parse::<u64>() {
                Ok(ms) => Some(ms),
                Err(_) => {
                    self.push_mcp_form_notice(t!("settings.mcp_timeout_invalid"), cx);
                    return;
                }
            },
            None => None,
        };
        let (command, args, env, cwd, url, headers) = if detail.form_http {
            let url = detail
                .form_url
                .as_ref()
                .map(|i| i.read(cx).value().trim().to_string())
                .unwrap_or_default();
            if url.is_empty() {
                self.push_mcp_form_notice(t!("settings.mcp_need_url"), cx);
                return;
            }
            let mut header_map = std::collections::BTreeMap::new();
            for (k, v) in &detail.form_headers {
                let key = k.read(cx).value().trim().to_string();
                if key.is_empty() {
                    continue;
                }
                header_map.insert(key, v.read(cx).value().to_string());
            }
            (
                String::new(),
                Vec::new(),
                std::collections::BTreeMap::new(),
                None,
                Some(url),
                header_map,
            )
        } else {
            let Some(cmd_in) = &detail.form_command else {
                return;
            };
            let command = cmd_in.read(cx).value().trim().to_string();
            if command.is_empty() {
                self.push_mcp_form_notice(t!("settings.mcp_need_command"), cx);
                return;
            }
            let args: Vec<String> = detail
                .form_args
                .iter()
                .map(|i| i.read(cx).value().trim().to_string())
                .filter(|v| !v.is_empty())
                .collect();
            let mut env = std::collections::BTreeMap::new();
            for (k, v) in &detail.form_env {
                let key = k.read(cx).value().trim().to_string();
                if key.is_empty() {
                    continue;
                }
                env.insert(key, v.read(cx).value().to_string());
            }
            let cwd = detail
                .form_cwd
                .as_ref()
                .map(|i| i.read(cx).value().trim().to_string())
                .filter(|c| !c.is_empty());
            (
                command,
                args,
                env,
                cwd,
                None,
                std::collections::BTreeMap::new(),
            )
        };
        let entry = liuma_core::settings::McpServerEntry {
            id,
            enabled: detail.form_enabled,
            command,
            args,
            env,
            cwd,
            tool_call_timeout_ms: timeout,
            url,
            headers,
        };
        match self.bridge.host().upsert_mcp_server(entry) {
            Ok(()) => {
                self.settings.mcp_detail = None;
                self.settings_refresh(cx);
            }
            Err(e) => {
                self.settings.settings_notice = Some((false, e.message));
                cx.notify();
            }
        }
    }

    pub(crate) fn push_mcp_form_notice(
        &mut self,
        msg: impl Into<gpui_kit::SharedString>,
        cx: &mut Context<Self>,
    ) {
        let msg: gpui_kit::SharedString = msg.into();
        if let Some(detail) = self.settings.mcp_detail.as_mut() {
            detail.json_preview = Some(Err(msg.to_string()));
        }
        cx.notify();
    }

    /// 启停 MCP server(enabled 翻转,upsert 落盘)
    pub fn toggle_mcp_server(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(mut entry) = self.settings.settings_snapshot["mcpServers"]
            .as_array()
            .and_then(|list| list.iter().find(|e| e["id"] == *id).cloned())
            .and_then(|v| serde_json::from_value::<liuma_core::settings::McpServerEntry>(v).ok())
        else {
            return;
        };
        entry.enabled = !entry.enabled;
        if self.bridge.host().upsert_mcp_server(entry).is_ok() {
            self.settings_refresh(cx);
        }
        cx.notify();
    }

    /// 卸载 MCP server(删除注册条目;端口池同步停机,工具面即时收敛)
    pub fn remove_mcp_server(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.bridge.host().remove_mcp_server(id).is_ok() {
            self.settings_refresh(cx);
        }
        cx.notify();
    }
}

pub(crate) fn mcp_section(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let st_add = store.clone();
    let Some(detail) = st.settings.mcp_detail.clone() else {
        // ── 列表页:行卡(id / command / 启停 Switch / 编辑 / 卸载)+ 添加钮 ──
        let servers = st.settings.settings_snapshot["mcpServers"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut col = div()
            .v_flex()
            .gap(px(12.))
            .child(section_title(t!("settings.mcp_servers_title")))
            .children(st.settings.settings_notice.as_ref().map(|(ok, msg)| {
                div()
                    .debug_selector(|| "mcp-settings-notice".to_string())
                    .text_size(px(12.))
                    .text_color(if *ok {
                        theme::SUCCESS()
                    } else {
                        theme::DANGER()
                    })
                    .child(format!("{} {msg}", if *ok { "✓" } else { "⚠" }))
            }))
            .child(intro_line(t!("settings.mcp_intro")));
        // 列表头:「已安装 N」+ 新建主钮
        let total = servers.len();
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
                        .text_color(theme::CAPTION())
                        .child(t!("settings.installed", total = total)),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("mcp-add")
                        .debug_selector(|| "mcp-add".to_string())
                        .flex()
                        .h(px(28.))
                        .items_center()
                        .px(px(12.))
                        .rounded(px(8.))
                        .bg(theme::LABEL())
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(theme::INK())
                        .hover(|s| s.opacity(0.9))
                        .on_click({
                            let st_open = st_add.clone();
                            move |_, window, cx| {
                                st_open.update(cx, |st, cx| st.open_mcp_add(window, cx));
                            }
                        })
                        .child(t!("settings.add_new")),
                ),
        );

        let mut rows = div().v_flex().gap(px(8.));
        if servers.is_empty() {
            rows = rows.child(caption_line(t!("settings.mcp_none")));
        }
        for (ix, s) in servers.into_iter().enumerate() {
            let id = s["id"].as_str().unwrap_or_default().to_string();
            let command = s["command"].as_str().unwrap_or_default().to_string();
            let args: Vec<String> = s["args"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let enabled = s["enabled"].as_bool().unwrap_or(false);
            let status = st
                .settings
                .mcp_status_by_id
                .get(&id)
                .cloned()
                .or_else(|| {
                    // 帧未达时的兜底:设置快照里的宿主已知状态
                    st.settings.settings_snapshot["mcpStatus"]["servers"]
                        .as_array()?
                        .iter()
                        .find(|s| s["id"].as_str() == Some(id.as_str()))
                        .map(|s| {
                            (
                                s["status"].as_str().unwrap_or_default().to_string(),
                                s["error"].as_str().unwrap_or_default().to_string(),
                            )
                        })
                })
                .unwrap_or_else(|| ("stopped".into(), String::new()));
            let (st_switch, st_edit, st_remove) = (store.clone(), store.clone(), store.clone());
            let (id_switch_click, id_edit_click, id_remove_click) =
                (id.clone(), id.clone(), id.clone());
            let (id_sw_dbg, id_sw_click) = (id_switch_click.clone(), id_switch_click.clone());
            let (id_ed_dbg, id_ed_click) = (id_edit_click.clone(), id_edit_click.clone());
            let (id_rm_dbg, id_rm_click) = (id_remove_click.clone(), id_remove_click.clone());
            let (dot_color, status_text) = match status.0.as_str() {
                "ready" => (theme::SUCCESS(), String::new()),
                "connecting" => (
                    theme::LABEL_2(),
                    t!("settings.status_connecting").to_string(),
                ),
                "reconnecting" => (
                    theme::LABEL_2(),
                    t!("settings.status_reconnecting").to_string(),
                ),
                "failed" => (theme::DANGER(), t!("settings.status_failed").to_string()),
                _ => (theme::CAPTION(), String::new()),
            };
            // 摘要随传输形态:url 在场 = http(host),否则 stdio(命令+参数)
            let is_http = s["url"].as_str().is_some_and(|u| !u.is_empty());
            let summary = if is_http {
                let url = s["url"].as_str().unwrap_or_default();
                let host = url
                    .split("://")
                    .nth(1)
                    .unwrap_or(url)
                    .split('/')
                    .next()
                    .unwrap_or(url);
                format!("http · {host}")
            } else {
                let mut sm = format!("stdio · {command}");
                if !args.is_empty() {
                    sm.push(' ');
                    sm.push_str(&args.join(" "));
                }
                sm
            };
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(10.))
                    .bg(theme::LAYER())
                    .px(px(12.))
                    .py(px(10.))
                    // 左列:名称行(状态点 + id)/ 摘要行
                    .child(
                        div()
                            .v_flex()
                            .gap(px(2.))
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(div().size(px(7.)).rounded_full().bg(dot_color))
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                                            .text_color(theme::LABEL())
                                            .child(id_switch_click.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .min_w(px(0.))
                                            .text_size(px(11.))
                                            .text_color(theme::CAPTION())
                                            .child(summary),
                                    )
                                    .children((!status_text.is_empty()).then(|| {
                                        div()
                                            .text_size(px(11.))
                                            .text_color(status_color_of(&status))
                                            .child(status_text)
                                    })),
                            ),
                    )
                    .child(
                        // 库 Switch(组件库迁移):on_click 收 &bool,点击/
                        // 键盘语义由库托管(此前 on_mouse_down 拖走也触发)
                        div()
                            .id(("mcp-switch", ix))
                            .debug_selector(move || format!("mcp-switch-{id_sw_dbg}"))
                            .child(
                                Switch::new(("mcp-switch-toggle", ix))
                                    .checked(enabled)
                                    .color(theme::LABEL())
                                    .on_click({
                                        let st_switch = st_switch.clone();
                                        let id_sw = id_sw_click.clone();
                                        move |_, _, cx| {
                                            st_switch.update(cx, |st, cx| {
                                                st.toggle_mcp_server(&id_sw, cx)
                                            });
                                        }
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .id(("mcp-edit", ix))
                            .debug_selector(move || format!("mcp-edit-{id_ed_dbg}"))
                            .flex()
                            .h(px(22.))
                            .items_center()
                            .px(px(8.))
                            .rounded(px(11.))
                            .cursor_pointer()
                            .text_size(px(11.))
                            .text_color(theme::LABEL_2())
                            .hover(|s| s.bg(theme::DOCK()))
                            .on_click(move |_, window, cx| {
                                st_edit.update(cx, |st, cx| {
                                    st.open_mcp_edit(&id_ed_click, window, cx)
                                });
                            })
                            .child(t!("common.edit")),
                    )
                    .child(
                        div()
                            .id(("mcp-remove", ix))
                            .debug_selector(move || format!("mcp-remove-{id_rm_dbg}"))
                            .flex()
                            .h(px(22.))
                            .items_center()
                            .px(px(8.))
                            .rounded(px(11.))
                            .cursor_pointer()
                            .text_size(px(11.))
                            .text_color(theme::DANGER())
                            .hover(|s| s.bg(theme::DOCK()))
                            .on_click(move |_, _, cx| {
                                st_remove
                                    .update(cx, |st, cx| st.remove_mcp_server(&id_rm_click, cx));
                            })
                            .child(t!("common.uninstall")),
                    ),
            );
        }
        col = col.child(rows);
        return col.into_any_element();
    };

    // ── 详情页(新增/编辑;页签只切换编辑形态,保存是同一个动作)──
    let (title, intro) = match &detail.editing {
        Some(id) => (
            t!("settings.mcp_edit_card", id = id).into_owned(),
            t!("settings.mcp_edit_desc").to_string(),
        ),
        None => (
            t!("settings.mcp_new_card").to_string(),
            t!("settings.mcp_new_desc").to_string(),
        ),
    };
    let json_open = detail.mode == McpDetailMode::Json;
    let (st_close, st_save, _st_env_toggle, _st_env_add, st_uninstall) = (
        store.clone(),
        store.clone(),
        store.clone(),
        store.clone(),
        store.clone(),
    );
    let st_form = store.clone();
    let st_json = store.clone();
    let st_cancel = st_close.clone();

    // 标题 + 说明 + 页签(右上,一体胶囊组)
    let mut head = div()
        .flex()
        .items_start()
        .gap(px(12.))
        .child(
            div()
                .v_flex()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(theme::LABEL())
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme::CAPTION())
                        .child(intro),
                ),
        )
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .rounded(px(10.))
                .bg(theme::LAYER())
                .p(px(2.))
                .child(
                    div()
                        .id("mcp-tab-form")
                        .debug_selector(|| "mcp-tab-form".to_string())
                        .flex()
                        .h(px(22.))
                        .items_center()
                        .px(px(10.))
                        .rounded(px(8.))
                        .cursor_pointer()
                        .text_size(px(11.))
                        .text_color(if json_open {
                            theme::CAPTION().into()
                        } else {
                            gpui_kit::white()
                        })
                        .bg(if json_open {
                            theme::LAYER()
                        } else {
                            theme::BRAND()
                        })
                        .on_click(move |_, window, cx| {
                            st_form.update(cx, |st, cx| {
                                if st.settings.mcp_detail.as_ref().map(|d| d.mode)
                                    != Some(McpDetailMode::Form)
                                {
                                    st.switch_mcp_mode(McpDetailMode::Form, window, cx);
                                }
                            });
                        })
                        .child(t!("settings.form_section")),
                )
                .child(
                    div()
                        .id("mcp-tab-json")
                        .debug_selector(|| "mcp-tab-json".to_string())
                        .flex()
                        .h(px(22.))
                        .items_center()
                        .px(px(10.))
                        .rounded(px(8.))
                        .cursor_pointer()
                        .text_size(px(11.))
                        .text_color(if json_open {
                            gpui_kit::white()
                        } else {
                            theme::CAPTION().into()
                        })
                        .bg(if json_open {
                            theme::BRAND()
                        } else {
                            theme::LAYER()
                        })
                        .on_click(move |_, window, cx| {
                            st_json.update(cx, |st, cx| {
                                if st.settings.mcp_detail.as_ref().map(|d| d.mode)
                                    != Some(McpDetailMode::Json)
                                {
                                    st.switch_mcp_mode(McpDetailMode::Json, window, cx);
                                }
                            });
                        })
                        .child("JSON"),
                ),
        );
    // ✕(返回列表)
    head = head.child(
        div()
            .id("mcp-detail-close")
            .size(px(24.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(theme::CAPTION())
            .hover(|s| s.bg(theme::DOCK()))
            .on_click(move |_, _, cx| {
                st_close.update(cx, |st, cx| st.close_mcp_detail(cx));
            })
            .child(fixed(IconName::Close, 12.)),
    );

    // 详情卡(中性边框;蓝 BRAND 只给主钮)
    let mut card = div()
        .id("mcp-add-card")
        .debug_selector(|| "mcp-add-card".to_string())
        .v_flex()
        .gap(px(14.))
        .rounded(px(10.))
        .border_1()
        .border_color(theme::BORDER())
        .bg(theme::LAYER())
        .p(px(14.))
        .child(head);

    // 页签内容
    if json_open {
        if let Some(input) = &detail.json_input {
            card = card.child(
                div()
                    .v_flex()
                    .gap(px(6.))
                    .child(caption_line(t!("settings.full_config")))
                    .child(
                        div()
                            .id("mcp-json-input")
                            .debug_selector(|| "mcp-json-input".to_string())
                            .w_full()
                            .min_w(px(0.))
                            .child(gpui_kit::component::input::Editor::new(input).h(px(280.))),
                    ),
            );
        }
        if let Some(Err(e)) = &detail.json_preview {
            card = card.child(
                div()
                    .text_size(px(11.))
                    .text_color(theme::DANGER())
                    .child(format!("⚠ {e}")),
            );
        }
        if let Some(Ok(entries)) = &detail.json_preview {
            let mut list = div().v_flex().gap(px(4.));
            for e in entries {
                list = list.child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme::LABEL_2())
                        .child(format!("{} · {}", e.id, e.command)),
                );
            }
            card = card.child(list);
        }
    } else {
        // 通用字段行(标签 64px 左置 + 输入 flex_1)
        let id_field: gpui_kit::AnyElement = if let Some(id) = &detail.editing {
            div()
                .id("mcp-id-input")
                .debug_selector(|| "mcp-id-input".to_string())
                .flex_1()
                .min_w(px(0.))
                .h(px(32.))
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(theme::CAPTION())
                .child(id.clone())
                .into_any_element()
        } else {
            div()
                .id("mcp-id-input")
                .debug_selector(|| "mcp-id-input".to_string())
                .flex_1()
                .min_w(px(0.))
                .h(px(32.))
                .children(
                    detail
                        .form_id
                        .as_ref()
                        .map(|e| div().w_full().child(Input::new(e))),
                )
                .into_any_element()
        };
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .w(px(64.))
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(theme::CAPTION())
                        .child(t!("settings.name")),
                )
                .child(id_field),
        );
        // 传输形态(stdio / HTTP):字段集随形态显隐
        let form_http = detail.form_http;
        let st_transport = store.clone();
        let transport_chip = |sel: &'static str,
                              label: &'static str,
                              active: bool,
                              want: bool,
                              st: Entity<AppStore>| {
            let base = div()
                .id(sel)
                .flex()
                .h(px(26.))
                .items_center()
                .px(px(10.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_size(px(12.));
            let base = if active {
                base.bg(theme::ONGOING().opacity(0.14))
                    .text_color(theme::LABEL_2())
            } else {
                base.text_color(theme::CAPTION())
                    .hover(|s| s.bg(theme::DOCK()).text_color(theme::LABEL_2()))
            };
            base.child(label)
                .on_click(move |_, _, cx| {
                    // 绝对值方向:点击目标形态与当前不同才切换(连发幂等)
                    let active_now = st
                        .read(cx)
                        .settings
                        .mcp_detail
                        .as_ref()
                        .map(|d| d.form_http)
                        .unwrap_or(false);
                    if active_now != want {
                        st.update(cx, |st, cx| st.toggle_mcp_transport(cx));
                    }
                })
                .into_any_element()
        };
        let transport_row = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(transport_chip(
                "mcp-transport-stdio",
                "stdio",
                !form_http,
                false,
                st_transport.clone(),
            ))
            .child(transport_chip(
                "mcp-transport-http",
                "HTTP",
                form_http,
                true,
                st_transport,
            ));
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .w(px(64.))
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(theme::CAPTION())
                        .child(t!("settings.transport")),
                )
                .child(transport_row),
        );
        if detail.form_http {
            card = card.child(field_input("URL", "mcp-url-input", &detail.form_url));
            // 请求头(键值对;原样透传,如 Authorization)
            let mut header_rows = div().v_flex().gap(px(4.));
            for (ix, (k, v)) in detail.form_headers.iter().enumerate() {
                let st_rm = store.clone();
                header_rows = header_rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(div().w(px(120.)).flex_shrink_0().child(Input::new(k)))
                        .child(div().flex_1().min_w(px(0.)).child(Input::new(v)))
                        .child(
                            div()
                                .id(("mcp-header-rm", ix))
                                .flex()
                                .h(px(22.))
                                .items_center()
                                .px(px(6.))
                                .rounded(px(6.))
                                .cursor_pointer()
                                .text_size(px(11.))
                                .text_color(theme::CAPTION())
                                .hover(|s| s.bg(theme::DOCK()).text_color(theme::DANGER()))
                                .on_click({
                                    let st_rm = st_rm.clone();
                                    move |_, _, cx| {
                                        st_rm.update(cx, |st, cx| st.remove_mcp_header(ix, cx));
                                    }
                                })
                                .child(t!("common.remove")),
                        ),
                );
            }
            let st_header_add = store.clone();
            card = card.child(
                div()
                    .v_flex()
                    .gap(px(4.))
                    .child(caption_line(t!("settings.headers_desc")))
                    .child(header_rows)
                    .child(
                        div()
                            .id("mcp-header-add")
                            .flex()
                            .h(px(24.))
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_size(px(11.))
                            .text_color(theme::CAPTION())
                            .hover(|s| s.bg(theme::DOCK()).text_color(theme::LABEL_2()))
                            .on_click({
                                let st_header_add = st_header_add.clone();
                                move |_, window, cx| {
                                    st_header_add
                                        .update(cx, |st, cx| st.add_mcp_header(window, cx));
                                }
                            })
                            .child(t!("settings.add_header")),
                    ),
            );
        } else {
            card = card.child(field_input(
                "command",
                "mcp-command-input",
                &detail.form_command,
            ));
            card = card.child(field_input("cwd", "mcp-cwd-input", &detail.form_cwd));
            // 参数(每参数一条;含空格的参数单行化会吞内容)
            let mut arg_rows = div().v_flex().gap(px(4.));
            for (ix, arg) in detail.form_args.iter().enumerate() {
                let st_rm = store.clone();
                arg_rows = arg_rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(div().flex_1().min_w(px(0.)).child(Input::new(arg)))
                        .child(
                            div()
                                .id(("mcp-arg-rm", ix))
                                .flex()
                                .h(px(22.))
                                .items_center()
                                .px(px(6.))
                                .rounded(px(6.))
                                .cursor_pointer()
                                .text_size(px(11.))
                                .text_color(theme::CAPTION())
                                .hover(|s| s.bg(theme::DOCK()).text_color(theme::DANGER()))
                                .on_click({
                                    let st_rm = st_rm.clone();
                                    move |_, _, cx| {
                                        st_rm.update(cx, |st, cx| st.remove_mcp_arg(ix, cx));
                                    }
                                })
                                .child(t!("common.remove")),
                        ),
                );
            }
            let st_arg_add = store.clone();
            card = card.child(
                div()
                    .v_flex()
                    .gap(px(4.))
                    .child(caption_line(t!("settings.args_desc")))
                    .child(arg_rows)
                    .child(
                        div()
                            .id("mcp-arg-add")
                            .flex()
                            .h(px(24.))
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_size(px(11.))
                            .text_color(theme::CAPTION())
                            .hover(|s| s.bg(theme::DOCK()).text_color(theme::LABEL_2()))
                            .on_click({
                                let st_arg_add = st_arg_add.clone();
                                move |_, window, cx| {
                                    st_arg_add.update(cx, |st, cx| st.add_mcp_arg(window, cx));
                                }
                            })
                            .child(t!("settings.add_arg")),
                    ),
            );
            // 环境变量(键值对,平铺)
            let mut env_rows = div().v_flex().gap(px(4.));
            for (ix, (k, v)) in detail.form_env.iter().enumerate() {
                let st_rm = store.clone();
                env_rows = env_rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(div().w(px(120.)).flex_shrink_0().child(Input::new(k)))
                        .child(div().flex_1().min_w(px(0.)).child(Input::new(v)))
                        .child(
                            div()
                                .id(("mcp-env-rm", ix))
                                .flex()
                                .h(px(22.))
                                .items_center()
                                .px(px(6.))
                                .rounded(px(6.))
                                .cursor_pointer()
                                .text_size(px(11.))
                                .text_color(theme::CAPTION())
                                .hover(|s| s.bg(theme::DOCK()).text_color(theme::DANGER()))
                                .on_click({
                                    let st_rm = st_rm.clone();
                                    move |_, _, cx| {
                                        st_rm.update(cx, |st, cx| st.remove_mcp_env(ix, cx));
                                    }
                                })
                                .child(t!("common.remove")),
                        ),
                );
            }
            let st_env_add = store.clone();
            card = card.child(
                div()
                    .v_flex()
                    .gap(px(4.))
                    .child(caption_line(t!("settings.env_desc")))
                    .child(env_rows)
                    .child(
                        div()
                            .id("mcp-env-add")
                            .flex()
                            .h(px(24.))
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_size(px(11.))
                            .text_color(theme::CAPTION())
                            .hover(|s| s.bg(theme::DOCK()).text_color(theme::LABEL_2()))
                            .on_click({
                                let st_env_add = st_env_add.clone();
                                move |_, window, cx| {
                                    st_env_add.update(cx, |st, cx| st.add_mcp_env(window, cx));
                                }
                            })
                            .child(t!("settings.add_env")),
                    ),
            );
        }
        card = card.child(field_input(
            t!("settings.timeout_ms_mcp"),
            "mcp-timeout-input",
            &detail.form_timeout,
        ));
        // 启用开关(启停由表单随保存落盘)
        let st_enable = store.clone();
        card = card.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(if detail.form_enabled {
                            theme::LABEL_2()
                        } else {
                            theme::CAPTION()
                        })
                        .child(if detail.form_enabled {
                            t!("settings.enabled")
                        } else {
                            t!("settings.disabled")
                        }),
                )
                .child(
                    Switch::new("mcp-enabled")
                        .small()
                        .checked(detail.form_enabled)
                        .color(theme::LABEL())
                        .on_click({
                            let st_enable = st_enable.clone();
                            move |_, _, cx| {
                                st_enable.update(cx, |st, cx| {
                                    st.toggle_mcp_form_enabled(cx);
                                });
                            }
                        }),
                ),
        );
    }
    // 底部按钮组:左 = 卸载(编辑态);右 = 保存(统一动作)+ 取消
    let mut footer = div().flex().items_center().gap(px(8.));
    if detail.editing.is_some() {
        footer = footer.child(
            div()
                .id("mcp-uninstall")
                .flex()
                .h(px(28.))
                .items_center()
                .gap(px(4.))
                .px(px(8.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(theme::DANGER())
                .hover(|s| s.bg(theme::DOCK()))
                .on_click(move |_, _, cx| {
                    st_uninstall.update(cx, |st, cx| st.uninstall_mcp_detail(cx));
                })
                .child(t!("common.uninstall")),
        );
    }
    footer = footer.child(div().flex_1()).child(
        div()
            .id("mcp-submit")
            .debug_selector(|| "mcp-submit".to_string())
            .flex()
            .h(px(28.))
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .bg(theme::LABEL())
            .px(px(18.))
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(theme::INK())
            .hover(|s| s.opacity(0.9))
            .on_click(move |_, _, cx| {
                st_save.update(cx, |st, cx| st.save_mcp_detail(cx));
            })
            .child(t!("common.save")),
    );
    footer = footer.child(
        div()
            .id("mcp-cancel")
            .flex()
            .h(px(28.))
            .items_center()
            .px(px(10.))
            .rounded(px(8.))
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(theme::LABEL_2())
            .hover(|s| s.bg(theme::DOCK()))
            .on_click(move |_, _, cx| {
                st_cancel.update(cx, |st, cx| st.close_mcp_detail(cx));
            })
            .child(t!("common.cancel")),
    );
    card = card.child(footer);
    card.into_any_element()
}

/// MCP 连接状态着色(ready 绿 / connecting 中性 / failed 红 / 其余灰)
pub(crate) fn status_color_of(status: &(String, String)) -> gpui_kit::Rgba {
    match status.0.as_str() {
        "ready" => theme::SUCCESS(),
        "connecting" => theme::LABEL_2(),
        "failed" => theme::DANGER(),
        _ => theme::CAPTION(),
    }
}
