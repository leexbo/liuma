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
//! 生命周期:tab 首次打开/激活懒 spawn;关闭 tab 即 kill;切换激活
//! 工作区时若 cwd 失配同样 kill(下次打开重 spawn 到新根)。会话长驻
//! = tab 存活期,切到其他 tab 状态保留(泵照喂,仅不触发重绘)。

use std::path::PathBuf;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use gpui_kit::{Context, Entity, FocusHandle};

use crate::shell::store::AppStore;

/// 滚回上限(行)。网格按 `rows + SCROLLBACK` 预分配(Cell ≈ 24B):
/// 1000 行 × 100 列 ≈ 2.4MB/会话,v1 取 VS Code 默认档
const SCROLLBACK: usize = 1000;

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
/// vim/htop 的能力探测会挂等。其余事件 v1 忽略(Title 后续可接 tab 文案)
pub(crate) struct PtyEventProxy {
    writer: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

impl EventListener for PtyEventProxy {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(text) = event {
            let _ = self.writer.send(text.into_bytes());
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
    /// 子进程已退出(EOF 泵侧置位;退出码见 `exit_code`)
    pub exited: bool,
    pub exit_code: Option<i32>,
}

impl TerminalSession {
    /// 喂输出字节进 VT 状态机(UI 线程)
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
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

/// 终端切片状态
pub(crate) struct TerminalStore {
    /// 长驻会话(None = 未开/已关)
    pub session: Option<TerminalSession>,
    /// 当前生效网格尺寸(与 `term`/PTY 一致;resize 变化检测基线)
    pub cols: usize,
    pub rows: usize,
    /// 实测单元格尺寸 (宽, 高)(等宽字体一次测得缓存;None = 未测)
    pub cell: Option<(f32, f32)>,
    /// 会话焦点句柄(tab 激活/点击聚焦;按键仅在 focused 时编码)
    pub focus: FocusHandle,
    /// tab 激活置位,渲染期消费(消费即清,避免抢走后续焦点)
    pub wants_focus: bool,
    /// spawn 在途防重入(`call_blocking` 往返期间禁重复拉起)
    pub spawning: bool,
    /// 会话代次(spawn/kill 各 +1):输出泵据此识别自己服务的是否仍是
    /// 当前会话,失配即自退(替代显式 abort,同 lineage_tick 手法)
    pub generation: u64,
}

impl TerminalStore {
    pub(crate) fn new(focus: FocusHandle) -> Self {
        Self {
            session: None,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            cell: None,
            focus,
            wants_focus: false,
            spawning: false,
            generation: 0,
        }
    }
}

impl AppStore {
    /// 终端面板当前可见(激活标签 = 终端):输出泵触发重绘的门控判据
    pub(crate) fn terminal_visible(&self) -> bool {
        matches!(
            self.panel_active_tab,
            Some(crate::shell::panel::PanelTab::Terminal)
        )
    }

    /// 标签切入/打开:cwd 失配(首次/换工作区)先杀旧会话,再懒 spawn。
    /// 同工作区内切走再切回**不重启**(长驻语义)
    pub(crate) fn terminal_ensure(&mut self, cx: &mut Context<Self>) {
        let cwd_ok = self
            .terminal
            .session
            .as_ref()
            .is_some_and(|s| Some(&s.cwd) == self.current_workspace_dir().as_ref());
        if !cwd_ok {
            self.terminal_kill(cx);
        }
        if self.terminal.session.is_none() && !self.terminal.spawning {
            self.terminal_spawn(cx);
        }
        self.terminal.wants_focus = true;
    }

    /// 激活工作区/会话变更(open_session 与 select_workspace 尾部挂)。
    /// 终端归属工作区而非会话:cwd 失配即杀,**杀时机在此、重 spawn
    /// 后置到下次打开/激活**(terminal_ensure),避免切会话顺手拉 shell
    pub(crate) fn terminal_on_workspace_change(&mut self, cx: &mut Context<Self>) {
        let cwd_ok = self
            .terminal
            .session
            .as_ref()
            .is_some_and(|s| Some(&s.cwd) == self.current_workspace_dir().as_ref());
        if !cwd_ok {
            self.terminal_kill(cx);
        }
    }

