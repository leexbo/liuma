//! 终端会话状态与线程边界:PTY + alacritty `Term` 的所有权模型。
//!
//! 线程划分(UI 线程独占 / tokio 阻塞池):
//! - **gpui 线程独占**:`Term`、`Processor`(皆非 Send 约束面)、按键
//!   编码、scroll_display、term.resize——无锁单线程所有权;
//! - **tokio 阻塞池**:PTY 读任务(`start_read_chunks`)/写任务
//!   (`start_write_chunks`),经无界通道与 UI 互通;kill 后读端 EOF
//!   自退、writer drop 后写端自退,无需显式 abort;
//! - **PTY spawn/拆线**:`HostBridge::call_blocking`(blocking 池,
//!   不占 worker);结果经 oneshot 回 UI 装配会话。
//!
//! 生命周期:标签首开懒 spawn,一路标签 = 一路 shell(多 tab 并存,
//! 各自独立 PTY/网格/泵);关闭标签即 kill;切换激活工作区时 cwd
//! 失配的会话统一杀(下次打开重 spawn 到新根)。会话长驻 = 标签存活
//! 期,切到其他标签状态保留(泵照喂,仅不触发重绘)。

use std::cell::RefCell;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::rc::Rc;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::search::{RegexIter, RegexSearch};
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext as _, Context, Entity, FocusHandle};

use crate::shell::panel::TerminalTabId;
use crate::shell::store::AppStore;

/// 滚回上限(行)。网格按 `rows + SCROLLBACK` 预分配(Cell ≈ 24B):
/// 1000 行 × 100 列 ≈ 2.4MB/会话,v1 取 VS Code 默认档
const SCROLLBACK: usize = 1000;

/// 连击判定窗口(双击取词/三击取行)
const CLICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// 默认行列(首帧 bounds 未测得时的 spawn 尺寸;后续由 resize 校正)
const DEFAULT_COLS: usize = 80;
const DEFAULT_ROWS: usize = 24;

/// 网格尺寸(`grid::Dimensions`;total_lines = 可见行 + 滚回)
pub(crate) struct TermDims {
    pub cols: usize,
    pub rows: usize,
}

impl Dimensions for TermDims {
    fn columns(&self) -> usize {
        self.cols
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn total_lines(&self) -> usize {
        self.rows + SCROLLBACK
    }
}

/// alacritty 事件代理:VT 查询应答(DA/DSR 等)必须回写 PTY,否则
/// vim/htop 的能力探测会挂等。Title/ResetTitle 写入共享缓冲(代理在
/// feed 期被 Term 同步调用,泵侧排空后落标签标题);其余事件忽略
pub(crate) struct PtyEventProxy {
    writer: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    titles: Rc<RefCell<Vec<Option<String>>>>,
}

impl EventListener for PtyEventProxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => {
                let _ = self.writer.send(text.into_bytes());
            }
            Event::Title(title) => self.titles.borrow_mut().push(Some(title)),
            Event::ResetTitle => self.titles.borrow_mut().push(None),
            _ => {}
        }
    }
}

/// 一条长驻终端会话(UI 线程所有权;不可跨线程共享)
pub(crate) struct TerminalSession {
    /// VT 状态机(网格/滚回/alt-screen)
    pub term: Term<PtyEventProxy>,
    /// ANSI 字节解析器(驱动 `term`)
    pub parser: Processor,
    /// PTY 会话(kill/resize/try_wait 在 UI 线程直调)
    pub pty: liuma_sandbox::pty::PtySession,
    /// 键入 + 查询应答的统一写入口(发送端 clone 自由、send 永不阻塞)
    pub writer: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    /// 会话归属工作区根(失配判据)
    pub cwd: PathBuf,
    /// Title/ResetTitle 事件缓冲(与 Term 内代理共享;feed 后由泵排空;
    /// `None` = ResetTitle,标题回落 cwd 目录名)
    pub titles: Rc<RefCell<Vec<Option<String>>>>,
    /// 子进程已退出(EOF 泵侧置位;退出码见 `exit_code`)
    pub exited: bool,
    pub exit_code: Option<i32>,
}

