//! 轨迹功能切片的 store 域:台账缓存与版本、检查器(目标/tab/宽度/折叠)、
//! 时间线(选区/视口/拖拽)、turns/calls 折叠、工具卡 Inspect 待定位。
//! 视图见 features::trajectory::views;状态自持,shell 底座经 method
//! 调用跨功能互作(open_session/open_panel_tab 触 refresh_trajectory)。

use std::collections::HashSet;

use gpui_kit::component::input::InputState;
use gpui_kit::{Context, Entity};

use liuma_core::trajectory::{TrajectoryRecord, TrajectoryRequest};

use crate::features::trajectory::views::LedgerCache;
use crate::shell::panel::PanelTab;
use crate::shell::store::AppStore;

/// 检查器目标(台账记录 / 请求)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectTarget {
    /// 台账记录(全局 index)
    Record(u64),
    /// 请求(#N)
    Request(u64),
}

/// 会话轨迹台账缓存(来自 `AppHost::trajectory_page`;切会话重拉)。
#[derive(Debug, Default, Clone)]
pub struct TrajectoryView {
    /// 台账记录(尾窗,时间序)
    pub records: Vec<TrajectoryRecord>,
    /// 全量请求清单(#1..#N)
    pub requests: Vec<TrajectoryRequest>,
    /// 是否有更早记录(beforeIndex 分页)
    pub has_older: bool,
    /// 总记录数(全会话)
    pub total: u64,
    /// 首拉进行中(防重复)
    pub loading: bool,
    /// 「加载更早」进行中(防重复)
    pub loading_older: bool,
}

/// 轨迹初始尾窗大小(RPC clamp 1..2000)
pub const TRAJECTORY_WINDOW: usize = 200;
/// 「加载更早」页大小
pub const TRAJECTORY_PAGE: usize = 500;
/// 桌面侧已载记录上限(「加载更早」可反复翻,无上限则窗口随使用时长
/// 单调增长)。超出丢**最旧**并把 `has_older` 置真——与「加载更早」语义
/// 自洽(丢掉的正是那批最早的行),可见台账永远是最近 MAX_RETAINED 行
pub const TRAJECTORY_MAX_RETAINED: usize = 2000;

