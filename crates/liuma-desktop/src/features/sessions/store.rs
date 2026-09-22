//! 会话与工作区树的 store 域:侧栏行菜单/重命名/删除确认、会话行
//! CRUD(fork/archive/导出)、工作区 CRUD 与下拉/组头菜单、以及会话
//! 命名/工作区归属镜像查询(title_for/workspace_of/basename)。视图见
//! features::sessions::views;行/组菜单开态本域自持,shell 底座经
//! close_all_menus 统一关闭。

use std::collections::{HashMap, HashSet};

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext, Context, Entity, Window};

use crate::features::chat::ChatNode;
use crate::shell::reducer;
use crate::shell::store::AppStore;
use liuma_core::proto::HistoryValue;

/// 导出保存对话框初始目录:`LIUMA_DOWNLOAD_DIR` 或 ~/Downloads(仅作
/// 对话框起点;最终路径由用户选定,应用不代选)
pub(crate) fn downloads_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("LIUMA_DOWNLOAD_DIR") {
        return std::path::PathBuf::from(d);
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    std::path::PathBuf::from(home).join("Downloads")
}

/// 侧栏列表分组方式(视图选项菜单;内存视图态,不持久化)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum GroupMode {
    /// 按工作区分组(默认;组头 + 组内缩进行)
    #[default]
    Workspace,
    /// 单列表:全部会话平铺,无组头
    Flat,
}

/// 侧栏列表排序方式(视图选项菜单;内存视图态,不持久化)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum OrderMode {
    /// 最近更新(宿主清单序即 updated_at 降序,默认)
    #[default]
    Updated,
    /// 手动排序(拖拽重排;本期仅菜单占位,不可选中)
    Manual,
}

/// 会话与工作区树功能切片状态(侧栏行/组/工作区菜单开态与坐标锚、
/// 重命名目标、工作区路径/标题/分支表、折叠组)。
#[derive(Default)]
pub(crate) struct SessionsStore {
    /// 标题栏会话菜单(⋯)开时的点击坐标(菜单卡根级渲染定位锚;
    /// 菜单作用于当前会话,None = 收起)
    pub session_menu_pos: Option<gpui_kit::Point<gpui_kit::Pixels>>,
    /// 重命名目标会话
    pub rename_target: Option<String>,
    /// 重命名输入态(挂窗后建)
    pub rename_input: Option<Entity<InputState>>,
    /// 标题栏工作区下拉开态
    pub workspace_menu_open: bool,
    /// 工作区名 → 路径(workspace_view 解析)
    pub ws_paths: HashMap<String, std::path::PathBuf>,
    /// 工作区名 → 显示标题(workspace_view 解析;标题仅显示层)
    pub ws_titles: HashMap<String, String>,
    /// 侧栏组头 ⋯ 菜单打开的工作区
    pub menu_open_ws: Option<String>,
    /// 工作区菜单开时的点击坐标(根级渲染定位锚)
    pub ws_menu_pos: Option<gpui_kit::Point<gpui_kit::Pixels>>,
    /// 重命名目标工作区(与会话重命名共用输入态)
    pub rename_ws_target: Option<String>,
    /// 工作区名 → git 分支(None = 非 repo)。模型可经 bash 切分支,
    /// running 期间随统计轮询刷新(非准静态数据)
    pub ws_branches: HashMap<String, Option<String>>,
    /// 侧栏折叠的工作区组(搜索词非空时渲染层忽略)
    pub collapsed_workspaces: HashSet<String>,
    /// 侧栏列表分组方式(顶栏视图选项菜单)
    pub group_mode: GroupMode,
    /// 侧栏列表排序方式(顶栏视图选项菜单)
    pub order_mode: OrderMode,
    /// 视图选项菜单开时的点击坐标(根级渲染定位锚)
    pub view_menu_pos: Option<gpui_kit::Point<gpui_kit::Pixels>>,
}

