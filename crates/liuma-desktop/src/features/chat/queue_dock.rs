//! 队列条带:composer 输入卡顶部的附着面(DSH QueueDock 同构)——
//! 宽 = 输入卡宽 − 两侧 12px inset,底部负 margin 塞进输入卡下 3px,
//! 只圆上角、无下边框(输入卡顶边收口),与输入卡读作一个面。
//! 空队列不渲染;单条直显行(自带队列图标);多条 = 「N 条排队消息」
//! 计数头 + 可折叠列表,行间发丝分隔。行内动作:编辑(行内输入)/
//! 立即投递(steer)/ 移除——走 host `update_queue`(edit/remove/
//! steer),变更后 `session/queue` 帧自动广播回填;动作钮悬停出
//! tooltip(槽位 [`crate::shell::store::TIP_QUEUE_BASE`] + 行序×3 +
//! 钮序)。`steering` 落位不在本组件(消息流尾部的插队气泡,见
//! chat_pane)。立即投递仅在会话运行中渲染:空闲时 queued 条目本就
//! 会被驱动立即认领,steer 窗口已关,徒行只会收到 steer-unavailable
//! 死胡同(DSH 同规:动作仅 running 时可用)。

use gpui_kit::component::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::StyledExt;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Entity, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::features::chat::{QueueEntry, QueuePlacement};
use crate::kits::i18n::dict;
use crate::kits::icons::{LiumaIcon, fixed};
use crate::kits::theme;
use crate::shell::store::AppStore;
use crate::shell::store::TIP_QUEUE_BASE;
use crate::shell::tip_capture_layer;

/// 队列条带(空 = 不渲染;仅 `queued` 落位;composer 附着面)
pub fn render(store: &Entity<AppStore>, window: &mut Window, cx: &mut App) -> impl IntoElement {
    let (col_w, queued, collapsed, editing, session_id, running) = {
        let st = store.read(cx);
        let session_id = st.state.current_id.clone();
        let Some(session_id) = session_id else {
            return div().into_any_element();
        };
        let Some(chat) = st.state.chats.get(&session_id) else {
            return div().into_any_element();
        };
        let queued: Vec<QueueEntry> = chat
            .queue
            .iter()
            .filter(|e| e.placement == QueuePlacement::Queued)
            .cloned()
            .collect();
        let col_w = crate::shell::metrics::window_chat_col_w(
            window,
            st.sidebar_collapsed,
            st.sidebar_px,
            st.panel_open,
            st.panel_px,
        );
        let editing = st.chat.queue_editing.clone();
        let collapsed = st.chat.queue_dock_collapsed;
        // host/session-status 广播的运行态(认领~结算窗口;缺省 = 不在运行)
        let running = st
            .state
            .running_by_id
            .get(&session_id)
            .copied()
            .unwrap_or(false);
        (col_w, queued, collapsed, editing, session_id, running)
    };
    if queued.is_empty() {
        return div().into_any_element();
    }
    // 单条恒直显;多条默认折叠成计数头(点开列表)
    let collapsed = collapsed && queued.len() > 1;
    let is_editing = editing.as_deref().is_some();
    // 附着面:负 margin 塞进 composer 卡下 3px(后绘者覆盖),只圆上角,
    // 下边由 composer 卡顶边收口(DSH:panel + input card 同一面)
    let mut dock = div()
        .debug_selector(|| "queue-dock".to_string())
        .v_flex()
        .mx_auto()
        .w(col_w - px(24.))
        .mb(px(-3.))
        .rounded_tl(px(12.))
        .rounded_tr(px(12.))
        .border_t_1()
        .border_l_1()
        .border_r_1()
        .border_color(theme::BORDER())
        .bg(theme::LAYER())
        .py(px(2.));
    // 多条 = 计数头(可折叠;编辑态强制展开由 list_visible 承担)
    if queued.len() > 1 {
        let s = store.clone();
        let collapsed_now = st_chat_collapsed(store, cx);
        dock = dock.child(
            div()
                .id("queue-dock-header")
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(36.))
                .pl(px(12.))
                .pr(px(4.))
                .rounded(px(8.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::DOCK()))
                .child(fixed(LiumaIcon::ListChecks, 14.).text_color(theme::CAPTION()))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(px(13.))
                        .text_color(theme::LABEL())
                        .font_medium()
                        .child(dict::chat::queue_count(queued.len())),
                )
                .child(fixed(
                    if collapsed_now {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    },
                    14.,
                ))
                .on_click(move |_, _, cx| {
                    s.update(cx, |st, cx| st.toggle_queue_dock_collapse(cx));
                }),
        );
    }
    let list_visible = !collapsed || is_editing;
    if list_visible {
        for (ix, entry) in queued.iter().enumerate() {
            dock = dock.child(queue_row(
                store,
                ix,
                queued.len() == 1,
                entry,
                &session_id,
                &editing,
                running,
                window,
                cx,
            ));
        }
    }
    dock.into_any_element()
}

fn st_chat_collapsed(store: &Entity<AppStore>, cx: &App) -> bool {
    store.read(cx).chat.queue_dock_collapsed
}