impl TerminalSession {
    /// 喂输出字节进 VT 状态机(UI 线程)
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    /// 排空 Title 事件缓冲(OSC 标题序列 → 标签文案)
    pub fn drain_titles(&mut self) -> Vec<Option<String>> {
        std::mem::take(&mut *self.titles.borrow_mut())
    }

    /// 当前模式(按键编码与滚轮语义查询)
    pub fn mode(&self) -> &TermMode {
        self.term.mode()
    }

    /// 可见区文本(逐行拼接;测试断言/诊断用,生产不调用)
    #[cfg(test)]
    pub(crate) fn visible_text(&self) -> Vec<String> {
        let content = self.term.renderable_content();
        let offset = content.display_offset as i32;
        super::palette::bucket_lines(
            content.display_iter,
            offset,
            self.term.grid().screen_lines(),
        )
        .iter()
        .map(|cells| cells.iter().map(|c| c.c).collect::<String>())
        .collect()
    }
}

/// 终端内搜索态(按标签隔离):查询串 + 全部命中(网格坐标)+ 当前
/// 命中(以其起点定位,重扫后仍可对上)。命中在导航/改词时全量重扫,
/// 输出增长导致的漂移随之自愈
pub(crate) struct TerminalSearch {
    pub query: String,
    pub matches: Vec<RangeInclusive<Point>>,
    pub current: Option<Point>,
}

/// 一路终端标签:面板 `PanelTab::Terminal(id)` 的状态载体。焦点/泵/
/// 代次随标签走(会话重建复用焦点句柄),标题为 `None` 时回落 cwd 目录名
pub(crate) struct TerminalTab {
    pub id: TerminalTabId,
    /// 长驻会话(None = 未开/已关/重启在途)
    pub session: Option<TerminalSession>,
    /// 会话焦点句柄(标签激活/点击聚焦;按键仅在 focused 时编码)
    pub focus: FocusHandle,
    /// 标签激活置位,渲染期消费(消费即清,避免抢走后续焦点)
    pub wants_focus: bool,
    /// spawn 在途防重入(`call_blocking` 往返期间禁重复拉起)
    pub spawning: bool,
    /// 会话代次(spawn/kill 各 +1):输出泵据此识别自己服务的是否仍是
    /// 当前会话,失配即自退(替代显式 abort,同 lineage_tick 手法)
    pub generation: u64,
    /// OSC Title 文本(None = 回落 cwd 目录名;ResetTitle 清回 None)
    pub title: Option<String>,
    /// 左键拖选进行中(窗口级 move/up 消费;单击起拖、抬起结算)
    pub drag: bool,
    /// 终端内搜索态(None = 搜索条关闭)
    pub search: Option<TerminalSearch>,
    /// 上次左键按下((时刻, 列, 行, 连击序);双击取词/三击取行判定)
    pub last_click: Option<(std::time::Instant, usize, i32, u8)>,
    /// 输出泵任务句柄(存住才在跑:Task drop = 取消)
    pub pump: Option<gpui_kit::Task<()>>,
}

/// 终端切片状态(多标签;网格目标尺寸全标签共享——同一面板几何)
pub(crate) struct TerminalStore {
    /// 标签表(序无关;面板标签序由 panel_tabs 承担)
    pub tabs: Vec<TerminalTab>,
    /// id 分配器(自 1 起;0 = `TerminalTabId::NEW` 哨兵)
    pub next_id: u64,
    /// 目标网格尺寸(与活跃会话 term/PTY 同步;resize 变化检测基线)
    pub cols: usize,
    pub rows: usize,
    /// 实测单元格尺寸 (宽, 高)(等宽字体一次测得缓存;None = 未测)
    pub cell: Option<(f32, f32)>,
    /// 右键「复制」暂存(菜单弹出时抓取,App 级动作消费——分发期
    /// 无 window 回读实时选区,同聊天选区手法)
    pub pending_copy: Option<String>,
    /// 搜索条输入(全局一只,重开保留上次查询;订阅 Change/Enter)
    pub search_input: Option<Entity<InputState>>,
}

