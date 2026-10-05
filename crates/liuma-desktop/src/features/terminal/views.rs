//! 终端视图:网格行渲染 + 焦点键盘输入 + 滚轮回看 + 尺寸自适应。
//!
//! 渲染面:`renderable_content()` 逐行 → `StyledText` 高亮段(palette
//! 纯逻辑),行高 18px(同聊天终端卡 Menlo 12px/1.5)。容器不滚动——
//! 滚回由 Term 的 `display_offset` 承担(滚轮 → `scroll_display`)。
//! 焦点:仓库首个焦点路径视图(`track_focus` + `key_context`),标签
//! 激活经 `wants_focus` 在渲染期消费;按键 capture 阶段编码直写 PTY。
//! 尺寸:canvas 回调记 bounds(线程局部,同轨迹时间线),下一渲染帧
//! 变化检测后同步 `term.resize` + PTY ioctl(SIGWINCH 由内核送)。
//!
//! 视图按标签 id 参数化:只有激活终端标签进渲染(tab_body 分发),
//! 后台标签会话保留。网格读侧全程持 `store.read` 守卫(`&Cell` 引用
//! 不可跨守卫),行文本与高亮段在此作用域内物化为 owned(`String` +
//! `HighlightStyle`)。

use std::cell::Cell as StdCell;
use std::rc::Rc;

use alacritty_terminal::index::Side;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::{IconName, Sizable};
use gpui_kit::{
    App, Bounds, Entity, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, ScrollDelta,
    ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled as _, StyledText,
    Window, div, px,
};

use crate::features::terminal::palette::{
    CellMark, match_row_range, row_runs, selection_row_ranges,
};
use crate::kits::icons::fixed;
use crate::kits::theme;
use crate::shell::panel::TerminalTabId;
use crate::shell::store::AppStore;

/// 单元格字号/行高(对齐聊天终端卡:Menlo 12px,行高 1.5× = 18px)
const FONT_SIZE: f32 = 12.;
const LINE_HEIGHT: f32 = 18.;
const FONT_FAMILY: &str = "Menlo";
/// 视图内边距(文本不贴边;背景仍全 bleed)。cols/rows 按扣边后的
/// 净区计算,末列/末行不裁进 padding
pub(crate) const PAD_X: f32 = 10.;
pub(crate) const PAD_Y: f32 = 6.;
/// 搜索条高(在流内占位:网格可用高要扣掉它)
const SEARCH_BAR_H: f32 = 30.;

/// 渲染期 bounds 记录(线程局部跨帧共享,同轨迹 `track_bounds_cell`)
fn bounds_cell() -> Rc<StdCell<Option<Bounds<Pixels>>>> {
    thread_local! {
        static CELL: Rc<StdCell<Option<Bounds<Pixels>>>> = Rc::default();
    }
    CELL.with(|c| c.clone())
}