/// 轨迹功能切片状态(台账缓存与滚动态、检查器与时间线交互态、
/// turns/calls 折叠与 Inspect 待定位)。
pub(crate) struct TrajectoryStore {
    /// Inspect 待定位(会话 id, 记录 kind, seq):切轨迹后按 kind+seq
    /// 选台账行。工具卡给 `tool`,压缩标记行给 `compacted`——kind 必带,
    /// 否则两种跳转会在同 seq 上撞车
    pub inspect_locate: Option<(String, String, u64)>,
    // ── 轨迹视图(数据缓存 + 交互态;trajectory_session 标识缓存归属)──
    /// 台账缓存(当前会话)
    pub trajectory: TrajectoryView,
    /// 轨迹缓存归属会话(失配 = 需重拉)
    pub trajectory_session: Option<String>,
    /// 已应用的增量帧计数(trajectory/delta;基线回包竞态守卫基准)
    pub trajectory_deltas: u64,
    /// 当前基线拉取发起时的增量计数(回包落库时计数已前进 = 拉取
    /// 期间有增量到达,重拉一次收敛)
    pub trajectory_refresh_basis: u64,
    /// 轨迹搜索输入态(挂窗后建;渲染期读值过滤)
    pub trajectory_search: Option<Entity<InputState>>,
    /// 台账虚拟列表状态(内建 list();逐项测高缓存 + 滚动位托管)。
    /// 条数 = 行槽缓存长度,渲染期由 [`Self::sync_trajectory_rows`] 对齐
    pub trajectory_list: gpui_kit::ListState,
    /// 台账数据版本(拉取/翻页 +1;渲染侧比对驱动滚动)
    pub trajectory_version: u64,
    /// 渲染侧已消费的台账版本
    pub trajectory_rendered_version: u64,
    /// 行槽/投影行缓存(签名守卫;虚拟列表每帧按 index 直读,免每帧重建
    /// 全部行 + 免每帧深拷贝台账)
    pub view_cache: Option<LedgerCache>,
    /// 上次同步进 ListState 的行类型序列(定高行:类型序列相同 = 总高不变,
    /// 无需任何列表操作;增删/形状变化才走 splice/reset)
    pub row_kinds: Vec<u8>,
    /// 折叠态版本(行槽缓存签名的一份;折叠切换处 +1)
    pub collapse_ver: u64,
    /// 台账滚动回调是否已装(list 生命期只装一次)
    scroll_handler_installed: bool,
    /// 本帧发生过台账滚动(handler 只置位;需要回读 ListState 的判断
    /// 留到渲染期 flush,见 `flush_trajectory_scroll`)
    pub trajectory_scroll_dirty: bool,
    /// 列表当前位置是否**可信**。
    ///
    /// `ListAlignment::Top` 下 `logical_scroll_top()` 在「未滚动 / 被 reset」
    /// 时返回 `item_ix: 0` —— 与「真的在顶部」不可区分。而 `reset`(行槽形状
    /// 变化时必调)会清掉位置,于是每次 reset 后都会被误判成「在顶部」→
    /// 触发「加载更早」→ 又要 reset → **翻页级联**(实测 200→700→…→2000,
    /// 期间翻页行 Spinner 常转 = 整窗 60fps 十余秒)。故只在 reset 之后真的
    /// 滚动过时才认位置
    pub scroll_pos_known: bool,
    /// 悬浮回底钮可见性的滚动跟随镜像(事件字段 `is_following_tail`,
    /// 仅翻转时 notify——与聊天列 `at_bottom_ui` 同款去重缓存;渲染期
    /// 读 `trajectory_at_bottom()` 权威值)
    pub at_bottom_ui: bool,
    /// Duration 切换(时间线按耗时投影;进程内状态,不持久化)
    pub trajectory_duration: bool,
    /// 折叠的 turn(turn 号)
    pub collapsed_turns: HashSet<u64>,
    /// Turns 全局折叠(工具栏)
    pub all_turns_collapsed: bool,
    /// Calls 折叠的 assistant(message record index)
    pub collapsed_calls: HashSet<u64>,
    /// Calls 全局折叠(工具栏)
    pub all_calls_collapsed: bool,
    /// 检查器目标(None = 关闭)
    pub inspector: Option<InspectTarget>,
    /// 检查器激活 tab(实体切换时按最近访问恢复)
    pub inspector_tab: Option<&'static str>,
    /// 检查器最近访问 tab(跨实体记忆)
    pub inspector_last_tab: &'static str,
    /// 检查器宽度(左缘拖拽;320..720)
    pub inspector_width: f32,
    /// 拖宽锚点(按下时光标 x / 当时宽度)
    pub inspector_resize_anchor: Option<(f32, f32)>,
    /// Raw tab 的 Thinking 折叠
    pub inspector_raw_thinking: bool,
    /// JSON 树展开的节点路径(子级默认折叠;key=`{ix}/{path}`,
    /// path 为节点路径编码 `s{len}:{key}` / `n{index}`)
    pub json_expanded: HashSet<String>,
    /// Tools 页展开的工具名(目录卡片折叠态)
    pub expanded_inspector_tools: HashSet<String>,
    /// Tools 页展开集版本(检查器正文缓存签名的一份;切换处 +1)
    pub expanded_tools_ver: u64,
    /// 检查器正文派生缓存(签名守卫,单槽 = 当前 target+tab;
    /// pretty 序列化/逐行 tokenize/LCS 只在签名变化时跑)
    pub inspector_cache: Option<crate::features::trajectory::views::InspectorCache>,
    /// 时间线选区(域归一化 0..1;Some = 表格按时间窗过滤)
    pub timeline_selection: Option<(f64, f64)>,
    /// 时间线视口(域归一化 0..1;None = 全览)
    pub timeline_viewport: Option<(f64, f64)>,
    /// 时间线拖拽中(锚点域分数;Some 时渲染期注册窗口级 move/up)
    pub timeline_drag: Option<f64>,
    /// 拖拽草稿选区(未提交;渲染层叠加显示)
    pub timeline_draft: Option<(f64, f64)>,
}

/// 窗口裁剪:超 [`TRAJECTORY_MAX_RETAINED`] 丢最旧,返回丢弃条数。
/// 纯函数(调用点与单测共用)
pub(crate) fn trim_records(records: &mut Vec<TrajectoryRecord>) -> usize {
    if records.len() <= TRAJECTORY_MAX_RETAINED {
        return 0;
    }
    let drop = records.len() - TRAJECTORY_MAX_RETAINED;
    records.drain(..drop);
    drop
}

/// 已载子集按 index 升序的 upsert(新增插入保序;同 index 覆盖)
fn upsert_record(records: &mut Vec<TrajectoryRecord>, rec: TrajectoryRecord) {
    match records.binary_search_by_key(&rec.index, |r| r.index) {
        Ok(i) => records[i] = rec,
        Err(i) => records.insert(i, rec),
    }
}