/// 队列行:preview(或行内编辑输入)+ 动作钮组。`lead` = 单条直显时
/// 行首队列图标(多条时由计数头承担);行间发丝分隔(ix > 0 上边框)。
#[allow(clippy::too_many_arguments)]
fn queue_row(
    store: &Entity<AppStore>,
    ix: usize,
    lead: bool,
    entry: &QueueEntry,
    session_id: &str,
    editing: &Option<String>,
    running: bool,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let is_editing = editing.as_deref() == Some(entry.id.as_str());
    let editable = entry.text.is_some();
    // 编辑态:输入框(值存 ChatStore.queue_edit_input;渲染期惰建由 store
    // 保证——进入编辑态时已建)
    let mut row = div()
        .id(sid("queue-row", &entry.id))
        .debug_selector(move || format!("queue-row-{}", entry.id))
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(36.))
        .pl(px(12.))
        .pr(px(5.))
        .rounded(px(8.));
    if ix > 0 {
        // 行间发丝分隔(DSH:inset hairline)
        row = row.border_t_1().border_color(theme::BORDER());
    }
    if lead {
        row = row.child(fixed(LiumaIcon::ListChecks, 14.).text_color(theme::CAPTION()));
    }
    if is_editing {
        let st = store.read(cx);
        let input = st.chat.queue_edit_input.clone();
        if let Some(input) = input {
            row = row.child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h(px(28.))
                    .child(gpui_kit::component::input::Input::new(&input).small()),
            );
        }
    } else {
        row = row.child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_size(px(13.))
                .text_color(theme::LABEL_2())
                .child(entry.preview.clone()),
        );
    }
    row.child(queue_actions(
        store, ix, entry, session_id, is_editing, editable, running, window, cx,
    ))
}

/// 动作钮(28px 圆形透明底;悬停浮底 + 提亮;悬停 100ms 出 tooltip)。
/// `slot`/`tip` = tooltip 锚槽位与文案;`on_click` 收 (store, window,
/// ctx)——编辑进入需要 window,其余忽略。
fn action_button(
    store: &Entity<AppStore>,
    slot: usize,
    tip: &'static str,
    icon: gpui_kit::AnyElement,
    id: &'static str,
    on_click: impl Fn(&mut AppStore, &mut Window, &mut Context<AppStore>) + 'static,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let s = store.clone();
    let s_click = store.clone();
    div()
        .id(id)
        .flex()
        .size(px(28.))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .text_color(theme::CAPTION())
        .hover(|s| s.bg(theme::DOCK()).text_color(theme::LABEL()))
        .on_hover(move |enter: &bool, _, cx| {
            s.update(cx, |st, cx| st.header_tip_hover(slot, tip, *enter, cx));
        })
        .on_click(move |_, window, cx| {
            s_click.update(cx, |st, cx| on_click(st, window, cx));
        })
        .child(icon)
        .child(tip_capture_layer(store, slot))
}

/// 行内动作钮组(编辑态 = 保存/取消;常态 = 编辑/[立即投递]/移除)。
/// 槽位序:0 编辑(编辑态 = 保存)/ 1 立即投递(编辑态 = 取消)/
/// 2 移除;立即投递仅运行中渲染。编辑态只保留保存/取消(DSH 同规:
/// 编辑中的行不外露其他动作)。
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
fn queue_actions(
    store: &Entity<AppStore>,
    ix: usize,
    entry: &QueueEntry,
    session_id: &str,
    is_editing: bool,
    editable: bool,
    running: bool,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let _ = (window, cx);
    let session_id = SharedString::from(session_id.to_string());
    let item_id = entry.id.clone();
    let slot = |n: usize| TIP_QUEUE_BASE + ix * 3 + n;
    let mut actions = div().flex().items_center().gap(px(10.));
    if is_editing {
        let (sid_c, iid_c) = (session_id.clone(), item_id.clone());
        actions = actions
            .child(action_button(
                store,
                slot(0),
                dict::common::save(),
                fixed(IconName::Check, 14.).into_any_element(),
                "queue-save",
                move |st, _window, cx| st.queue_save_edit(&sid_c, &iid_c, cx),
            ))
            .child(action_button(
                store,
                slot(1),
                dict::common::cancel(),
                fixed(IconName::Close, 14.).into_any_element(),
                "queue-cancel-edit",
                move |st, _window, cx| st.queue_cancel_edit(cx),
            ));
    } else {
        let sid_e = session_id.clone();
        let iid_e = item_id.clone();
        let (sid_s, iid_s) = (session_id.clone(), item_id.clone());
        let (sid_r, iid_r) = (session_id.clone(), item_id.clone());
        actions = actions
            .when(editable, |el| {
                el.child(action_button(
                    store,
                    slot(0),
                    dict::common::edit(),
                    fixed(LiumaIcon::Pencil, 14.).into_any_element(),
                    "queue-edit",
                    move |st, window, cx| st.queue_begin_edit(&sid_e, &iid_e, window, cx),
                ))
            })
            // 立即投递仅运行中渲染:空闲时 steer 窗口已关(host 会回
            // steer-unavailable),不渲染即不给出死胡同入口
            .when(running, |el| {
                el.child(action_button(
                    store,
                    slot(1),
                    dict::chat::queue_steer(),
                    fixed(IconName::ArrowUp, 14.).into_any_element(),
                    "queue-steer",
                    move |st, _window, cx| {
                        st.queue_action(&sid_s, &iid_s, serde_json::json!({ "kind": "steer" }), cx);
                    },
                ))
            })
            .child(action_button(
                store,
                slot(2),
                dict::common::remove(),
                fixed(IconName::Delete, 14.).into_any_element(),
                "queue-remove",
                move |st, _window, cx| {
                    st.queue_action(&sid_r, &iid_r, serde_json::json!({ "kind": "remove" }), cx)
                },
            ));
    }
    actions.into_any_element()
}

fn sid(prefix: &str, key: &str) -> SharedString {
    SharedString::from(format!("{prefix}-{key}"))
}