    /// 关闭标签/换工作区的收尾:杀 PTY + 会话与泵任务一并撤除
    pub(crate) fn terminal_kill(&mut self, cx: &mut Context<Self>) {
        if let Some(mut session) = self.terminal.session.take() {
            session.pty.kill();
        }
        self.terminal.spawning = false;
        self.terminal.generation += 1;
        self.terminal_pump.take();
        cx.notify();
    }

    /// 拉起会话(spawn 在 blocking 池;装配回 UI 线程)。shell 选择:
    /// `$SHELL` → /bin/zsh → /bin/bash;macOS 走登录 shell(`-l`,
    /// PATH/别名与用户终端一致)。终端是用户自己的 shell,不过沙箱
    pub(crate) fn terminal_spawn(&mut self, cx: &mut Context<Self>) {
        self.terminal.spawning = true;
        self.terminal.generation += 1;
        let generation = self.terminal.generation;
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
        // 输出泵在其内另起并存句柄(kill 换代后自退)
        cx.spawn(async move |_this, cx| {
            let assembled = rx.await.ok().and_then(|result| result.ok());
            let Some((pty, writer, reader)) = assembled else {
                store.update(cx, |s, cx| {
                    s.terminal.spawning = false;
                    cx.notify();
                });
                return;
            };
            // 装配 + 起泵;spawn 往返期间代次已变(被 kill/重启)则静默
            // 丢弃:reader drop 后阻塞读任务随 EOF 自退
            let pump_store = store.clone();
            store.update(cx, |s, cx| {
                s.terminal.spawning = false;
                if s.terminal.generation != generation || s.terminal.session.is_some() {
                    return;
                }
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
                    },
                );
                s.terminal.session = Some(TerminalSession {
                    term,
                    parser: Processor::new(),
                    pty,
                    writer,
                    cwd,
                    exited: false,
                    exit_code: None,
                });
                s.terminal.wants_focus = true;
                s.terminal_pump = Some(cx.spawn(async move |_pump, cx| {
                    s_terminal_pump(&pump_store, reader, generation, cx).await;
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
        let Some(session) = self.terminal.session.as_mut() else {
            return;
        };
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
        keystroke: &gpui_kit::Keystroke,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(session) = self.terminal.session.as_mut() else {
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
    pub(crate) fn terminal_write_paste(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(session) = self.terminal.session.as_mut() {
            let bytes = super::input::encode_paste(text, session.mode());
            let _ = session.writer.send(bytes);
            session
                .term
                .scroll_display(alacritty_terminal::grid::Scroll::Bottom);
            cx.notify();
        }
    }

    /// 滚动显示区(Delta 正值 = 向历史方向;alt-screen 交给应用自理)
    pub(crate) fn terminal_scroll(&mut self, lines: i32, cx: &mut Context<Self>) {
        if let Some(session) = self.terminal.session.as_mut() {
            session
                .term
                .scroll_display(alacritty_terminal::grid::Scroll::Delta(lines));
            cx.notify();
        }
    }
}

/// 输出泵:PTY 增量块 → VT 状态机,合帧触发重绘;EOF = 子进程退出。
/// 任务句柄存 `AppStore::terminal_pump`(kill 换代后泵自退);会话
/// 不可见时照喂但不 notify(切回即最新,不空转窗口重绘)
async fn s_terminal_pump(
    store: &Entity<AppStore>,
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
            if s.terminal.generation != generation {
                return;
            }
            let Some(session) = s.terminal.session.as_mut() else {
                return;
            };
            session.feed(&batch);
            if s.terminal_visible() {
                cx.notify();
            }
        });
    }
    store.update(cx, |s, cx| {
        if s.terminal.generation != generation {
            return;
        }
        let Some(session) = s.terminal.session.as_mut() else {
            return;
        };
        session.exited = true;
        session.exit_code = session
            .pty
            .try_wait()
            .ok()
            .flatten()
            .and_then(|status| status.code);
        if s.terminal_visible() {
            cx.notify();
        }
    });
}