impl Default for TrajectoryStore {
    fn default() -> Self {
        Self {
            inspect_locate: None,
            trajectory: TrajectoryView::default(),
            trajectory_session: None,
            trajectory_deltas: 0,
            trajectory_refresh_basis: 0,
            trajectory_search: None,
            trajectory_list: gpui_kit::ListState::new(
                0,
                gpui_kit::ListAlignment::Top,
                gpui_kit::px(600.),
            ),
            trajectory_version: 0,
            trajectory_rendered_version: 0,
            view_cache: None,
            row_kinds: Vec::new(),
            collapse_ver: 0,
            scroll_handler_installed: false,
            trajectory_scroll_dirty: false,
            scroll_pos_known: false,
            at_bottom_ui: true,
            trajectory_duration: false,
            collapsed_turns: HashSet::new(),
            all_turns_collapsed: false,
            collapsed_calls: HashSet::new(),
            all_calls_collapsed: false,
            inspector: None,
            inspector_tab: None,
            inspector_last_tab: "summary",
            inspector_width: 400.,
            inspector_resize_anchor: None,
            inspector_raw_thinking: false,
            json_expanded: HashSet::new(),
            expanded_inspector_tools: HashSet::new(),
            expanded_tools_ver: 0,
            inspector_cache: None,
            timeline_selection: None,
            timeline_viewport: None,
            timeline_drag: None,
            timeline_draft: None,
        }
    }
}

impl AppStore {
    /// 工具卡 Inspect:按 `call:{seq}` 登记待定位的 tool 记录并开轨迹
    /// 面板标签(= [`Self::inspect_record`] 的调用点包装)。
    /// callId == tool/call 事件 seq,与 chat.rs 的 `call:{seq}` key 同源。
    pub fn inspect_call(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(seq) = key
            .strip_prefix("call:")
            .and_then(|s| s.parse::<u64>().ok())
        else {
            return;
        };
        self.inspect_record("tool", seq, cx);
    }

    /// 定位台账记录并开轨迹面板(kind + seq 即锚:tool/call 的 seq 与
    /// 压缩标记行的 summary seq 各自唯一)。
    ///
    /// 与检索跳转同用「先登记、轨迹数据就绪后再定位」的延迟模式(见
    /// `open_search_hit`/`locate_search_hit`),避免会话在轨迹打开前缓存
    /// 为空导致同步 `records.find` 落空、跳转无声失败。
    pub fn inspect_record(&mut self, kind: &str, seq: u64, cx: &mut Context<Self>) {
        if let Some(sid) = self.state.current_id.clone() {
            self.trajectory.inspect_locate = Some((sid, kind.to_string(), seq));
        }
        self.open_panel_tab(PanelTab::Trajectory, cx);
        // 记录已在窗口(用户先前停在轨迹标签):立即定位;否则留待
        // refresh_trajectory 就绪后的 locate_inspect 收尾。
        self.locate_inspect(cx);
    }

    /// 轨迹就绪后按 kind+seq 定位台账记录(Inspect 收尾;展开所属 turn,
    /// 选中记录开检查器)。找不到时静默保留待定位,等下次轨迹数据覆盖再试。
    pub fn locate_inspect(&mut self, cx: &mut Context<Self>) {
        let Some((sid, kind, seq)) = self.trajectory.inspect_locate.clone() else {
            return;
        };
        if self.state.current_id.as_deref() != Some(sid.as_str()) {
            return;
        }
        let Some(rec) = self
            .trajectory
            .trajectory
            .records
            .iter()
            .find(|r| r.kind == kind && r.seq == seq)
        else {
            return;
        };
        let turn = rec.turn;
        if let Some(t) = turn {
            self.trajectory.collapsed_turns.remove(&t);
        }
        self.select_trajectory_record(rec.index, cx);
        self.trajectory.inspect_locate = None;
    }

    /// 轨迹缓存是否失配当前会话(切入 tab / 切会话时判断)
    fn trajectory_stale(&self) -> bool {
        self.trajectory.trajectory_session.as_deref() != self.state.current_id.as_deref()
    }

