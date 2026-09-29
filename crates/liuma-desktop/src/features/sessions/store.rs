//! 会话与工作区树的 store 域:侧栏行菜单/重命名/删除确认、会话行
//! CRUD(fork/archive/导出)、工作区 CRUD 与下拉/组头菜单、以及会话
//! 命名/工作区归属镜像查询(title_for/workspace_of/basename)。视图见
//! features::sessions::views;行/组菜单开态本域自持,shell 底座经
//! close_all_menus 统一关闭。

use std::collections::{HashMap, HashSet};

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::features::chat::ChatNode;
use crate::kits::i18n::t;
use crate::kits::theme;
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
    /// 重命名目标会话
    pub rename_target: Option<String>,
    /// 标题栏会话 ⋯ 菜单开态(受控 Popover;该钮在标题栏拖拽区上,
    /// 须保留 mousedown 豁免 → 无法用库内部开态,退化为纯 bool)
    pub session_menu_open: bool,
    /// 侧栏视图选项菜单开态(受控;同上,钮在侧栏头拖拽区上)
    pub view_menu_open: bool,
    /// 重命名输入态(挂窗后建)
    pub rename_input: Option<Entity<InputState>>,
    /// 标题栏工作区下拉开态
    pub workspace_menu_open: bool,
    /// 工作区名 → 路径(workspace_view 解析)
    pub ws_paths: HashMap<String, std::path::PathBuf>,
    /// 工作区名 → 显示标题(workspace_view 解析;标题仅显示层)
    pub ws_titles: HashMap<String, String>,
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
    /// 置顶会话(host settings 权威,refresh 时拉取;侧栏「置顶」节)
    pub pinned_sessions: Vec<String>,
    /// 置顶工作区(host settings 权威;置顶的工作区从「项目」节提出)
    pub pinned_workspaces: Vec<String>,
    /// 「展开显示」已额外展开的批数(键 = 工作区;每批 5 条,分页
    /// 渐进;重置 = 收起。内存视图态)
    pub expanded_show: HashMap<String, usize>,
    /// 工作区信息卡悬停源:行右段 / 铅笔钮 / 卡本体。内存视图态
    pub ws_info_hover_row: Option<String>,
    pub ws_info_hover_card: bool,
    /// 信息卡关闭延迟任务(行/卡双离开后启动;新悬停 drop 旧任务 =
    /// 取消)
    pub ws_info_close_task: Option<gpui_kit::Task<()>>,
    /// 信息卡开态:Some((工作区, 进入点光标位置))。hover 回调在事件
    /// 分发期执行,window.mouse_position() 即当帧指针位,卡随开随有
    /// 锚——无渲染期捕获(canvas/prepaint)的一帧延迟。根级卡按侧栏
    /// 右缘 + 锚 y 定位(见 shell/mod)
    pub ws_info_card: Option<(String, gpui_kit::Point<gpui_kit::Pixels>)>,
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

    /// 侧栏单选写入口:开会话即清工作区选中(清会话选中不动工作区
    /// 选中,remove_workspace 需先清会话再重指工作区)。current_id
    /// 的所有写入必须经此,保证两态互斥
    fn select_session(&mut self, id: Option<String>) {
        let selected = id.is_some();
        self.state.current_id = id;
        if selected {
            self.state.active_workspace = None;
        }
    }

    /// 打开会话:切换 + 历史尾窗加载(后台折叠)+ 统计 + 配置缓存
    pub fn open_session(&mut self, id: &str, cx: &mut Context<Self>) {
        self.select_session(Some(id.to_string()));
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
        // hint,残留脏标记会在下一帧触发对旧会话槽位的重测 = 打开优化破功
        self.chat.tv_remeasure_keys.clear();
        self.chat.node_slot.clear();
        self.chat.nav_anchors_cache = None;
        self.chat.anchor_index.clear();
        self.chat.row_slots.clear();
        self.chat.row_slots_sig = None;
        // active_workspace 不随会话联动:工作区选中是独立状态,仅由
        // 显式点击工作区行/下拉设置(否则会话选中跨节点亮工作区行,
        // 见侧栏互斥渲染的系列回归)
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
                    s.push_local_notice(t!("sessions.history_load_failed", detail = &detail), cx);
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
                    // 会话是否仍在运行(运行中不冻结:交给 live 事件收尾)
                    let live = s.state.running_by_id.get(&id).copied().unwrap_or(false);
                    let chat = s.state.chats.entry(id.clone()).or_default();
                    chat.merge_history(page_events);
                    if !live {
                        // 日志停在回合中途(被杀回合)时,未落定的调用会永久
                        // 停在 Running —— 渲染层的运行扫光是 repeat 动画,
                        // 会让整窗 60fps 永久重绘
                        chat.freeze_unfinished_calls();
                    }
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
        let id = self.bridge.host().create_session(None, None, ws.clone());
        let cwd = self.ws_cwd_label(ws.as_deref());
        self.push_local_session_row(id.clone(), true, None, Some(cwd));
        self.refresh_list(cx);
        self.open_session(&id, cx);
    }

    /// 选中工作区(侧栏组头/置顶行/工作区下拉/添加工作区共用):设
    /// active_workspace + 目标组自动展开 + 分支表刷新(切到的可能是
    /// 别的 repo),并清空会话选中(右栏切 hero 空态)。不打开也不
    /// 新建会话——新建经「+」钮/新会话钮落入选中工作区
    pub fn select_workspace(&mut self, ws: &str, cx: &mut Context<Self>) {
        self.state.active_workspace = Some(ws.to_string());
        // 单选互斥:选中工作区即清会话选中(右栏切 hero 空态;新会话
        // 经「+」钮/顶栏按钮落入选中工作区,hero 态发送即建会话)。
        // 此前「顺手打开/新建会话」会让两态并存,侧栏多行同亮
        self.state.current_id = None;
        self.sessions.collapsed_workspaces.remove(ws);
        self.refresh_branches();
        cx.notify();
    }

    /// 生效工作区的绝对路径(@file 补全根;源以 header.cwd 为根)。
    /// 会话态取会话归属(id 形如 `wsName/sessionId` 非默认区 / 裸 id
    /// 默认区);工作区选中态取所选区(hero 态 @file/文件树/预览有根)
    pub fn current_workspace_dir(&self) -> Option<std::path::PathBuf> {
        let default = self.default_workspace();
        let ws = match &self.state.current_id {
            Some(cid) => reducer::workspace_of(cid, &default).to_string(),
            None => self.state.active_workspace.clone()?,
        };
        // ws_paths 以工作区名为键;兜底用自身(单工作区场景默认工作区)
        self.sessions
            .ws_paths
            .get(&ws)
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
        let (pinned_sessions, pinned_workspaces) = self.bridge.host().pinned();
        self.sessions.pinned_sessions = pinned_sessions;
        self.sessions.pinned_workspaces = pinned_workspaces;
    }

    /// 置顶/取消置顶会话(host 落盘为权威,成功后本地同步 + 清单刷新)
    pub fn toggle_pinned_session(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.bridge.host().toggle_pinned_session(id).is_ok() {
            let (ps, pw) = self.bridge.host().pinned();
            self.sessions.pinned_sessions = ps;
            self.sessions.pinned_workspaces = pw;
            self.refresh_list(cx);
        }
    }

    /// 置顶/取消置顶工作区(同上)
    pub fn toggle_pinned_workspace(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.bridge.host().toggle_pinned_workspace(name).is_ok() {
            let (ps, pw) = self.bridge.host().pinned();
            self.sessions.pinned_sessions = ps;
            self.sessions.pinned_workspaces = pw;
            self.refresh_list(cx);
        }
    }

    /// 行右段悬停变化:进入即开卡(锚 = 进入点光标位置;hover 回调
    /// 在事件分发期执行,window.mouse_position() 即当帧指针位);离开
    /// 启动 300ms 延迟关(卡悬停或重入行即取消——指针跨越行与卡间
    /// 空隙时卡保持在场)
    pub fn set_ws_info_hover_row(
        &mut self,
        ws: Option<String>,
        at: gpui_kit::Point<gpui_kit::Pixels>,
        cx: &mut Context<Self>,
    ) {
        match ws {
            Some(key) => {
                self.sessions.ws_info_hover_row = Some(key.clone());
                self.sessions.ws_info_card = Some((key, at));
                self.sessions.ws_info_close_task = None;
            }
            None => {
                self.sessions.ws_info_hover_row = None;
                if !self.sessions.ws_info_hover_card {
                    self.ws_info_arm_close(cx);
                }
            }
        }
        cx.notify();
    }

    /// 卡本体悬停变化
    pub fn set_ws_info_hover_card(&mut self, hovering: bool, cx: &mut Context<Self>) {
        self.sessions.ws_info_hover_card = hovering;
        if hovering {
            self.sessions.ws_info_close_task = None;
        } else if self.sessions.ws_info_hover_row.is_none() {
            self.ws_info_arm_close(cx);
        }
        cx.notify();
    }

    /// 双离开后 300ms 关卡(期间任一悬停恢复即取消任务)
    fn ws_info_arm_close(&mut self, cx: &mut Context<Self>) {
        if self.sessions.ws_info_close_task.is_some() {
            return;
        }
        let store = cx.entity().clone();
        self.sessions.ws_info_close_task = Some(cx.spawn(async move |_this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(300))
                .await;
            store.update(cx, |s, cx| {
                if s.sessions.ws_info_hover_row.is_none() && !s.sessions.ws_info_hover_card {
                    s.sessions.ws_info_card = None;
                    s.sessions.ws_info_close_task = None;
                    cx.notify();
                }
            });
        }));
    }

    /// 「展开显示」:该组多显一批(5 条;分页渐进)
    pub fn expand_workspace_more(&mut self, ws: &str, cx: &mut Context<Self>) {
        *self
            .sessions
            .expanded_show
            .entry(ws.to_string())
            .or_insert(0) += 1;
        cx.notify();
    }

    /// 「收起显示」:重置该组为预览态(5 条)
    pub fn collapse_workspace(&mut self, ws: &str, cx: &mut Context<Self>) {
        self.sessions.expanded_show.remove(ws);
        cx.notify();
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
        let ws = self.effective_workspace();
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
            tokio::task::spawn_blocking(move || host.pick_workspace_directory())
                .await
                .unwrap_or_else(|e| {
                    Err(liuma_core::proto::RpcError::internal(t!(
                        "sessions.picker_task_failed",
                        e = &e
                    )))
                })
        });
        let store = cx.entity().clone();
        cx.spawn(async move |_this, cx| {
            // 内层 Err = 选择失败/取消;外层 = 通道
            let picked = rx.await.unwrap_or_else(|_| {
                Err(liuma_core::proto::RpcError::internal(
                    t!("sessions.picker_channel_failed").to_string(),
                ))
            });
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
                                s.push_local_notice(
                                    t!("sessions.add_workspace_failed", msg = &e.message),
                                    cx,
                                );
                            }
                        }
                        cx.notify();
                    });
                }
                // 用户取消:预期分支,静默
                Err(e) if e.code == "bad-request" => {}
                Err(e) => {
                    store.update(cx, |s, cx| {
                        s.push_local_notice(t!("sessions.open_dir_failed", msg = &e.message), cx);
                        cx.notify();
                    });
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
        cx.notify();
    }

    /// 标题栏会话 ⋯ 菜单开/收(受控 Popover)
    pub fn toggle_session_menu(&mut self, cx: &mut Context<Self>) {
        self.sessions.session_menu_open = !self.sessions.session_menu_open;
        cx.notify();
    }

    /// 侧栏视图选项菜单开/收(受控 Popover)
    pub fn toggle_view_menu(&mut self, cx: &mut Context<Self>) {
        self.sessions.view_menu_open = !self.sessions.view_menu_open;
        cx.notify();
    }

    /// 切换列表分组方式(菜单项选择即收菜单)
    pub fn set_group_mode(&mut self, mode: GroupMode, cx: &mut Context<Self>) {
        self.sessions.group_mode = mode;
        cx.notify();
    }

    /// 切换列表排序方式(菜单项选择即收菜单)。手动排序本期仅占位:
    /// 拖拽重排未实现,菜单项置灰不触发
    pub fn set_order_mode(&mut self, mode: OrderMode, cx: &mut Context<Self>) {
        if mode == OrderMode::Manual {
            return;
        }
        self.sessions.order_mode = mode;
        cx.notify();
    }

    /// 打开工作区重命名(复用会话重命名输入态)
    pub fn open_rename_workspace(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let already_open_ws =
            self.sessions.rename_target.is_some() || self.sessions.rename_ws_target.is_some();
        if self.sessions.rename_input.is_none() {
            let input =
                cx.new(|cx| InputState::new(window, cx).placeholder(t!("sessions.ws_title_ph")));
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
        if !already_open_ws {
            self.open_rename_dialog(window, cx);
        }
        cx.notify();
    }

    /// 移除工作区(默认工作区拒绝;当前工作区被移除 → 切回首会话/新建)。
    /// 刷新已异步化:同 archive,本地先同步剔除被移除工作区的会话
    pub fn remove_workspace(&mut self, name: &str, cx: &mut Context<Self>) {
        match self.bridge.host().remove_workspace(name) {
            Ok(()) => {
                self.refresh_workspaces();
                let default = self.default_workspace();
                self.state
                    .sessions
                    .retain(|s| reducer::workspace_of(&s.session_id, &default) != name);
                self.refresh_list(cx);
                // 单选互斥收口:① 被删区正被选中 → 选首余工作区(不落
                // 会话);② 当前会话属被删区(或已不在清单)→ 清会话选中
                // 回 hero。先清会话再重指工作区,顺序不可倒。不再强开/
                // 新建会话——同 select_workspace 的单选纪律
                if self
                    .state
                    .current_id
                    .as_deref()
                    .is_some_and(|cid| !self.state.sessions.iter().any(|s| s.session_id == *cid))
                {
                    self.select_session(None);
                }
                if self.state.active_workspace.as_deref() == Some(name) {
                    self.state.active_workspace =
                        self.bridge.host().workspace_names().first().cloned();
                }
                cx.notify();
            }
            Err(e) => self.push_local_notice(t!("sessions.remove_failed", msg = &e.message), cx),
        }
    }

    /// 打开重命名(输入态惰建 + Enter 确认订阅;模态走组件库 Dialog
    /// 层,Esc/遮罩/焦点陷阱由库托管)
    pub fn open_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let already_open =
            self.sessions.rename_target.is_some() || self.sessions.rename_ws_target.is_some();
        if self.sessions.rename_input.is_none() {
            let input = cx
                .new(|cx| InputState::new(window, cx).placeholder(t!("sessions.session_title_ph")));
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
        if !already_open {
            self.open_rename_dialog(window, cx);
        }
        cx.notify();
    }

    /// 重命名模态(组件库 Dialog 层:标题按目标分派会话/工作区 +
    /// 输入 + 取消/重命名 footer;确认/取消/X/Esc/遮罩全部收敛到
    /// store 动作。库 Dialog 无默认 footer——按钮自绘)
    fn open_rename_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::WindowExt as _;
        let Some(input) = self.sessions.rename_input.clone() else {
            return;
        };
        let ws_mode = self.sessions.rename_ws_target.is_some();
        let store = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let input = input.clone();
            let title = if ws_mode {
                t!("misc.rename_workspace")
            } else {
                t!("misc.rename_session")
            };
            let store_ok = store.clone();
            let store_cancel = store.clone();
            dialog
                .title(title)
                .w(px(420.))
                .bg(theme::LAYER())
                .content({
                    let input = input.clone();
                    move |content, _, _| {
                        content.child(gpui_kit::component::input::Input::new(&input))
                    }
                })
                .footer(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            div()
                                .id("rename-cancel")
                                .debug_selector(|| "rename-cancel".to_string())
                                .flex()
                                .h(px(32.))
                                .items_center()
                                .px(px(16.))
                                .rounded(px(16.))
                                .border_1()
                                .border_color(theme::BORDER())
                                .cursor_pointer()
                                .text_size(px(13.))
                                .text_color(theme::LABEL_2())
                                .hover(|s| s.bg(theme::DOCK()))
                                .child(t!("common.cancel"))
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                }),
                        )
                        .child(
                            div()
                                .id("rename-ok")
                                .debug_selector(|| "rename-ok".to_string())
                                .flex()
                                .h(px(32.))
                                .items_center()
                                .px(px(16.))
                                .rounded(px(16.))
                                .bg(theme::BRAND())
                                .cursor_pointer()
                                .text_size(px(13.))
                                .text_color(theme::LABEL())
                                .hover(|s| s.opacity(0.9))
                                // 确认钮文案 = 动作词(参照:「重命名」)
                                .child(if ws_mode {
                                    t!("sessions.rename")
                                } else {
                                    t!("misc.rename_session")
                                })
                                .on_click(move |_, window, cx| {
                                    store_ok.update(cx, |st, cx| st.confirm_rename(cx));
                                    window.close_dialog(cx);
                                }),
                        ),
                )
                .on_cancel({
                    let store_cancel = store_cancel.clone();
                    move |_, _, cx| {
                        store_cancel.update(cx, |st, cx| st.cancel_rename(cx));
                        true
                    }
                })
                .on_close({
                    let store_close = store_cancel.clone();
                    move |_, _, cx| {
                        store_close.update(cx, |st, cx| st.cancel_rename(cx));
                    }
                })
        });
    }

    /// 确认重命名(空标题视为取消;工作区与 会话共用输入态,按目标分派;
    /// Enter 订阅路径无 window,经 with_window 桥关库 Dialog 层)
    pub fn confirm_rename(&mut self, cx: &mut Context<Self>) {
        self.with_window_deferred(cx, |window, cx| window.close_dialog(cx));
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
                    Err(e) => {
                        self.push_local_notice(t!("sessions.rename_failed", msg = &e.message), cx)
                    }
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
        self.sessions.rename_ws_target = None;
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
            Err(e) => self.push_local_notice(t!("sessions.fork_failed", msg = &e.message), cx),
        }
    }

    /// 归档会话(当前会话被归档 → 打开剩余首个,无则新建)。
    /// 刷新已异步化:切走判定不得依赖尚未回填的清单,本地先同步剔除
    /// 被归档项(回填后以宿主清单为权威)
    pub fn archive(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.bridge.host().archive_session(id).is_ok() {
            self.state.sessions.retain(|s| s.session_id != id);
            self.refresh_list(cx);
            // 归档当前会话 → 回 hero(不隐式跳下一会话:单选模型下
            // 自动导航属意外选中;再聊点行或「+」)
            if self.state.current_id.as_deref() == Some(id) {
                self.select_session(None);
            }
            cx.notify();
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
                        window.push_notification(
                            Notification::error(msg).title(t!("sessions.export_failed")),
                            cx,
                        );
                    });
                };
                let chosen = match rx.await {
                    Ok(Ok(Some(path))) => path,
                    Ok(Ok(None)) => return, // 用户取消:静默
                    Ok(Err(e)) => {
                        return notify_err(
                            cx,
                            t!("sessions.save_dialog_failed", e = &e).into_owned(),
                        );
                    }
                    Err(_) => return, // 通道断开(窗口销毁)
                };
                let (path, bytes, title) = match host.export_session_zip(&id, true) {
                    Ok(bytes) => (chosen, bytes, t!("sessions.exported_zip").to_string()),
                    Err(_) => {
                        let Ok(log) = host.export_session_log(&id) else {
                            return notify_err(cx, t!("sessions.export_unreadable").to_string());
                        };
                        (
                            chosen.with_extension("jsonl"),
                            log.into_bytes(),
                            t!("sessions.exported_single").to_string(),
                        )
                    }
                };
                if let Err(e) = std::fs::write(&path, bytes) {
                    return notify_err(cx, t!("sessions.write_failed", e = &e).into_owned());
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
        let title = if let Some(t) = self.state.titles.get(id).filter(|t| !t.is_empty()) {
            t.clone()
        } else if let Some(s) = self.state.sessions.iter().find(|s| s.session_id == id) {
            match s
                .projections
                .as_ref()
                .and_then(|p| p.values.get("title"))
                .and_then(|v| v.as_str())
            {
                Some(t) => t.to_string(),
                None if s.blank => t!("sessions.new_session").into(),
                None => id.to_string(),
            }
        } else {
            id.to_string()
        };
        single_line(&title)
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

    /// 非默认工作区才返回 Some(新建会话目标)。读生效工作区:选中态
    /// 落选中工作区;会话态落当前会话工作区(新会话跟手)
    pub(crate) fn non_default_workspace(&self) -> Option<String> {
        let default = self.default_workspace();
        let ws = self.effective_workspace();
        (ws != default).then_some(ws)
    }

    /// 新会话本地行的 cwd 标签:目标工作区路径(`ws_paths` 由宿主工作
    /// 区表回填),无表项兜底宿主当前路径。此前恒记宿主当前路径(=
    /// 默认区),非默认目标的新会话行 @file 根错位
    fn ws_cwd_label(&self, ws: Option<&str>) -> String {
        let default = self.default_workspace();
        let name = ws.unwrap_or(&default);
        if let Some(p) = self.sessions.ws_paths.get(name) {
            return p.display().to_string();
        }
        self.bridge.host().workspace().display().to_string()
    }

    /// 生效工作区 = 显式选中 > 当前会话归属 > 默认工作区。
    /// 选中态互斥(active_workspace 与 current_id 不同时在场),读端
    /// 统一走此入口取「界面上下文工作区」,不感知选中形态
    pub fn effective_workspace(&self) -> String {
        if let Some(ws) = &self.state.active_workspace {
            return ws.clone();
        }
        if let Some(id) = &self.state.current_id {
            return reducer::workspace_of(id, &self.default_workspace()).to_string();
        }
        self.default_workspace()
    }
}

/// 路径末段(`/a/b` → `b`;空 → 空串)
fn basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or("").to_string()
}

/// 标题单行化:换行折成空格(其余字符原样,含全角空格)。
///
/// 标题的每个槽都按**单行**布局——侧栏行是 34px 定高的 `truncate()` 文本,
/// 标题栏是绝对定位的居中区。而临时标题就是首条用户消息原文(模型生成的
/// 标题落档前一直用它),原文里的 `\n` 会让文本元素长成多行:侧栏行被撑爆、
/// 居中区溢出。`.truncate()` 挡不住它 —— nowrap 只禁软换行,硬换行照断。
fn single_line(title: &str) -> String {
    title.replace(['\n', '\r'], " ").trim().to_string()
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

    #[test]
    fn titles_collapse_to_a_single_line() {
        assert_eq!(single_line("1. 单选\n2. 多选"), "1. 单选 2. 多选");
        assert_eq!(single_line("甲\r\n乙"), "甲  乙");
        assert_eq!(single_line("  首尾留白  "), "首尾留白");
        // 其余空白(含全角)原样保留:只折硬换行
        assert_eq!(single_line("甲　乙 丙"), "甲　乙 丙");
    }
}