impl AppStore {
    /// 新会话行同步插入清单(清单刷新已异步化,若等回填,侧栏行晚一拍
    /// ——负载下可见;回填后以宿主清单为权威)。
    pub(crate) fn push_local_session_row(
        &mut self,
        id: String,
        blank: bool,
        parent: Option<String>,
        cwd: Option<String>,
    ) {
        self.state.sessions.push(liuma_core::proto::SessionSummary {
            session_id: id,
            updated_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            running: false,
            blank,
            parent_session_id: parent,
            origin: None,
            cwd,
            agent_preset: None,
            projections: None,
        });
    }

    /// 打开会话:切换 + 历史尾窗加载(后台折叠)+ 统计 + 配置缓存
    pub fn open_session(&mut self, id: &str, cx: &mut Context<Self>) {
        self.state.current_id = Some(id.to_string());
        // 命令行是输入意图,不属于会话状态:切换即弃(否则残留到别的
        // 会话,发送时被拼上 /plan 等——跨会话污染)
        self.chat.pending_command = None;
        // TextView 流式注册表随旧会话焚毁(重开重解析一次;防跨会话
        // key 残留与 map 无界增长);观察者订阅一并退订。轮次锚点索引
        // 与行槽缓存同属会话视图态:不清则上一会话的锚点/行槽在新会话
        // 渲染(refresh_stats 异步回填与下一次 sync 前尤其可见)
        self.chat.tv_streams.clear();
        self.chat.tv_subs.clear();
        // 旧会话晚到落地的重测标记一并焚毁:新列表刚吃 60px uniform
        // hint,残留脏标记会在下一帧触发 0..n 全量重测 = 打开优化破功
        self.chat.tv_remeasure_dirty = false;
        self.chat.nav_anchors_cache = None;
        self.chat.anchor_index.clear();
        self.chat.row_slots.clear();
        self.chat.row_slots_sig = None;
        self.sync_active_workspace_from_current();
        // 配置异步回填:permission fold 大日志,同步跑主线程冻结切换瞬间
        // (见 shell::AppStore::refresh_session_cfg)
        self.refresh_session_cfg(id, cx);
        // 统计异步回填:冷路径全量折叠大日志,同步跑 GPUI 线程会
        // 冻结切换瞬间(见 shell::AppStore::refresh_stats)
        self.refresh_stats(id, cx);
        self.load_history(id.to_string(), cx);
        if self.trajectory_visible() {
            self.refresh_trajectory(cx);
        }
        if self.files_visible() {
            self.files_ensure(cx);
        }
        self.sync_run_tick(cx);
        cx.notify();
    }