/// 实测等宽单元格尺寸(一次测得缓存进 store;8 字符取平均消亚像素误差)
fn measure_cell(window: &Window) -> (f32, f32) {
    let sample = "W".repeat(8);
    let font = gpui_kit::Font {
        family: FONT_FAMILY.into(),
        ..Default::default()
    };
    let run = gpui_kit::TextRun {
        len: sample.len(),
        font,
        color: gpui_kit::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let layout = window
        .text_system()
        .layout_line(&sample, px(FONT_SIZE), &[run], None);
    let width = f32::from(layout.width) / sample.len() as f32;
    (width.max(1.), LINE_HEIGHT)
}

/// 由 bounds 与单元格尺寸算网格行列(上下限 clamp 见 terminal_apply_size)
fn grid_dims(width: f32, height: f32, cell: (f32, f32)) -> (usize, usize) {
    let cols = ((width / cell.0).floor() as usize).max(1);
    let rows = ((height / cell.1).floor() as usize).max(1);
    (cols, rows)
}

/// 窗口坐标 → 视口格坐标 + 半格侧(光标在格内右半 → Right)。bounds
/// 取上一渲染帧 canvas 记录(含 padding 区);终端区外/尺寸未测 = None
fn cell_at(
    store: &Entity<AppStore>,
    cx: &App,
    position: gpui_kit::Point<Pixels>,
) -> Option<(usize, usize, Side)> {
    let bounds = bounds_cell().get()?;
    let cell = store.read(cx).terminal.cell?;
    let fx = (position.x.as_f32() - f32::from(bounds.origin.x) - PAD_X) / cell.0;
    let fy = (position.y.as_f32() - f32::from(bounds.origin.y) - PAD_Y) / cell.1;
    if fx < 0. || fy < 0. {
        return None;
    }
    let st = store.read(cx);
    let col = (fx.floor() as usize).min(st.terminal.cols.saturating_sub(1));
    let row = (fy.floor() as usize).min(st.terminal.rows.saturating_sub(1));
    let side = if fx - fx.floor() > 0.5 {
        Side::Right
    } else {
        Side::Left
    };
    Some((col, row, side))
}

/// 终端标签正文(面板 tab_body 分发臂;根 size_full 填满)
pub(crate) fn render(
    store: &Entity<AppStore>,
    id: TerminalTabId,
    window: &mut Window,
    cx: &mut App,
) -> gpui_kit::AnyElement {
    // —— 突变相位(各自收口读写,再进网格读侧长守卫)——
    // 单元格尺寸缓存:首帧实测,字号恒定后无重测
    if store.read(cx).terminal.cell.is_none() {
        let cell = measure_cell(window);
        store.update(cx, |s, cx| {
            s.terminal.cell = Some(cell);
            cx.notify();
        });
    }
    // 尺寸自适应:上一帧 canvas bounds 算行列,变化才落 store(bounds
    // 由本帧 prepaint 记录,下一帧生效——拖宽/窗口缩放的稳态零开销;
    // 搜索条在流内占位,网格可用高按开合扣减)
    let search_open = store
        .read(cx)
        .terminal
        .tab(id)
        .is_some_and(|t| t.search.is_some());
    let bounds = bounds_cell();
    if let Some(b) = bounds.take()
        && let Some(cell) = store.read(cx).terminal.cell
    {
        let bar_h = if search_open { SEARCH_BAR_H } else { 0. };
        let (cols, rows) = grid_dims(
            f32::from(b.size.width) - 2. * PAD_X,
            f32::from(b.size.height) - 2. * PAD_Y - bar_h,
            cell,
        );
        let unchanged = {
            let s = store.read(cx);
            s.terminal.cols == cols && s.terminal.rows == rows
        };
        if !unchanged {
            store.update(cx, |s, cx| s.terminal_apply_size(cols, rows, cx));
        }
    }
    // 标签激活置位的焦点消费:请求聚焦,成功(焦点确实落在终端句柄)
    // 才清旗标——失败(窗口焦点系统未就绪/他者抢走)保留待下帧重试,
    // 避免首帧请求被静默吞掉后永远拿不到键盘
    let wants_focus = store
        .read(cx)
        .terminal
        .tab(id)
        .is_some_and(|t| t.wants_focus);
    if wants_focus && let Some(handle) = store.read(cx).terminal.tab(id).map(|t| t.focus.clone()) {
        window.focus(&handle, cx);
        if handle.is_focused(window) {
            store.update(cx, |s, _| {
                if let Some(tab) = s.terminal.tab_mut(id) {
                    tab.wants_focus = false;
                }
            });
        }
    }

    // —— 网格读侧:守卫内快照 + 行物化 ——
    let s = store.read(cx);
    let Some(tab) = s.terminal.tab(id) else {
        // 标签已撤(理论不可达:tab_body 分发自面板标签表):空视图
        return div().into_any_element();
    };
    let focus = tab.focus.clone();
    let exited = tab.session.as_ref().is_some_and(|session| session.exited);

    let mut root = div()
        .debug_selector(|| "panel-terminal-view".to_string())
        .relative()
        .size_full()
        .v_flex()
        .px(px(PAD_X))
        .py(px(PAD_Y))
        .font_family(FONT_FAMILY)
        .text_size(px(FONT_SIZE))
        .line_height(gpui_kit::relative(LINE_HEIGHT / FONT_SIZE))
        .bg(theme::code(cx))
        .text_color(theme::label(cx))
        // 网格区鼠标 = 文字输入 I 形(子件按需覆盖:按钮 cursor_pointer)
        .cursor(gpui_kit::CursorStyle::IBeam)
        .overflow_hidden()
        // 焦点路径:仓库首例 track_focus 视图;点击聚焦 + 拖选/取词/取行
        // (坐标换算在 cell_at:canvas bounds 线程局部,渲染帧已备)
        .track_focus(&focus)
        .key_context("Terminal")
        .on_mouse_down(MouseButton::Left, {
            let s = store.clone();
            move |ev: &MouseDownEvent, window, cx| {
                if let Some(handle) = s.read(cx).terminal.tab(id).map(|t| t.focus.clone()) {
                    window.focus(&handle, cx);
                }
                if let Some((col, row, side)) = cell_at(&s, cx, ev.position) {
                    s.update(cx, |st, cx| {
                        st.terminal_pointer_down(id, col, row, side, cx)
                    });
                }
            }
        })
        // 右键:有选区 → 原生菜单「复制」(文本在此抓取进暂存,动作经
        // App 级 on_action 消费——分发期无 window 回读实时选区,同聊天)
        .on_mouse_down(MouseButton::Right, {
            let s = store.clone();
            move |ev: &MouseDownEvent, window, cx| {
                let Some(text) = s.read(cx).terminal_selection_text(id) else {
                    return;
                };
                s.update(cx, |st, _| st.terminal.pending_copy = Some(text));
                NativeMenu::new()
                    .menu(
                        crate::kits::i18n::t!("terminal.copy_menu").to_string(),
                        Box::new(super::CopyTerminalSelection),
                    )
                    .show(ev.position, window, cx);
            }
        })
        // 滚轮:行增量为历史方向(alt-screen 时 Term 自理);Pixels 态
        // 按单元格高折算
        .on_scroll_wheel({
            let s = store.clone();
            move |ev: &ScrollWheelEvent, _window: &mut Window, cx: &mut App| {
                let lines = match ev.delta {
                    ScrollDelta::Lines(p) => (p.y * 3.).round() as i32,
                    ScrollDelta::Pixels(p) => {
                        let cell_h = s.read(cx).terminal.cell.map_or(LINE_HEIGHT, |c| c.1);
                        (p.y.as_f32() / cell_h).round() as i32
                    }
                };
                if lines != 0 {
                    s.update(cx, |st, cx| st.terminal_scroll(id, lines, cx));
                }
            }
        })
        // 键盘 capture:焦点在终端时编码直写 PTY(命中即 stop_propagation;
        // ⌘ 系与未映射键放行给 UI 快捷键体系)
        .capture_key_down({
            let s = store.clone();
            move |ev: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                let focused = s
                    .read(cx)
                    .terminal
                    .tab(id)
                    .is_some_and(|t| t.focus.is_focused(window));
                if !focused {
                    return;
                }
                // ⌘C 复制选区。control/alt 修饰挡住:Windows/Linux 上
                // platform 即 ctrl,ctrl+c 是 SIGINT 必须照旧落 PTY
                if ev.keystroke.modifiers.platform
                    && !ev.keystroke.modifiers.control
                    && !ev.keystroke.modifiers.alt
                    && ev.keystroke.key == "c"
                    && let Some(text) = s.read(cx).terminal_selection_text(id)
                {
                    cx.stop_propagation();
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
                    return;
                }
                // ⌘F 搜索条(control/alt 挡住:Windows/Linux ctrl+f
                // 是 shell 前向字符,必须照旧落 PTY)
                if ev.keystroke.modifiers.platform
                    && !ev.keystroke.modifiers.control
                    && !ev.keystroke.modifiers.alt
                    && ev.keystroke.key == "f"
                {
                    cx.stop_propagation();
                    s.update(cx, |st, cx| st.terminal_search_open(id, window, cx));
                    return;
                }
                // ⌘V 粘贴(bracketed paste 按 PTY 模式包装)
                if ev.keystroke.modifiers.platform
                    && ev.keystroke.key == "v"
                    && let Some(item) = cx.read_from_clipboard()
                    && let Some(text) = item.text()
                {
                    cx.stop_propagation();
                    s.update(cx, |st, cx| st.terminal_write_paste(id, &text, cx));
                    return;
                }
                let keystroke = ev.keystroke.clone();
                if s.update(cx, |st, cx| st.terminal_write_key(id, &keystroke, cx)) {
                    cx.stop_propagation();
                }
            }
        });

    // 尺寸测量 canvas(样式直接挂在 canvas 上:叶子元素无固有尺寸,
    // 套包裹 div 会量到 0 高 → 网格被错 resize 成 2 行。绝对锚定铺满
    // 视图,prepaint 记 bounds 供下一帧变化检测)
    let bounds_for_canvas = bounds.clone();
    root = root.child(
        gpui_kit::canvas(
            move |b, _, _| bounds_for_canvas.set(Some(b)),
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    );

    // 拖选进行中:窗口级 move/up 注册(同面板拖宽手法——move 更新
    // 选区,up 独立注册保证抬起必收尾)
    if s.terminal.tab(id).is_some_and(|t| t.drag) {
        let s_move = store.clone();
        let s_up = store.clone();
        root = root.child(
            gpui_kit::canvas(
                |_, _, _| (),
                move |_, _, window, _cx| {
                    window.on_mouse_event(move |ev: &MouseMoveEvent, _, _, cx| {
                        if let Some((col, row, side)) = cell_at(&s_move, cx, ev.position) {
                            s_move.update(cx, |st, cx| {
                                st.terminal_pointer_drag(id, col, row, side, cx)
                            });
                        }
                    });
                    window.on_mouse_event(move |_: &MouseUpEvent, _, _, cx| {
                        s_up.update(cx, |st, cx| st.terminal_pointer_up(id, cx));
                    });
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
    }

    // 搜索条(在流内,压网格顶;canvas 为绝对锚定不占流高)
    if search_open && let Some(input) = s.terminal.search_input.clone() {
        root = root.child(search_bar(store, id, &input, cx));
    }

    let session = match tab.session.as_ref() {
        Some(session) => session,
        None => {
            // 未装配(spawning 在途)/无会话:占位,避免空白抖动
            root = root.child(
                div()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_color(theme::caption(cx))
                    .child(crate::kits::i18n::t!("terminal.starting").to_string()),
            );
            if exited {
                root = root.child(exited_strip(store.clone(), id, cx));
            }
            return root.into_any_element();
        }
    };

    let content = session.term.renderable_content();
    let offset = content.display_offset as i32;
    let lines = crate::features::terminal::palette::bucket_lines(
        content.display_iter,
        offset,
        s.terminal.rows,
    );
    // 光标块:网格坐标 + display_offset = 视口行(滚回时光标随内容
    // 下移,移出视口则不画)
    let cursor_cell = usize::try_from(content.cursor.point.line.0 + offset)
        .ok()
        .filter(|line| *line < lines.len())
        .map(|line| (line, content.cursor.point.column.0));
    let term_fg = gpui_kit::Hsla::from(theme::label(cx));
    let term_bg = gpui_kit::Hsla::from(theme::code(cx));
    let selection_bg = gpui_kit::Hsla::from(theme::brand(cx)).opacity(0.35);
    let match_bg = gpui_kit::Hsla::from(theme::brand(cx)).opacity(0.22);
    let current_bg = gpui_kit::Hsla::from(theme::brand(cx)).opacity(0.5);
    // 选区:网格 SelectionRange → 每视口行含端点列区间
    let selection_ranges = content
        .selection
        .as_ref()
        .map(|range| selection_row_ranges(range, offset, s.terminal.rows));
    // 搜索命中(全部弱染 + 当前命中强染);行内标记顺序 = 普通命中
    // < 选区 < 当前命中(后位覆盖前位)
    let search_state = tab.search.as_ref();
    for (row_ix, cells) in lines.iter().enumerate() {
        let cursor_here = cursor_cell
            .filter(|(line, _)| *line == row_ix)
            .map(|(_, col)| col);
        let line = row_ix as i32 - offset;
        let mut row_marks: Vec<CellMark> = Vec::new();
        if let Some(search) = search_state {
            for m in &search.matches {
                if search.current == Some(*m.start()) {
                    continue;
                }
                if let Some((start, end)) = match_row_range(m, line) {
                    row_marks.push(CellMark {
                        start,
                        end,
                        color: match_bg,
                    });
                }
            }
        }
        if let Some((start, end)) = selection_ranges
            .as_ref()
            .and_then(|ranges| ranges.get(row_ix).copied().flatten())
        {
            row_marks.push(CellMark {
                start,
                end,
                color: selection_bg,
            });
        }
        if let Some(search) = search_state
            && let Some(current) = search.current
        {
            for m in &search.matches {
                if m.start() != &current {
                    continue;
                }
                if let Some((start, end)) = match_row_range(m, line) {
                    row_marks.push(CellMark {
                        start,
                        end,
                        color: current_bg,
                    });
                }
            }
        }
        let (text, runs) = row_runs(
            cells,
            content.colors,
            term_fg,
            term_bg,
            cursor_here,
            &row_marks,
        );
        if text.is_empty() {
            root = root.child(div().h(px(LINE_HEIGHT)).flex_shrink_0());
            continue;
        }
        root = root.child(
            div()
                .h(px(LINE_HEIGHT))
                .flex_shrink_0()
                .child(StyledText::new(text).with_highlights(runs)),
        );
    }
    if exited {
        root = root.child(exited_strip(store.clone(), id, cx));
    }
    root.into_any_element()
}

/// 终端内搜索条:输入 + 上/下导航 + 计数 + 关闭。整条左键按下截停
/// (不触发终端选区/聚焦);✕ 关闭并把焦点还给终端;Enter/Shift-Enter
/// 导航在输入订阅里(terminal_search_open)
fn search_bar(
    store: &Entity<AppStore>,
    id: TerminalTabId,
    input: &Entity<InputState>,
    cx: &App,
) -> gpui_kit::AnyElement {
    let count = store
        .read(cx)
        .terminal
        .tab(id)
        .and_then(|t| t.search.as_ref())
        .map(|search| {
            let at = search
                .current
                .and_then(|p| search.matches.iter().position(|m| m.start() == &p));
            match (at, search.matches.len()) {
                (Some(i), n) => {
                    crate::kits::i18n::t!("terminal.search_count", cur = i + 1, total = n)
                        .to_string()
                }
                _ => crate::kits::i18n::t!("terminal.search_none").to_string(),
            }
        });
    let s_prev = store.clone();
    let s_next = store.clone();
    let s_close = store.clone();
    div()
        .flex_shrink_0()
        .h(px(SEARCH_BAR_H))
        .flex()
        .items_center()
        .gap(px(4.))
        .mb(px(4.))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .h(px(SEARCH_BAR_H))
                .flex()
                .items_center()
                .rounded(px(8.))
                .border_1()
                .border_color(theme::border_2(cx))
                .bg(theme::card(cx))
                .pl(px(6.))
                .pr(px(4.))
                .child(
                    // XSmall = text_xs 12px,与终端网格字号一致(small 14px
                    // 相形偏大)
                    Input::new(input).xsmall().appearance(false),
                ),
        )
        .child(search_nav_btn(&s_prev, id, true, cx))
        .child(search_nav_btn(&s_next, id, false, cx))
        .child(
            div()
                .flex_shrink_0()
                .min_w(px(48.))
                .text_size(px(11.))
                .text_color(theme::caption(cx))
                .child(count.unwrap_or_default()),
        )
        .child(
            div()
                .id("term-search-close")
                .flex()
                .size(px(22.))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .text_color(theme::caption(cx))
                .hover(|st| st.bg(theme::dock(cx)).text_color(theme::label(cx)))
                .child(fixed(IconName::Close, 12.))
                .on_click(move |_, window, cx| {
                    s_close.update(cx, |st, cx| st.terminal_search_close(id, window, cx));
                }),
        )
        .into_any_element()
}

/// 搜索导航钮(↑ 上一个 / ↓ 下一个)
fn search_nav_btn(
    store: &Entity<AppStore>,
    id: TerminalTabId,
    up: bool,
    cx: &App,
) -> gpui_kit::AnyElement {
    let (icon, aid) = if up {
        (IconName::ChevronUp, "term-search-prev")
    } else {
        (IconName::ChevronDown, "term-search-next")
    };
    let s = store.clone();
    div()
        .id(SharedString::from(aid))
        .flex()
        .size(px(22.))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .text_color(theme::label_2(cx))
        .hover(|st| st.bg(theme::dock(cx)).text_color(theme::label(cx)))
        .child(fixed(icon, 13.))
        .on_click(move |_, _, cx| {
            s.update(cx, |st, cx| {
                if up {
                    st.terminal_search_prev(id, cx);
                } else {
                    st.terminal_search_next(id, cx);
                }
            });
        })
        .into_any_element()
}

/// 退出态条:提示 + 重开入口(点击 = 杀旧会话并重 spawn)
fn exited_strip(store: Entity<AppStore>, id: TerminalTabId, cx: &App) -> gpui_kit::AnyElement {
    div()
        .flex_shrink_0()
        .h(px(28.))
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .text_size(px(11.))
        .text_color(theme::caption(cx))
        .bg(theme::layer(cx))
        .child(crate::kits::i18n::t!("terminal.exited").to_string())
        .child(
            div()
                .id("terminal-restart")
                .px(px(6.))
                .py(px(2.))
                .rounded(px(5.))
                .cursor_pointer()
                .text_color(theme::label_2(cx))
                .hover(|st| st.bg(theme::dock(cx)).text_color(theme::label(cx)))
                .child(crate::kits::i18n::t!("terminal.restart").to_string())
                .on_click(move |_, _, cx| {
                    store.update(cx, |st, cx| {
                        st.terminal_kill(id, cx);
                        st.terminal_spawn(id, cx);
                    });
                }),
        )
        .into_any_element()
}