impl TerminalStore {
    pub(crate) fn new() -> Self {
        Self {
            tabs: Vec::new(),
            next_id: 1,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            cell: None,
            pending_copy: None,
            search_input: None,
        }
    }

    pub(crate) fn tab(&self, id: TerminalTabId) -> Option<&TerminalTab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub(crate) fn tab_mut(&mut self, id: TerminalTabId) -> Option<&mut TerminalTab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// 分配真实标签 id(菜单哨兵经 open_panel_tab 兑换)
    pub(crate) fn alloc_id(&mut self) -> TerminalTabId {
        let id = TerminalTabId(self.next_id);
        self.next_id += 1;
        id
    }

    /// 标签记录建档(无则建:焦点句柄在此分配,会话稍后懒 spawn)
    fn ensure_record(&mut self, id: TerminalTabId, cx: &Context<AppStore>) {
        if self.tab(id).is_none() {
            self.tabs.push(TerminalTab {
                id,
                session: None,
                focus: cx.focus_handle(),
                wants_focus: false,
                spawning: false,
                generation: 0,
                title: None,
                drag: false,
                search: None,
                last_click: None,
                pump: None,
            });
        }
    }
}

impl AppStore {
    /// 激活标签的终端 id(未在终端页 = None)
    pub(crate) fn terminal_active_id(&self) -> Option<TerminalTabId> {
        match self.panel_active_tab {
            Some(crate::shell::panel::PanelTab::Terminal(id)) => Some(id),
            _ => None,
        }
    }

    /// 激活终端的会话(测试与诊断入口)
    #[cfg(test)]
    pub(crate) fn terminal_active_session(&self) -> Option<&TerminalSession> {
        let id = self.terminal_active_id()?;
        self.terminal.tab(id)?.session.as_ref()
    }

    /// 标签切入/打开:cwd 失配(首次/换工作区)先杀旧会话,再懒 spawn。
    /// 同工作区内切走再切回**不重启**(长驻语义)
    pub(crate) fn terminal_ensure(&mut self, id: TerminalTabId, cx: &mut Context<Self>) {
        let cwd_ok = self
            .terminal
            .tab(id)
            .and_then(|t| t.session.as_ref())
            .is_some_and(|s| Some(&s.cwd) == self.current_workspace_dir().as_ref());
        if !cwd_ok {
            self.terminal_kill(id, cx);
        }
        let (spawning, has_session) = self
            .terminal
            .tab(id)
            .map_or((false, false), |t| (t.spawning, t.session.is_some()));
        if !has_session && !spawning {
            self.terminal_spawn(id, cx);
        }
        self.terminal_sync_size(id, cx);
        if let Some(tab) = self.terminal.tab_mut(id) {
            tab.wants_focus = true;
        }
    }

    /// 激活工作区/会话变更(open_session 与 select_workspace 尾部挂)。
    /// 终端归属工作区而非会话:cwd 失配即杀,**杀时机在此、重 spawn
    /// 后置到下次打开/激活**(terminal_ensure),避免切会话顺手拉 shell
    pub(crate) fn terminal_on_workspace_change(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<TerminalTabId> = self.terminal.tabs.iter().map(|t| t.id).collect();
        for id in ids {
            let cwd_ok = self
                .terminal
                .tab(id)
                .and_then(|t| t.session.as_ref())
                .is_some_and(|s| Some(&s.cwd) == self.current_workspace_dir().as_ref());
            if !cwd_ok {
                self.terminal_kill(id, cx);
            }
        }
    }