    /// 历史全量加载(打开即投影整段会话;渲染层虚拟化,只画可见行)。
    /// 分页方案(每页 fetch 全量翻译日志,比一次性翻译更贵)连同其
    /// 游标/合并重建/挂起跳转机器整体不采用。
    /// 直播帧可能先行到达——以已投影状态为基线回放,节点 key 幂等。
    /// **必须经 [`HostBridge::call`] 上 tokio**:history 内部 attach 会
    /// `tokio::spawn` 每-会话 worker,GPUI 后台线程无 reactor 会 panic。
    fn load_history(&mut self, id: String, cx: &mut Context<Self>) {
        if self.state.chats.contains_key(&id) {
            return; // 已投影
        }
        self.chat.history_loading = true;
        let store = cx.entity().clone();
        let host = self.bridge.host().clone();
        let sid = id.clone();
        // max_messages 取大数 = 不分页,一次带回全部事件
        let rx = self
            .bridge
            .call(async move { host.history(&sid, None, u32::MAX as usize).await });
        cx.spawn(async move |_this, cx| {
            let page = rx.await;
            store.update(cx, |s, cx| {
                s.chat.history_loading = false;
                let Ok(Ok(page)) = page else {
                    // 历史加载失败(最常见:日志 seq 守卫拒载)不再静默——
                    // 聊天区通告带宿主错误原文,故障可见
                    let detail = match &page {
                        Ok(Err(rpc)) => rpc.message.clone(),
                        Err(e) => format!("{e}"),
                        _ => String::new(),
                    };
                    s.push_local_notice(&format!("历史加载失败:{detail}"), cx);
                    return;
                };
                let HistoryValue {
                    events,
                    projections,
                    ..
                } = page;
                let page_events: Vec<liuma_core::proto::SessionEvent> =
                    events.into_iter().map(|e| e.event).collect();
                {
                    let chat = s.state.chats.entry(id.clone()).or_default();
                    chat.merge_history(page_events);
                    // 历史消息含 image 块 → 收集 aid,异步拉取缓存
                    // (与实时帧同路径;避免 chat 可变借用与 s 再借用冲突)
                    let mut aids: Vec<String> = Vec::new();
                    for node in &chat.nodes {
                        if let ChatNode::User { images, .. } = node {
                            for b in images {
                                if let Some(aid) = b["attachment"]["attachmentId"].as_str() {
                                    aids.push(aid.to_string());
                                }
                            }
                        }
                    }
                    let sid = id.clone();
                    for aid in aids {
                        s.ensure_image_loaded(&sid, &aid, cx);
                    }
                };
                if let Some(p) = projections
                    && let Some(t) = p.values.get("title")
                    && let Some(t) = t.as_str().filter(|t| !t.is_empty())
                {
                    s.state.titles.insert(id.clone(), t.to_string());
                }
                cx.notify();
            });
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// 新建会话(目标工作区 = 选中工作区;非默认工作区才传名)。
    /// 新会话行**同步插入**清单——清单刷新已异步化,若等回填,侧栏行
    /// 晚一拍(负载下可见);回填后以宿主清单为权威
    pub fn create_session(&mut self, cx: &mut Context<Self>) {
        let ws = self.non_default_workspace();
        let id = self.bridge.host().create_session(None, None, ws);
        let cwd = self.bridge.host().workspace().display().to_string();
        self.push_local_session_row(id.clone(), true, None, Some(cwd));
        self.refresh_list(cx);
        self.open_session(&id, cx);
    }

    /// 切换工作区:打开其最近会话(优先非空白),无则新建;
    /// 目标组自动展开,分支表顺带刷新(切到的可能是别的 repo)
    pub fn select_workspace(&mut self, ws: &str, cx: &mut Context<Self>) {
        self.state.active_workspace = Some(ws.to_string());
        self.sessions.collapsed_workspaces.remove(ws);
        self.refresh_branches();
        let default = self.default_workspace();
        let sessions: Vec<String> = self
            .state
            .sessions
            .iter()
            .filter(|s| reducer::workspace_of(&s.session_id, &default) == ws)
            .map(|s| s.session_id.clone())
            .collect();
        match sessions.iter().find(|id| !self.is_blank(id)) {
            Some(id) => self.open_session(id, cx),
            None => {
                let target = if ws == default {
                    None
                } else {
                    Some(ws.to_string())
                };
                let id = self.bridge.host().create_session(None, None, target);
                self.push_local_session_row(id.clone(), true, None, None);
                self.refresh_list(cx);
                self.open_session(&id, cx);
            }
        }
    }

    /// 当前会话归属工作区的绝对路径(@file 补全根;源以 header.cwd 为根)。
    /// 会话 id 形如 `wsName/sessionId`(非默认工作区)或裸 id(默认)。
    pub fn current_workspace_dir(&self) -> Option<std::path::PathBuf> {
        let cid = self.state.current_id.clone()?;
        let default = self.default_workspace();
        let ws = crate::shell::reducer::workspace_of(&cid, &default);
        // ws_paths 以工作区名为键;兜底用自身(单工作区场景默认工作区)
        self.sessions
            .ws_paths
            .get(ws)
            .cloned()
            .or_else(|| self.sessions.ws_paths.values().next().cloned())
    }

    /// 刷新工作区表:workspace_view 解析路径 + 读各工作区分支
    pub fn refresh_workspaces(&mut self) {
        let view = self.bridge.host().workspace_view();
        self.sessions.ws_paths.clear();
        self.sessions.ws_titles.clear();
        if let Some(items) = view["items"].as_array() {
            for it in items {
                if let (Some(id), Some(path)) = (
                    it["workspaceId"].as_str(),
                    it["path"].as_str().map(std::path::PathBuf::from),
                ) {
                    self.sessions.ws_paths.insert(id.to_string(), path);
                    if let Some(title) = it["title"].as_str() {
                        self.sessions
                            .ws_titles
                            .insert(id.to_string(), title.to_string());
                    }
                }
            }
        }
        self.refresh_branches();
    }

    /// 只读分支(ws_paths 已知)。HEAD 为本地微秒级单文件读,同步即可
    pub fn refresh_branches(&mut self) {
        let entries: Vec<(String, std::path::PathBuf)> =
            self.sessions.ws_paths.clone().into_iter().collect();
        for (name, path) in entries {
            let branch = crate::gitinfo::branch_of(&path);
            self.sessions.ws_branches.insert(name, branch);
        }
    }

    /// 活动工作区分支(徽标用;未知工作区/非 repo → None)
    pub fn active_branch(&self) -> Option<String> {
        let ws = self
            .state
            .active_workspace
            .clone()
            .unwrap_or_else(|| self.default_workspace());
        self.sessions.ws_branches.get(&ws).cloned().flatten()
    }

    /// 标题栏工作区下拉开关(开时刷新工作区表——分支可能已变)
    pub fn toggle_workspace_menu(&mut self, cx: &mut Context<Self>) {
        self.sessions.workspace_menu_open = !self.sessions.workspace_menu_open;
        if self.sessions.workspace_menu_open {
            self.refresh_workspaces();
        }
        cx.notify();
    }

    /// 侧栏工作区组折叠切换
    pub fn toggle_workspace_collapsed(&mut self, ws: &str, cx: &mut Context<Self>) {
        if !self.sessions.collapsed_workspaces.insert(ws.to_string()) {
            self.sessions.collapsed_workspaces.remove(ws);
        }
        cx.notify();
    }

    /// 在指定工作区新建会话。默认工作区规范化为 None:宿主对任何
    /// 已知工作区名都会加 `<ws>/` 前缀,而默认区会话历史无前缀,
    /// 传 Some(default) 会造出混合 id 形态(registry resolve_session)
    pub fn create_session_in(&mut self, ws: &str, cx: &mut Context<Self>) {
        let target = if ws == self.default_workspace() {
            None
        } else {
            Some(ws.to_string())
        };
        let id = self.bridge.host().create_session(None, None, target);
        self.push_local_session_row(id.clone(), true, None, None);
        self.refresh_list(cx);
        self.open_session(&id, cx);
    }

    /// 系统目录选择器添加工作区(osascript / PowerShell 阻塞模态 → 宿主
    /// runtime spawn_blocking;占普通 worker 会饿死共享 runtime 的会话
    /// worker)。成功 → 刷新 describe/工作区表并切换;取消(bad-request)
    /// 静默;真失败出通知——此前 Err 一律被 `.ok()` 吞掉,「平台不支持」
    /// 与用户取消不可分,点了等于没点。
    pub fn add_workspace_via_picker(&mut self, cx: &mut Context<Self>) {
        let host = self.bridge.host().clone();
        let rx = self.bridge.call(async move {
            // JoinError(spawn_blocking 任务崩溃)在块内折成 internal,
            // 通道里只剩「选择结果」一层
            match tokio::task::spawn_blocking(move || host.pick_workspace_directory()).await {
                Ok(picked) => picked,
                Err(e) => Err(liuma_core::proto::RpcError::internal(format!(
                    "选择器任务失败:{e}"
                ))),
            }
        });
        let store = cx.entity().clone();
        cx.spawn(async move |_this, cx| {
            // 内层 Err = 选择失败/取消;外层 = 通道
            let picked = match rx.await {
                Ok(picked) => picked,
                Err(_) => Err(liuma_core::proto::RpcError::internal("选择器通道失败")),
            };
            match picked {
                Ok(path) => {
                    store.update(cx, |s, cx| {
                        match s.bridge.host().add_workspace(&path) {
                            Ok(ws) => {
                                s.state.host_info = s.bridge.describe();
                                s.refresh_workspaces();
                                s.select_workspace(&ws, cx);
                            }
                            Err(e) => {
                                s.push_local_notice(&format!("添加工作区失败:{}", e.message), cx);
                            }
                        }
                        cx.notify();
                    });
                }
                // 用户取消:预期分支,静默
                Err(e) if e.code == "bad-request" => {}
                Err(e) => {
                    store.update(cx, |s, cx| {
                        s.push_local_notice(&format!("无法打开目录选择:{}", e.message), cx);
                        cx.notify();
                    });
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
        cx.notify();
    }

    /// 组头 ⋯ 菜单开(带坐标;根级渲染定位)
    pub fn open_ws_menu_at(
        &mut self,
        name: &str,
        pos: gpui_kit::Point<gpui_kit::Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.sessions.session_menu_pos = None;
        self.sessions.menu_open_ws = Some(name.to_string());
        self.sessions.ws_menu_pos = Some(pos);
        cx.notify();
    }

    /// 标题栏会话菜单(⋯)开/收(toggle;带坐标,根级渲染定位)。
    /// 菜单作用于当前会话;与其余菜单互斥
    pub fn toggle_session_menu(
        &mut self,
        pos: gpui_kit::Point<gpui_kit::Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.sessions.session_menu_pos = if self.sessions.session_menu_pos.is_some() {
            None
        } else {
            Some(pos)
        };
        cx.notify();
    }

    /// 顶栏视图选项菜单开(带坐标;根级渲染定位,右对齐滑块钮展开)。
    /// 与其余菜单互斥:开时清兄弟菜单开态
    pub fn open_view_menu_at(
        &mut self,
        pos: gpui_kit::Point<gpui_kit::Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.sessions.session_menu_pos = None;
        self.sessions.menu_open_ws = None;
        self.sessions.workspace_menu_open = false;
        self.sessions.view_menu_pos = Some(pos);
        cx.notify();
    }

    /// 切换列表分组方式(菜单项选择即收菜单)
    pub fn set_group_mode(&mut self, mode: GroupMode, cx: &mut Context<Self>) {
        self.sessions.group_mode = mode;
        self.sessions.view_menu_pos = None;
        cx.notify();
    }

    /// 切换列表排序方式(菜单项选择即收菜单)。手动排序本期仅占位:
    /// 拖拽重排未实现,菜单项置灰不触发
    pub fn set_order_mode(&mut self, mode: OrderMode, cx: &mut Context<Self>) {
        if mode == OrderMode::Manual {
            return;
        }
        self.sessions.order_mode = mode;
        self.sessions.view_menu_pos = None;
        cx.notify();
    }

    /// 打开工作区重命名(复用会话重命名输入态)
    pub fn open_rename_workspace(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sessions.rename_input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("工作区标题"));
            cx.subscribe(&input, |this, _i, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    this.confirm_rename(cx);
                }
            })
            .detach();
            self.sessions.rename_input = Some(input);
        }
        let current = self.title_for_workspace(name);
        if let Some(input) = &self.sessions.rename_input {
            input.update(cx, |s, cx| s.set_value(&current, window, cx));
        }
        self.sessions.rename_ws_target = Some(name.to_string());
        self.sessions.menu_open_ws = None;
        cx.notify();
    }

    /// 移除工作区(默认工作区拒绝;当前工作区被移除 → 切回首会话/新建)。
    /// 刷新已异步化:同 archive,本地先同步剔除被移除工作区的会话
    pub fn remove_workspace(&mut self, name: &str, cx: &mut Context<Self>) {
        match self.bridge.host().remove_workspace(name) {
            Ok(()) => {
                self.sessions.menu_open_ws = None;
                self.refresh_workspaces();
                let default = self.default_workspace();
                self.state
                    .sessions
                    .retain(|s| reducer::workspace_of(&s.session_id, &default) != name);
                self.refresh_list(cx);
                if self.state.active_workspace.as_deref() == Some(name) {
                    self.state.active_workspace =
                        self.bridge.host().workspace_names().first().cloned();
                    match self.state.sessions.first().map(|s| s.session_id.clone()) {
                        Some(next) => self.open_session(&next, cx),
                        None => self.create_session(cx),
                    }
                } else {
                    cx.notify();
                }
            }
            Err(e) => self.push_local_notice(&format!("移除失败:{}", e.message), cx),
        }
    }

    /// 打开重命名(输入态惰建 + Enter 确认订阅;点击回调自带 window)
    pub fn open_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.sessions.rename_input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("会话标题"));
            cx.subscribe(&input, |this, _i, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    this.confirm_rename(cx);
                }
            })
            .detach();
            self.sessions.rename_input = Some(input);
        }
        let current = self.title_for(id);
        if let Some(input) = &self.sessions.rename_input {
            input.update(cx, |s, cx| s.set_value(&current, window, cx));
        }
        self.sessions.rename_target = Some(id.to_string());
        self.sessions.session_menu_pos = None;
        cx.notify();
    }

    /// 确认重命名(空标题视为取消;工作区与 会话共用输入态,按目标分派)
    pub fn confirm_rename(&mut self, cx: &mut Context<Self>) {
        let title = self
            .sessions
            .rename_input
            .as_ref()
            .map(|e| e.read(cx).value().trim().to_string())
            .unwrap_or_default();
        if let Some(ws) = self.sessions.rename_ws_target.take() {
            if !title.is_empty() {
                match self.bridge.host().rename_workspace(&ws, &title) {
                    Ok(()) => {
                        self.refresh_workspaces();
                        self.refresh_list(cx);
                    }
                    Err(e) => self.push_local_notice(&format!("重命名失败:{}", e.message), cx),
                }
            }
            cx.notify();
            return;
        }
        let Some(id) = self.sessions.rename_target.clone() else {
            return;
        };
        if !title.is_empty() && self.bridge.host().rename(&id, &title).is_ok() {
            self.state.titles.insert(id.clone(), title);
            self.refresh_list(cx);
        }
        self.sessions.rename_target = None;
        cx.notify();
    }

    /// 取消重命名
    pub fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        self.sessions.rename_target = None;
        cx.notify();
    }

    /// 分叉会话(按最后一个完成轮截断复制为新会话并打开;失败走通告)
    pub fn fork(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.bridge.host().fork_session(id, None) {
            Ok(new_id) => {
                self.push_local_session_row(new_id.clone(), false, Some(id.to_string()), None);
                self.refresh_list(cx);
                self.open_session(&new_id, cx);
            }
            Err(e) => self.push_local_notice(&format!("分叉失败:{}", e.message), cx),
        }
    }

    /// 归档会话(当前会话被归档 → 打开剩余首个,无则新建)。
    /// 刷新已异步化:切走判定不得依赖尚未回填的清单,本地先同步剔除
    /// 被归档项(回填后以宿主清单为权威)
    pub fn archive(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.bridge.host().archive_session(id).is_ok() {
            self.state.sessions.retain(|s| s.session_id != id);
            self.refresh_list(cx);
            if self.state.current_id.as_deref() == Some(id) {
                match self.state.sessions.first().map(|s| s.session_id.clone()) {
                    Some(next) => self.open_session(&next, cx),
                    None => self.create_session(cx),
                }
            }
        }
    }

    /// 导出会话日志(会话行菜单入口,按行 id 导出而非仅当前会话):
    /// 先弹系统保存对话框由用户选定路径(不再默认落 ~/Downloads),
    /// 选定后写 ZIP(根 + fork 后代血缘);ZIP 失败回落单文件文本
    /// (扩展名换 .jsonl)。结果以右上角通知呈现(不占消息流),
    /// 取消 = 静默。
    pub fn export_session_log(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::WindowExt as _;
        use gpui_kit::component::notification::Notification;
        let host = self.bridge.host().clone();
        let id = id.to_string();
        let safe = id.replace('/', "-");
        let rx =
            cx.prompt_for_new_path(&downloads_dir(), Some(&format!("liuma-session-{safe}.zip")));
        window
            .spawn(cx, async move |cx| {
                let notify_err = |cx: &mut gpui_kit::AsyncWindowContext, msg: String| {
                    let _ = cx.update(|window, cx| {
                        window.push_notification(Notification::error(msg).title("导出失败"), cx);
                    });
                };
                let chosen = match rx.await {
                    Ok(Ok(Some(path))) => path,
                    Ok(Ok(None)) => return, // 用户取消:静默
                    Ok(Err(e)) => return notify_err(cx, format!("保存对话框打开失败:{e}")),
                    Err(_) => return, // 通道断开(窗口销毁)
                };
                let (path, bytes, title) = match host.export_session_zip(&id, true) {
                    Ok(bytes) => (chosen, bytes, "已导出(含分叉后代)"),
                    Err(_) => {
                        let Ok(log) = host.export_session_log(&id) else {
                            return notify_err(cx, "导出失败:会话日志不可读".into());
                        };
                        (
                            chosen.with_extension("jsonl"),
                            log.into_bytes(),
                            "已导出(单文件)",
                        )
                    }
                };
                if let Err(e) = std::fs::write(&path, bytes) {
                    return notify_err(cx, format!("写入失败:{e}"));
                }
                let _ = cx.update(|window, cx| {
                    window.push_notification(
                        Notification::success(path.display().to_string()).title(title),
                        cx,
                    );
                });
            })
            .detach();
    }

    /// 会话标题:重命名 > 投影 > 空白「新会话」 > id。
    /// 空串标题视同缺省(历史投影曾下发烧穿串,会永久遮蔽清单摘录)
    /// 工作区显示标题(覆盖 → basename;身份恒用 basename)
    pub fn title_for_workspace(&self, name: &str) -> String {
        self.sessions
            .ws_titles
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    pub fn title_for(&self, id: &str) -> String {
        if let Some(t) = self.state.titles.get(id).filter(|t| !t.is_empty()) {
            return t.clone();
        }
        if let Some(s) = self.state.sessions.iter().find(|s| s.session_id == id) {
            if let Some(t) = s
                .projections
                .as_ref()
                .and_then(|p| p.values.get("title"))
                .and_then(|v| v.as_str())
            {
                return t.to_string();
            }
            if s.blank {
                return "新会话".into();
            }
        }
        id.to_string()
    }

    /// 空白会话判定:清单 blank 且投影无**内容**节点(投影先行时以节点
    /// 为准)。注入行(ChatNode::Context,attach 基线/AGENTS.md/@session
    /// 快照)是系统注入,不是用户或模型说的话——不算内容,否则新会话
    /// 的 AGENTS.md 基线注入会顶掉 hero 空态(模式选择 chip)。
    pub fn is_blank(&self, id: &str) -> bool {
        let has_nodes = self
            .state
            .chats
            .get(id)
            .map(|c| {
                c.nodes
                    .iter()
                    .any(|n| !matches!(n, ChatNode::Context { .. }))
            })
            .unwrap_or(false);
        if has_nodes {
            return false;
        }
        self.state
            .sessions
            .iter()
            .find(|s| s.session_id == id)
            .map(|s| s.blank)
            .unwrap_or(true)
    }

    /// 默认工作区名(= basename(cwd),describe workspaces[0] 同源)
    pub fn default_workspace(&self) -> String {
        self.state
            .host_info
            .workspaces
            .first()
            .cloned()
            .unwrap_or_else(|| basename(&self.state.host_info.cwd))
    }

    /// 非默认工作区才返回 Some(新建会话目标)
    pub(crate) fn non_default_workspace(&self) -> Option<String> {
        let default = self.default_workspace();
        match &self.state.active_workspace {
            Some(ws) if *ws != default => Some(ws.clone()),
            _ => None,
        }
    }

    /// 会话活动连带其工作区选中(对齐 web)
    fn sync_active_workspace_from_current(&mut self) {
        if let Some(id) = &self.state.current_id {
            let default = self.default_workspace();
            self.state.active_workspace = Some(reducer::workspace_of(id, &default).to_string());
        }
    }
}

/// 路径末段(`/a/b` → `b`;空 → 空串)
fn basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basename_tail() {
        assert_eq!(basename("/Volumes/DATA/proj"), "proj");
        assert_eq!(basename("proj"), "proj");
        assert_eq!(basename("/"), "");
    }
}
