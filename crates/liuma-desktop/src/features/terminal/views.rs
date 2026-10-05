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

use gpui_kit::component::StyledExt as _;
use gpui_kit::{
    App, Bounds, Entity, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, ParentElement as _, Pixels, ScrollDelta, ScrollWheelEvent,
    StatefulInteractiveElement as _, Styled as _, StyledText, Window, div, px,
};

use crate::features::terminal::palette::row_runs;
use crate::kits::theme;
use crate::shell::panel::TerminalTabId;
use crate::shell::store::AppStore;

/// 单元格字号/行高(对齐聊天终端卡:Menlo 12px,行高 1.5× = 18px)
const FONT_SIZE: f32 = 12.;
const LINE_HEIGHT: f32 = 18.;
const FONT_FAMILY: &str = "Menlo";
/// 视图内边距(文本不贴边;背景仍全 bleed)。cols/rows 按扣边后的
/// 净区计算,末列/末行不裁进 padding
const PAD_X: f32 = 10.;
const PAD_Y: f32 = 6.;

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
    // 由本帧 prepaint 记录,下一帧生效——拖宽/窗口缩放的稳态零开销)
    let bounds = bounds_cell();
    if let Some(b) = bounds.take()
        && let Some(cell) = store.read(cx).terminal.cell
    {
        let (cols, rows) = grid_dims(
            f32::from(b.size.width) - 2. * PAD_X,
            f32::from(b.size.height) - 2. * PAD_Y,
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
        .overflow_hidden()
        // 焦点路径:仓库首例 track_focus 视图;点击终端区聚焦
        .track_focus(&focus)
        .key_context("Terminal")
        .on_mouse_down(MouseButton::Left, {
            let s = store.clone();
            move |_: &MouseDownEvent, window, cx| {
                if let Some(handle) = s.read(cx).terminal.tab(id).map(|t| t.focus.clone()) {
                    window.focus(&handle, cx);
                }
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
    for (row_ix, cells) in lines.iter().enumerate() {
        let cursor_here = cursor_cell
            .filter(|(line, _)| *line == row_ix)
            .map(|(_, col)| col);
        let (text, runs) = row_runs(cells, content.colors, term_fg, term_bg, cursor_here);
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