    /// 应用轨迹增量帧(`trajectory/delta`;宿主折叠器变更缓冲):
    /// records 按 index、requests 按 number upsert(全量对象覆盖)。
    /// 会话失配丢弃(基线拉取自会覆盖);不论面板是否可见都应用——
    /// upsert 极廉价,切回轨迹标签即见新数据。基线未载时照常正向
    /// 建账(增量与批量同源,收敛点一致)
    pub fn apply_trajectory_delta(&mut self, payload: &serde_json::Value, cx: &mut Context<Self>) {
        let Some(sid) = payload["sessionId"].as_str() else {
            return;
        };
        if self.state.current_id.as_deref() != Some(sid) {
            return;
        }
        let mut touched = false;
        if let Some(recs) = payload["records"].as_array() {
            for r in recs {
                let Ok(rec) = serde_json::from_value::<TrajectoryRecord>(r.clone()) else {
                    continue;
                };
                upsert_record(&mut self.trajectory.trajectory.records, rec);
                touched = true;
            }
        }
        if let Some(reqs) = payload["requests"].as_array() {
            for q in reqs {
                let Ok(req) = serde_json::from_value::<TrajectoryRequest>(q.clone()) else {
                    continue;
                };
                match self
                    .trajectory
                    .trajectory
                    .requests
                    .iter()
                    .position(|x| x.number == req.number)
                {
                    Some(i) => self.trajectory.trajectory.requests[i] = req,
                    None => self.trajectory.trajectory.requests.push(req),
                }
                touched = true;
            }
        }
        if let Some(total) = payload["total"].as_u64() {
            self.trajectory.trajectory.total = total;
            touched = true;
        }
        if !touched {
            return;
        }
        self.trajectory.trajectory_deltas += 1;
        // 窗口上限:超出丢最旧(丢的正是「加载更早」那批),has_older 由
        // 已载最左 index 推导(基线窗口/翻页语义保持)
        self.trim_trajectory_window();
        self.trajectory.trajectory.has_older = self
            .trajectory
            .trajectory
            .records
            .first()
            .is_some_and(|r| r.index > 1);
        self.trajectory.trajectory_version += 1;
        // 待定位(Inspect/检索跳转)的数据就绪收尾
        self.locate_search_hit(cx);
        self.locate_inspect(cx);
        cx.notify();
    }

    /// 已载记录超上限时丢最旧(内存有界;窗口语义见 [`TRAJECTORY_MAX_RETAINED`])
    fn trim_trajectory_window(&mut self) -> usize {
        trim_records(&mut self.trajectory.trajectory.records)
    }

