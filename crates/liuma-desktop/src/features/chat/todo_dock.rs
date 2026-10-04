//! TodoDock:composer 上方的计划条,默认折叠一行
//! (标题 + 计数);展开为条目列表。空列表不渲染。
//!
//! 开合行为归库 `Accordion`,外壳(容器 chrome / 头行几何 / 面板内边距)
//! 见 [`crate::kits::collapse_strip`]。展开态受控:真相源仍是
//! `chat.todo_open`,由 `on_toggle` 回传的结果态写回。

use gpui_kit::{
    App, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, div, px,
};

use super::projection::TodoItem;
use crate::kits::collapse_strip;
use crate::kits::i18n::t;
use crate::kits::icons::{LiumaIcon, fixed};
use crate::kits::theme;
use crate::shell::store::AppStore;

/// 计划条整体(todos 为空返回空占位由调用方跳过)
pub fn render(store: &Entity<AppStore>, cx: &App) -> Option<impl IntoElement> {
    let st = store.read(cx);
    let chat = st.current_chat()?;
    if chat.todos.is_empty() {
        return None;
    }
    let open = st.chat.todo_open;
    let counts = todo_counts(&chat.todos);
    let s = store.clone();
    Some(collapse_strip::strip(
        "todo-dock",
        open,
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(fixed(LiumaIcon::ListChecks, 14.).text_color(theme::label_2(cx)))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme::label_2(cx))
                    .child(t!("shell.plan_tab")),
            )
            .child(
                div()
                    .debug_selector(|| "todo-dock-count".to_string())
                    .flex_1()
                    .text_size(px(11.))
                    .text_color(theme::caption(cx))
                    .child(t!(
                        "chat.todo_counts",
                        done = counts.0,
                        active = counts.1,
                        pending = counts.2
                    )),
            ),
        12.,
        chat.todos
            .iter()
            .map(|i| todo_row(i, cx))
            .collect::<Vec<_>>(),
        move |open, _, cx| {
            s.update(cx, |st, cx| st.set_todo_open(open, cx));
        },
        cx,
    ))
}

/// 单条 todo(状态点 + 内容;todo_write 工具卡展开体复用同一视觉)
pub(crate) fn todo_row(item: &TodoItem, cx: &App) -> impl IntoElement {
    let dot = match item.status.as_str() {
        "completed" => theme::success(cx),
        "in_progress" => theme::brand(cx),
        _ => theme::caption(cx),
    };
    let fg = if item.status == "completed" {
        theme::caption(cx)
    } else {
        theme::label_2(cx)
    };
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .py(px(2.))
        .child(div().size(px(6.)).flex_shrink_0().rounded_full().bg(dot))
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .text_size(px(12.))
                .text_color(fg)
                .child(item.content.clone()),
        )
}

/// (done, in_progress, pending) 计数
fn todo_counts(todos: &[TodoItem]) -> (usize, usize, usize) {
    let mut c = (0, 0, 0);
    for t in todos {
        match t.status.as_str() {
            "completed" => c.0 += 1,
            "in_progress" => c.1 += 1,
            _ => c.2 += 1,
        }
    }
    c
}