    /// 关闭标签/换工作区的收尾:杀 PTY + 会话与泵任务一并撤除
    /// (标签记录保留:焦点句柄复用,重开走 terminal_spawn)
    pub(crate) fn terminal_kill(&mut self, id: TerminalTabId, cx: &mut Context<Self>) {
        if let Some(tab) = self.terminal.tab_mut(id) {
            if let Some(mut session) = tab.session.take() {
                session.pty.kill();
            }
            tab.spawning = false;
            tab.generation += 1;
            tab.pump.take();
        }
        cx.notify();
    }

    /// 标签整档撤除(关标签走这里:杀会话 + 摘记录)
    pub(crate) fn terminal_remove(&mut self, id: TerminalTabId, cx: &mut Context<Self>) {
        self.terminal_kill(id, cx);
        self.terminal.tabs.retain(|t| t.id != id);
    }

    /// 拉起会话(spawn 在 blocking 池;装配回 UI 线程)。shell 选择:
    /// `$SHELL` → /bin/zsh → /bin/bash;macOS 走登录 shell(`-l`,
    /// PATH/别名与用户终端一致)。终端是用户自己的 shell,不过沙箱
    pub(crate) fn terminal_spawn(&mut self, id: TerminalTabId, cx: &mut Context<Self>) {
        self.terminal.ensure_record(id, cx);
        let generation = match self.terminal.tab_mut(id) {
            Some(tab) => {
                tab.spawning = true;
                tab.generation += 1;
                tab.generation
            }
            None => return,
        };
        let (cols, rows) = (self.terminal.cols, self.terminal.rows);
        let cwd = self
            .current_workspace_dir()
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_default();
        let shell = std::env::var("SHELL").unwrap_or_else(|_| {
            if cfg!(target_os = "macos") {
                "/bin/zsh".to_string()
            } else {
                "/bin/bash".to_string()
            }
        });
        let mut args: Vec<String> = Vec::new();
        if cfg!(target_os = "macos") {
            args.push("-l".to_string());
        }
        let program = shell;
        let cwd_clone = cwd.clone();
        let rx = self.bridge.call_blocking(move || {
            let size = portable_pty::PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            };
            let env = [
                ("TERM", "xterm-256color"),
                ("TERM_PROGRAM", "liuma"),
                ("COLORTERM", "truecolor"),
            ];
            let session = liuma_sandbox::pty::spawn_pty_sized(
                &program,
                &args,
                Some(&cwd_clone),
                None,
                size,
                &env,
            )?;
            let (writer, _write_task) = session.start_write_chunks()?;
            let (reader, _read_task) = session.start_read_chunks()?;
            anyhow::Ok((session, writer, reader))
        });
        let store = cx.entity().clone();
        // 装配任务一次性:detach 后台跑到收尾(Task drop = 取消,不能裸丢);
        // 输出泵在其内另起并存进标签记录(kill 换代后自退)
        cx.spawn(async move |_this, cx| {
            let assembled = rx.await.ok().and_then(|result| result.ok());
            let Some((pty, writer, reader)) = assembled else {
                store.update(cx, |s, cx| {
                    if let Some(tab) = s.terminal.tab_mut(id) {
                        tab.spawning = false;
                    }
                    cx.notify();
                });
                return;
            };
            // 装配 + 起泵;spawn 往返期间代次已变(被 kill/重启)则静默
            // 丢弃:reader drop 后阻塞读任务随 EOF 自退
            let pump_store = store.clone();
            store.update(cx, |s, cx| {
                let titles = Rc::new(RefCell::new(Vec::new()));
                let dims = TermDims {
                    cols: s.terminal.cols,
                    rows: s.terminal.rows,
                };
                let term = Term::new(
                    Config {
                        scrolling_history: SCROLLBACK,
                        ..Config::default()
                    },
                    &dims,
                    PtyEventProxy {
                        writer: writer.clone(),
                        titles: titles.clone(),
                    },
                );
                let session = TerminalSession {
                    term,
                    parser: Processor::new(),
                    pty,
                    writer,
                    cwd,
                    titles,
                    exited: false,
                    exit_code: None,
                };
                let Some(tab) = s.terminal.tab_mut(id) else {
                    return;
                };
                tab.spawning = false;
                if tab.generation != generation || tab.session.is_some() {
                    return;
                }
                tab.session = Some(session);
                tab.wants_focus = true;
                tab.pump = Some(cx.spawn(async move |_pump, cx| {
                    s_terminal_pump(&pump_store, id, reader, generation, cx).await;
                }));
                cx.notify();
            });
        })
        .detach();
    }

    /// 应用新网格尺寸(渲染期变化检测调用;未变化零开销)。term 与
    /// PTY 双侧同步:PTY resize 即内核 TIOCSWINSZ,前台进程组收 SIGWINCH
    pub(crate) fn terminal_apply_size(&mut self, cols: usize, rows: usize, cx: &mut Context<Self>) {
        let cols = cols.clamp(2, 500);
        let rows = rows.clamp(2, 200);
        if self.terminal.cols == cols && self.terminal.rows == rows {
            return;
        }
        self.terminal.cols = cols;
        self.terminal.rows = rows;
        if let Some(id) = self.terminal_active_id() {
            self.terminal_sync_size(id, cx);
        }
        cx.notify();
    }

    /// 会话网格对齐目标尺寸(标签切换后补齐:后台标签停驻期面板被
    /// 拖宽,切回时 target 已变而其 term 还在旧尺寸)
    fn terminal_sync_size(&mut self, id: TerminalTabId, cx: &mut Context<Self>) {
        let (cols, rows) = (self.terminal.cols, self.terminal.rows);
        let Some(tab) = self.terminal.tab_mut(id) else {
            return;
        };
        let Some(session) = tab.session.as_mut() else {
            return;
        };
        if session.term.grid().columns() == cols && session.term.grid().screen_lines() == rows {
            return;
        }
        let dims = TermDims { cols, rows };
        session.term.resize(dims);
        let _ = session.pty.resize(portable_pty::PtySize {
            rows: rows as u16,
            cols: cols as u16,
            pixel_width: 0,
            pixel_height: 0,
        });
        cx.notify();
    }

    /// 编码按键写入 PTY(命中才消费事件;None = 放行给 UI)
    pub(crate) fn terminal_write_key(
        &mut self,
        id: TerminalTabId,
        keystroke: &gpui_kit::Keystroke,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self.terminal.tab_mut(id) else {
            return false;
        };
        let Some(session) = tab.session.as_mut() else {
            return false;
        };
        if session.exited {
            return false;
        }
        let Some(bytes) = super::input::encode_key(keystroke, session.mode()) else {
            return false;
        };
        let _ = session.writer.send(bytes);
        // 键入即回底(alacritty 同款):滚回查看时敲键回到实时视图
        session
            .term
            .scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        cx.notify();
        true
    }

    /// 粘贴文本(bracketed paste 按 PTY 模式包装);键入面同款回底
    pub(crate) fn terminal_write_paste(
        &mut self,
        id: TerminalTabId,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.terminal.tab_mut(id)
            && let Some(session) = tab.session.as_mut()
        {
            let bytes = super::input::encode_paste(text, session.mode());
            let _ = session.writer.send(bytes);
            session
                .term
                .scroll_display(alacritty_terminal::grid::Scroll::Bottom);
            cx.notify();
        }
    }

    /// 滚动显示区(Delta 正值 = 向历史方向;alt-screen 交给应用自理)
    pub(crate) fn terminal_scroll(
        &mut self,
        id: TerminalTabId,
        lines: i32,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.terminal.tab_mut(id)
            && let Some(session) = tab.session.as_mut()
        {
            session
                .term
                .scroll_display(alacritty_terminal::grid::Scroll::Delta(lines));
            cx.notify();
        }
    }

    /// 选区文本(无选区/空选区 = None;⌘C 与右键复制共用)
    pub(crate) fn terminal_selection_text(&self, id: TerminalTabId) -> Option<String> {
        let session = self.terminal.tab(id)?.session.as_ref()?;
        session
            .term
            .selection
            .as_ref()
            .filter(|selection| !selection.is_empty())?;
        session.term.selection_to_string()
    }

    /// 指针按下(左键):单击起拖选(替代旧选区),双击取词,三击取行。
    /// 坐标为视口格坐标,`side` = 半格规则(光标在格内右半 → Right)
    pub(crate) fn terminal_pointer_down(
        &mut self,
        id: TerminalTabId,
        col: usize,
        row: usize,
        side: Side,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.terminal.tab_mut(id) else {
            return;
        };
        let Some(session) = tab.session.as_mut() else {
            return;
        };
        if session.exited {
            return;
        }
        let now = std::time::Instant::now();
        // 连击判定:半格距内 CLICK_INTERVAL 内连点;三击后重置连击
        let count = match tab.last_click {
            Some((at, lcol, lrow, n))
                if now.duration_since(at) < CLICK_INTERVAL
                    && lcol.abs_diff(col) <= 1
                    && (lrow - row as i32).abs() <= 1
                    && n < 3 =>
            {
                n + 1
            }
            _ => 1,
        };
        let offset = session.term.grid().display_offset() as i32;
        let point = Point::new(Line(row as i32 - offset), Column(col));
        tab.drag = false;
        match count {
            1 => {
                session.term.selection = Some(Selection::new(SelectionType::Simple, point, side));
                tab.drag = true;
            }
            2 => {
                // 取词:语义边界(`semantic_escape_chars` 内建词表)
                let left = session.term.semantic_search_left(point);
                let right = session.term.semantic_search_right(point);
                let mut selection = Selection::new(SelectionType::Simple, left, Side::Left);
                selection.update(right, Side::Right);
                session.term.selection = Some(selection);
            }
            _ => {
                // 取行:Lines 选区覆盖整行
                session.term.selection = Some(Selection::new(SelectionType::Lines, point, side));
            }
        }
        tab.last_click = match count {
            3 => None,
            n => Some((now, col, row as i32, n)),
        };
        cx.notify();
    }

    /// 拖选移动(窗口级 move 事件;非拖选态静默)
    pub(crate) fn terminal_pointer_drag(
        &mut self,
        id: TerminalTabId,
        col: usize,
        row: usize,
        side: Side,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.terminal.tab_mut(id) else {
            return;
        };
        if !tab.drag {
            return;
        }
        let Some(session) = tab.session.as_mut() else {
            return;
        };
        let offset = session.term.grid().display_offset() as i32;
        let point = Point::new(Line(row as i32 - offset), Column(col));
        if let Some(selection) = session.term.selection.as_mut() {
            selection.update(point, side);
            cx.notify();
        }
    }

    /// 拖选收尾(窗口级 up):单格范围内抬起 = 原单击,清选区
    pub(crate) fn terminal_pointer_up(&mut self, id: TerminalTabId, cx: &mut Context<Self>) {
        let Some(tab) = self.terminal.tab_mut(id) else {
            return;
        };
        if !tab.drag {
            return;
        }
        tab.drag = false;
        let Some(session) = tab.session.as_mut() else {
            return;
        };
        let single = session
            .term
            .selection
            .as_ref()
            .and_then(|selection| selection.to_range(&session.term))
            .is_some_and(|range| range.start == range.end);
        if single {
            session.term.selection = None;
        }
        cx.notify();
    }

    /// 打开搜索条(cmd-f):懒建输入(全局一只,重开保留查询)并聚焦,
    /// 标签搜索态就位(有查询则立即重扫)
    pub(crate) fn terminal_search_open(
        &mut self,
        id: TerminalTabId,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        if self.terminal.search_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx).placeholder(crate::kits::i18n::t!("terminal.search_ph"))
            });
            cx.subscribe(&input, |this, _i, event: &InputEvent, cx| {
                let Some(id) = this.terminal_active_id() else {
                    return;
                };
                match event {
                    InputEvent::Change => {
                        let query = this
                            .terminal
                            .search_input
                            .as_ref()
                            .map(|i| i.read(cx).value().to_string())
                            .unwrap_or_default();
                        this.terminal_search_query(id, &query, cx);
                    }
                    InputEvent::PressEnter { shift, .. } => {
                        if *shift {
                            this.terminal_search_prev(id, cx);
                        } else {
                            this.terminal_search_next(id, cx);
                        }
                    }
                    _ => {}
                }
            })
            .detach();
            self.terminal.search_input = Some(input);
        }
        if let Some(input) = &self.terminal.search_input {
            input.update(cx, |i, cx| i.focus(window, cx));
        }
        // 标签态就位:沿用输入框现值(重开 = 上次查询立即重扫)
        let query = self
            .terminal
            .search_input
            .as_ref()
            .map(|i| i.read(cx).value().to_string())
            .unwrap_or_default();
        self.terminal.ensure_record(id, cx);
        if let Some(tab) = self.terminal.tab_mut(id) {
            let changed = tab
                .search
                .as_ref()
                .is_none_or(|search| search.query != query);
            let search = tab.search.get_or_insert(TerminalSearch {
                query: String::new(),
                matches: Vec::new(),
                current: None,
            });
            search.query = query.clone();
            if changed && !query.is_empty() {
                self.terminal_search_scan(id, cx);
            }
        }
        cx.notify();
    }

    /// 关闭搜索条(✕):清标签搜索态,焦点还给终端
    pub(crate) fn terminal_search_close(
        &mut self,
        id: TerminalTabId,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.terminal.tab_mut(id) {
            tab.search = None;
        }
        if let Some(handle) = self.terminal.tab(id).map(|t| t.focus.clone()) {
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    /// 查询词更新(输入 Change;空词清命中)
    pub(crate) fn terminal_search_query(
        &mut self,
        id: TerminalTabId,
        query: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.terminal.tab_mut(id) else {
            return;
        };
        let Some(search) = tab.search.as_mut() else {
            return;
        };
        search.query = query.to_string();
        self.terminal_search_scan(id, cx);
    }

    /// 下一个命中(Enter;无当前 = 首个,尾部回卷)
    pub(crate) fn terminal_search_next(&mut self, id: TerminalTabId, cx: &mut Context<Self>) {
        self.terminal_search_step(id, 1, cx);
    }

    /// 上一个命中(Shift-Enter;无当前 = 末个,头部回卷)
    pub(crate) fn terminal_search_prev(&mut self, id: TerminalTabId, cx: &mut Context<Self>) {
        self.terminal_search_step(id, -1, cx);
    }

    /// 命中导航步进:全量重扫(输出增长漂移自愈),按方向取相邻命中,
    /// 滚动定位并渲染
    fn terminal_search_step(&mut self, id: TerminalTabId, dir: i32, cx: &mut Context<Self>) {
        self.terminal_search_scan(id, cx);
        let Some(tab) = self.terminal.tab_mut(id) else {
            return;
        };
        let Some(search) = tab.search.as_mut() else {
            return;
        };
        if search.matches.is_empty() {
            return;
        }
        let at = search
            .current
            .and_then(|point| search.matches.iter().position(|m| m.start() == &point));
        let len = search.matches.len() as i32;
        let next = match at {
            Some(i) => (i as i32 + dir).rem_euclid(len) as usize,
            None if dir > 0 => 0,
            None => len as usize - 1,
        };
        let point = *search.matches[next].start();
        search.current = Some(point);
        if let Some(session) = tab.session.as_mut() {
            session.term.scroll_to_point(point);
        }
        cx.notify();
    }

    /// 全量重扫当前查询(命中表 + 当前命中定位滚动)。会话/搜索态缺失
    /// 或空词时只清命中
    fn terminal_search_scan(&mut self, id: TerminalTabId, cx: &mut Context<Self>) {
        let Some(tab) = self.terminal.tab_mut(id) else {
            return;
        };
        let Some(search) = tab.search.as_mut() else {
            return;
        };
        let query = search.query.clone();
        let current = search.current;
        let Some(session) = tab.session.as_ref() else {
            return;
        };
        let matches = if query.is_empty() {
            Vec::new()
        } else {
            scan_matches(&session.term, &query)
        };
        let Some(search) = tab.search.as_mut() else {
            return;
        };
        search.matches = matches;
        // 当前命中若仍在新命中表里则保持,否则落首命中
        if !search.matches.iter().any(|m| Some(*m.start()) == current) {
            search.current = search.matches.first().map(|m| *m.start());
        }
        if let Some(point) = search.current
            && let Some(session) = tab.session.as_mut()
        {
            session.term.scroll_to_point(point);
        }
        cx.notify();
    }
}

/// 输出泵:PTY 增量块 → VT 状态机,合帧触发重绘;EOF = 子进程退出。
/// 任务句柄存标签记录(kill 换代后泵自退);代次按标签隔离。标签
/// 不可见时照喂但不 notify(切回即最新,不空转窗口重绘)
async fn s_terminal_pump(
    store: &Entity<AppStore>,
    id: TerminalTabId,
    mut reader: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    generation: u64,
    cx: &mut gpui_kit::AsyncApp,
) {
    loop {
        // 阻塞等首块;None = 读端 EOF(子进程退出/会话被杀)
        let Some(chunk) = reader.recv().await else {
            break;
        };
        // 合帧:本轮已就绪的块一次吃完,减少 entity update 往返
        let mut batch = chunk;
        while let Ok(more) = reader.try_recv() {
            batch.extend_from_slice(&more);
        }
        store.update(cx, |s, cx| {
            let visible = s.panel_active_tab == Some(crate::shell::panel::PanelTab::Terminal(id));
            let Some(tab) = s.terminal.tab_mut(id) else {
                return;
            };
            if tab.generation != generation {
                return;
            }
            let Some(session) = tab.session.as_mut() else {
                return;
            };
            session.feed(&batch);
            // OSC 标题 → 标签文案(ResetTitle 回落 cwd 目录名);标题
            // 变化即使标签在后台也要刷新 pill(头部在面板开时恒渲染)
            let mut titled = false;
            for event in session.drain_titles() {
                tab.title = event;
                titled = true;
            }
            if visible || titled {
                cx.notify();
            }
        });
    }
    store.update(cx, |s, cx| {
        let visible = s.panel_active_tab == Some(crate::shell::panel::PanelTab::Terminal(id));
        let Some(tab) = s.terminal.tab_mut(id) else {
            return;
        };
        if tab.generation != generation {
            return;
        }
        let Some(session) = tab.session.as_mut() else {
            return;
        };
        session.exited = true;
        session.exit_code = session
            .pty
            .try_wait()
            .ok()
            .flatten()
            .and_then(|status| status.code);
        if visible {
            cx.notify();
        }
    });
}

/// 查询词 → 正则模式:按字面处理(元字符转义)+ 忽略大小写
fn search_pattern(query: &str) -> String {
    let mut pattern = String::with_capacity(query.len() * 2 + 4);
    pattern.push_str("(?i)");
    for ch in query.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(ch) {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern
}

/// 全网格命中扫描(含滚回;网格坐标 0 = 屏顶,滚回为**负行**)。
/// 正则 DFA,DFA 尺寸上限内的查询毫秒级
fn scan_matches(term: &Term<PtyEventProxy>, query: &str) -> Vec<RangeInclusive<Point>> {
    let Ok(mut regex) = RegexSearch::new(&search_pattern(query)) else {
        return Vec::new();
    };
    let screen_lines = term.screen_lines() as i32;
    let history = term.total_lines() as i32 - screen_lines;
    let start = Point::new(Line(-history), Column(0));
    let end = Point::new(
        Line(screen_lines - 1),
        Column(term.columns().saturating_sub(1)),
    );
    RegexIter::new(start, end, Direction::Right, term, &mut regex).collect()
}