    /// 拉取轨迹台账(尾窗 200;tokio worker 折叠,大日志不卡 UI 线程)。
    /// 完成回调里会话已切换则丢弃;同会话直播重拉保留选中/折叠态
    pub fn refresh_trajectory(&mut self, cx: &mut Context<Self>) {
        let Some(sid) = self.state.current_id.clone() else {
            return;
        };
        if self.trajectory.trajectory.loading {
            return;
        }
        let session_changed = self.trajectory_stale();
        self.trajectory.trajectory.loading = true;
        self.trajectory.trajectory_refresh_basis = self.trajectory.trajectory_deltas;
        // 换会话:数据整体替换,跟随态复位为「跟随尾部」——否则会把上一
        // 会话上滚留下的停跟态带进新会话,台账停在半空的旧偏移上。
        // 同会话重拉(reload/翻页收敛)不动用户的当前位置
        if session_changed {
            self.trajectory
                .trajectory_list
                .set_follow_mode(gpui_kit::FollowMode::Tail);
        }
        cx.notify();
        let host = self.bridge.host().clone();
        let sid_for_call = sid.clone();
        let rx = self
            .bridge
            .call_blocking(move || host.trajectory_page(&sid_for_call, TRAJECTORY_WINDOW, None));
        let store = cx.entity().clone();
        cx.spawn(async move |_this, cx| {
            // 掉线也要清 loading:否则 Spinner(repeat 动画)永久挂在面板上,
            // 整窗持续 60fps 重绘
            let Ok(page) = rx.await else {
                store.update(cx, |s, cx| {
                    s.trajectory.trajectory.loading = false;
                    cx.notify();
                });
                return Ok::<(), anyhow::Error>(());
            };
            store.update(cx, |s, cx| {
                // 会话已切换:丢弃过期页
                if s.state.current_id.as_deref() != Some(sid.as_str()) {
                    s.trajectory.trajectory.loading = false;
                    return;
                }
                let page = match page {
                    Ok(p) => p,
                    Err(e) => {
                        s.trajectory.trajectory.loading = false;
                        eprintln!("[liuma-desktop] 轨迹拉取失败:{}", e.message);
                        cx.notify();
                        return;
                    }
                };
                // 选中记录可能已不在窗口(翻页/直播后)——失配则关检查器
                if let Some(InspectTarget::Record(ix)) = s.trajectory.inspector
                    && !page.records.iter().any(|r| r.index == ix)
                {
                    s.trajectory.inspector = None;
                    s.trajectory.inspector_tab = None;
                }
                if session_changed {
                    s.trajectory.collapsed_turns.clear();
                    s.trajectory.collapsed_calls.clear();
                    s.trajectory.all_turns_collapsed = false;
                    s.trajectory.all_calls_collapsed = false;
                    s.trajectory.inspector = None;
                    s.trajectory.inspector_tab = None;
                    s.trajectory.timeline_selection = None;
                    s.trajectory.timeline_viewport = None;
                    s.trajectory.timeline_drag = None;
                    s.trajectory.timeline_draft = None;
                }
                s.trajectory.trajectory = TrajectoryView {
                    records: page.records,
                    requests: page.requests,
                    has_older: page.has_older,
                    total: page.total,
                    loading: false,
                    loading_older: false,
                };
                s.trajectory.trajectory_session = Some(sid.clone());
                s.trajectory.trajectory_version += 1;
                s.locate_search_hit(cx);
                s.locate_inspect(cx);
                // 拉取期间有增量到达:回包是拉取时刻快照,已落后——
                // 先落库再重拉一次收敛(delta 计数即守卫)
                if s.trajectory.trajectory_deltas != s.trajectory.trajectory_refresh_basis {
                    s.refresh_trajectory(cx);
                }
                cx.notify();
            });
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// 加载更早记录(beforeIndex = 已载首条;prepend + 视口锚定)
    pub fn load_earlier_trajectory(&mut self, cx: &mut Context<Self>) {
        if self.trajectory.trajectory.loading
            || self.trajectory.trajectory.loading_older
            || !self.trajectory.trajectory.has_older
        {
            return;
        }
        let Some(sid) = self.state.current_id.clone() else {
            return;
        };
        let Some(first) = self.trajectory.trajectory.records.first() else {
            return;
        };
        let before = first.index;
        self.trajectory.trajectory.loading_older = true;
        self.trajectory.trajectory_refresh_basis = self.trajectory.trajectory_deltas;
        cx.notify();
        let host = self.bridge.host().clone();
        let sid_for_call = sid.clone();
        let rx = self.bridge.call_blocking(move || {
            host.trajectory_page(&sid_for_call, TRAJECTORY_PAGE, Some(before))
        });
        let store = cx.entity().clone();
        cx.spawn(async move |_this, cx| {
            // 掉线也要清位(同上:Spinner 常转会永久烧 CPU)
            let Ok(page) = rx.await else {
                store.update(cx, |s, cx| {
                    s.trajectory.trajectory.loading_older = false;
                    cx.notify();
                });
                return Ok::<(), anyhow::Error>(());
            };
            store.update(cx, |s, cx| {
                if s.state.current_id.as_deref() != Some(sid.as_str()) {
                    s.trajectory.trajectory.loading_older = false;
                    return;
                }
                let Ok(page) = page else {
                    s.trajectory.trajectory.loading_older = false;
                    cx.notify();
                    return;
                };
                // 拉取期间有增量到达:prepend 会把落后页拼进已含增量的
                // 窗口——放弃拼接,改走全量重拉收敛
                if s.trajectory.trajectory_deltas != s.trajectory.trajectory_refresh_basis {
                    s.trajectory.trajectory.loading_older = false;
                    s.refresh_trajectory(cx);
                    return;
                }
                // 前插:行槽重建后由 sync_trajectory_rows 走 splice(0..0, added)
                // ——list 内部把 logical_scroll_top 的 item_ix 平移 +added,
                // 滚动位与已测高度自动保持,无需 px 锚定数学
                let mut records = page.records;
                records.extend(std::mem::take(&mut s.trajectory.trajectory.records));
                s.trajectory.trajectory.records = records;
                s.trajectory.trajectory.requests = page.requests;
                s.trajectory.trajectory.has_older = page.has_older;
                s.trajectory.trajectory.total = page.total;
                s.trajectory.trajectory.loading_older = false;
                s.trajectory.trajectory_version += 1;
                cx.notify();
            });
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// 装台账滚动回调(list 生命期只装一次)。
    ///
    /// 用 list 的 `set_scroll_handler` 而非包裹层 `on_scroll_wheel`:滚轮、
    /// 触控板、滚动条拖拽三者都触发,且回调在滚动时机(窗口/handler 齐备)
    /// 内跑。回调闭包持 store 实体(与既有 `tv_subs` 订阅同类,生命期 = 应用)。
    ///
    /// 两件事都在此一次备齐:
    /// - **跟随尾部交给 list 自身**(`FollowMode::Tail`):布局期自动滚底,
    ///   用户上滚即停跟,滚回底自动重挂(库内 `follow_state` 维护);
    /// - **回调体内不得回读本 `ListState`**:gpui 在 `StateInner::scroll` 里
    ///   持 `borrow_mut` 调用 handler,任何 `logical_scroll_top()` /
    ///   `max_offset_for_scrollbar()` 回读都是 `already mutably borrowed`
    ///   panic(实测:打开轨迹面板一滚即崩)。且 `ListScrollEvent.visible_range`
    ///   取的是**滚动前**的位置(gpui 在 `scroll` 内 shadow 了该局部量,
    ///   出块即失效),不能当判据。故 handler 只落一个「滚过了」的脏标记,
    ///   真正的判断放到渲染期(`flush_trajectory_scroll`,不在借用内)
    pub fn install_trajectory_scroll_handler(&mut self, cx: &mut Context<Self>) {
        if self.trajectory.scroll_handler_installed {
            return;
        }
        self.trajectory.scroll_handler_installed = true;
        let list = self.trajectory.trajectory_list.clone();
        // 跟随尾部由库托管:新记录到达即滚底,用户上滚停跟、回底自动重挂
        list.set_follow_mode(gpui_kit::FollowMode::Tail);
        let store = cx.entity();
        list.set_scroll_handler(move |ev, _, cx| {
            // 只落脏标记 + 镜像:需回读 ListState 的判断留到渲染期
            store.update(cx, |st, cx| st.on_trajectory_scroll(ev, cx));
        });
    }

    /// 滚动事件副作用(在 list 的 `borrow_mut` 内被调用:**禁止**回读
    /// `ListState`,见 [`Self::install_trajectory_scroll_handler`])。
    /// 只做安全的事:同步「跟随尾部」镜像(取自事件字段,库已维护)、
    /// 落脏标记让渲染期去做需要回读的判断、悬浮回顶/回底钮可见性跟手
    /// notify(回底 = 镜像翻转;回顶无事件字段,近顶区逐 tick notify,
    /// 渲染期读权威位置)
    pub fn on_trajectory_scroll(&mut self, ev: &gpui_kit::ListScrollEvent, cx: &mut Context<Self>) {
        // 滚轮/触控板/滚动条拖拽一律只置位:跟随态与翻页判断都在渲染期
        // 用当前真实位置算(见 `flush_trajectory_scroll`)
        self.trajectory.trajectory_scroll_dirty = true;
        // 真滚动过:gpui 在 scroll 内已把 logical_scroll_top 置为 Some,
        // 此后读到的位置可信(见 `scroll_pos_known`)
        self.trajectory.scroll_pos_known = true;
        // 回底钮:is_following_tail 翻转才 notify(聊天列同款去重)
        let at_bottom = ev.is_following_tail;
        if self.trajectory.at_bottom_ui != at_bottom {
            self.trajectory.at_bottom_ui = at_bottom;
            cx.notify();
        } else if ev.visible_range.start <= 2 {
            // 回顶钮无事件字段:近顶区(前 2 行)逐 tick notify,渲染期
            // 读权威 `trajectory_at_top()`;远离顶部时它恒 false,免 notify
            cx.notify();
        }
    }

    /// 台账在顶(首行入视口;渲染期权威读)
    pub fn trajectory_at_top(&self) -> bool {
        let top = self.trajectory.trajectory_list.logical_scroll_top();
        top.item_ix == 0 && f32::from(top.offset_in_item) <= 48.
    }

    /// 台账在底(镜像聊天列 `at_bottom` 判据:跟随态或距底 ≤24px)
    pub fn trajectory_at_bottom(&self) -> bool {
        let list = &self.trajectory.trajectory_list;
        if list.logical_scroll_top().item_ix >= list.item_count() {
            return true; // 钉底跟随态
        }
        let max = f32::from(list.max_offset_for_scrollbar().y);
        let cur = f32::from(-list.scroll_px_offset_for_scrollbar().y);
        max - cur <= 24.
    }

    /// 轨迹列表跳到顶部(已载首行;置脏 + 置信,渲染期 flush 在
    /// `has_older` 时自动续拉更早页,前插锚定由 splice 自持)
    pub fn jump_trajectory_top(&mut self, cx: &mut Context<Self>) {
        self.trajectory.trajectory_scroll_dirty = true;
        self.trajectory.scroll_pos_known = true;
        self.trajectory
            .trajectory_list
            .scroll_to(gpui_kit::ListOffset {
                item_ix: 0,
                offset_in_item: gpui_kit::px(0.),
            });
        cx.notify();
    }

    /// 轨迹列表跳到底部(最新记录)
    pub fn jump_trajectory_bottom(&mut self, cx: &mut Context<Self>) {
        self.trajectory
            .trajectory_list
            .scroll_to(gpui_kit::ListOffset {
                item_ix: usize::MAX,
                offset_in_item: gpui_kit::px(0.),
            });
        cx.notify();
    }

    /// 渲染期冲洗:滚动副作用收尾(跟随滚底已由 list 的 Tail 模式托管,
    /// 这里只处理需要**回读 `ListState`** 的判断——渲染期不在 list 借用内,
    /// 安全)。前插锚定由 list 的 `splice` 自持
    pub fn flush_trajectory_scroll(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.trajectory.trajectory_scroll_dirty)
            && self.trajectory.scroll_pos_known
        {
            // 首行入视口即「加载更早」。位置只在 reset 之后滚动过时才可信——
            // 否则 Top 对齐把「位置被清」读成 item_ix 0,会误判在顶部并形成
            // 翻页级联(见 `scroll_pos_known`)
            let top = self.trajectory.trajectory_list.logical_scroll_top();
            if top.item_ix == 0
                && f32::from(top.offset_in_item) <= 48.
                && self.trajectory.trajectory.has_older
            {
                self.load_earlier_trajectory(cx);
            }
        }
        self.trajectory.trajectory_rendered_version = self.trajectory.trajectory_version;
    }

    /// 选中台账记录(再点同记录 = 取消;开检查器并恢复最近 tab)
    pub fn select_trajectory_record(&mut self, index: u64, cx: &mut Context<Self>) {
        self.trajectory.inspector =
            if self.trajectory.inspector == Some(InspectTarget::Record(index)) {
                None
            } else {
                Some(InspectTarget::Record(index))
            };
        self.trajectory.inspector_tab = None;
        self.trajectory.inspector_raw_thinking = false;
        cx.notify();
    }

    /// 选中请求(Request #N 圆点;开检查器)
    pub fn select_trajectory_request(&mut self, number: u64, cx: &mut Context<Self>) {
        self.trajectory.inspector = Some(InspectTarget::Request(number));
        self.trajectory.inspector_tab = None;
        cx.notify();
    }

    /// 关闭检查器(× / 点表空白)
    pub fn close_inspector(&mut self, cx: &mut Context<Self>) {
        self.trajectory.inspector = None;
        self.trajectory.inspector_tab = None;
        cx.notify();
    }

    /// 切检查器 tab(记忆最近访问,跨实体恢复)
    pub fn set_inspector_tab(&mut self, tab: &'static str, cx: &mut Context<Self>) {
        self.trajectory.inspector_tab = Some(tab);
        self.trajectory.inspector_last_tab = tab;
        cx.notify();
    }

    /// Duration 切换(时间线耗时投影;模式切换清时间线选区)
    pub fn toggle_trajectory_duration(&mut self, cx: &mut Context<Self>) {
        self.trajectory.trajectory_duration = !self.trajectory.trajectory_duration;
        self.trajectory.timeline_selection = None;
        cx.notify();
    }

    /// Turns 全局折叠(工具栏;影响 turn_start 非首条之外整轮)
    pub fn toggle_all_turns(&mut self, cx: &mut Context<Self>) {
        self.trajectory.all_turns_collapsed = !self.trajectory.all_turns_collapsed;
        self.trajectory.collapsed_turns.clear();
        self.trajectory.collapse_ver += 1;
        cx.notify();
    }

    /// 单 turn 折叠切换(摘要行/双击 turn 首)
    pub fn toggle_turn(&mut self, turn: u64, cx: &mut Context<Self>) {
        if !self.trajectory.collapsed_turns.insert(turn) {
            self.trajectory.collapsed_turns.remove(&turn);
        }
        self.trajectory.collapse_ver += 1;
        cx.notify();
    }

    /// Calls 全局折叠(工具栏;assistant 连续工具调用)
    pub fn toggle_all_calls(&mut self, cx: &mut Context<Self>) {
        self.trajectory.all_calls_collapsed = !self.trajectory.all_calls_collapsed;
        self.trajectory.collapsed_calls.clear();
        self.trajectory.collapse_ver += 1;
        cx.notify();
    }

    /// 单 assistant 调用折叠切换
    pub fn toggle_call(&mut self, message_index: u64, cx: &mut Context<Self>) {
        if !self.trajectory.collapsed_calls.insert(message_index) {
            self.trajectory.collapsed_calls.remove(&message_index);
        }
        self.trajectory.collapse_ver += 1;
        cx.notify();
    }

    /// 提交/清空时间线选区(None = 清;右键/Esc/双击)
    pub fn set_timeline_selection(&mut self, sel: Option<(f64, f64)>, cx: &mut Context<Self>) {
        self.trajectory.timeline_selection = sel.map(|(a, b)| (a.min(b), a.max(b)));
        cx.notify();
    }

    /// 时间线视口(缩放;None = 全览)
    pub fn set_timeline_viewport(&mut self, vp: Option<(f64, f64)>, cx: &mut Context<Self>) {
        self.trajectory.timeline_viewport = vp.map(|(a, b)| (a.min(b), a.max(b)));
        cx.notify();
    }

    /// 时间线拖拽开始(锚点域分数;draft 清空,move 时填充)
    pub fn begin_timeline_drag(&mut self, anchor_frac: f64, cx: &mut Context<Self>) {
        self.trajectory.timeline_drag = Some(anchor_frac);
        self.trajectory.timeline_draft = None;
        cx.notify();
    }

    /// 时间线拖拽移动(更新草稿选区)
    pub fn move_timeline_drag(&mut self, cur_frac: f64, cx: &mut Context<Self>) {
        if let Some(anchor) = self.trajectory.timeline_drag {
            self.trajectory.timeline_draft = Some((anchor.min(cur_frac), anchor.max(cur_frac)));
            cx.notify();
        }
    }

    /// 时间线拖拽收尾(清拖拽态;选区提交由调用方决定)
    pub fn clear_timeline_drag(&mut self, cx: &mut Context<Self>) {
        self.trajectory.timeline_drag = None;
        self.trajectory.timeline_draft = None;
        cx.notify();
    }

    /// 检查器拖宽开始(锚点 = 光标 x + 当时宽度)
    pub fn inspector_resize_begin(&mut self, cursor_x: f32, cx: &mut Context<Self>) {
        self.trajectory.inspector_resize_anchor = Some((cursor_x, self.trajectory.inspector_width));
        cx.notify();
    }

    /// 检查器拖宽移动(clamp 320..720)
    pub fn inspector_resize_move(&mut self, cursor_x: f32, cx: &mut Context<Self>) {
        if let Some((anchor_x, anchor_w)) = self.trajectory.inspector_resize_anchor {
            self.trajectory.inspector_width = (anchor_w - (cursor_x - anchor_x)).clamp(320., 720.);
            cx.notify();
        }
    }

    /// 检查器拖宽结束
    pub fn inspector_resize_end(&mut self, cx: &mut Context<Self>) {
        self.trajectory.inspector_resize_anchor = None;
        cx.notify();
    }

    /// Raw tab 的 Thinking 折叠切换
    pub fn toggle_inspector_thinking(&mut self, cx: &mut Context<Self>) {
        self.trajectory.inspector_raw_thinking = !self.trajectory.inspector_raw_thinking;
        cx.notify();
    }

    /// Tools 页工具目录卡展开/折叠
    pub fn toggle_inspector_tool(&mut self, name: &str, cx: &mut Context<Self>) {
        if !self
            .trajectory
            .expanded_inspector_tools
            .insert(name.to_string())
        {
            self.trajectory.expanded_inspector_tools.remove(name);
        }
        self.trajectory.expanded_tools_ver += 1;
        cx.notify();
    }

    /// JSON 树节点展开/折叠(子级默认折叠,
    /// 集合存「已展开」路径)
    pub fn toggle_json_node(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.trajectory.json_expanded.insert(key.to_string()) {
            self.trajectory.json_expanded.remove(key);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod window_tests {
    use super::{TRAJECTORY_MAX_RETAINED, trim_records};
    use liuma_core::trajectory::TrajectoryRecord;

    /// 只填测试关心的字段(结构无 Default;逐字段列全)
    fn recs(n: usize) -> Vec<TrajectoryRecord> {
        (1..=n)
            .map(|i| TrajectoryRecord {
                index: i as u64,
                seq: i as u64,
                kind: "tool".into(),
                turn: Some(1),
                group: "Step 1".into(),
                turn_start: false,
                text: format!("记录 {i}"),
                result: None,
                is_error: false,
                time_seconds: None,
                started_at: None,
                request_number: None,
                input: None,
                output: None,
                think: None,
                ttft_ms: None,
                payload: None,
                output_detail: None,
                thinking_detail: None,
                system_prompt: None,
                tools_catalog: None,
                schema_detail: None,
                source: None,
                decision: None,
                fold: None,
            })
            .collect()
    }

    /// 窗口上限:只丢最旧、保留最近 N 条,且丢的正是「加载更早」那批
    /// (`has_older` 由已载最左 index > 1 推导,故语义自洽)
    #[test]
    fn trims_oldest_beyond_cap() {
        let mut r = recs(TRAJECTORY_MAX_RETAINED + 7);
        let dropped = trim_records(&mut r);
        assert_eq!(dropped, 7);
        assert_eq!(r.len(), TRAJECTORY_MAX_RETAINED);
        assert_eq!(r.first().map(|x| x.index), Some(8));
        assert_eq!(
            r.last().map(|x| x.index),
            Some((TRAJECTORY_MAX_RETAINED + 7) as u64)
        );
        // 未超上限不动
        assert_eq!(trim_records(&mut r), 0);
        assert_eq!(r.len(), TRAJECTORY_MAX_RETAINED);
    }
}
