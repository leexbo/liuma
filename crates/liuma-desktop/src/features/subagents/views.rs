//! 血缘 UI:页头
//! 「{count} 个子代理」触发器(运行中带活动圆点)+ 树形下拉目录——
//! 每行 = 状态点 + 任务名 + 副行(prompt 截断 · 结算 detail)+ 运行计时,
//! 点击跳子会话。数据以 jobs 帧权威,子会话清单补位。
//! (从 ui::topbar 切出;trigger 挂于顶栏标题右侧操作组。)
//!
//! 任务条的折叠头/行列表由库 `Accordion` 托管,外壳(容器 chrome /
//! 头行几何 / 面板内边距)见 [`crate::kits::collapse_strip`];展开态
//! 受控,真相源是 `subagents.task_bar_open`。

use gpui_kit::component::StyledExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::kits::collapse_strip;
use crate::kits::icons::{LiumaIcon, fixed};
use crate::kits::theme;
use crate::shell::store::AppStore;

use super::store::LineageRow;
use crate::kits::i18n::t;

/// 状态点:running=活动蓝,completed=done 绿,
/// failed=红,killed=黄
fn state_dot(dot: &'static str, cx: &App) -> gpui_kit::AnyElement {
    let color = match dot {
        "running" => theme::ongoing(cx),
        "failed" => theme::danger(cx),
        "killed" => theme::warning(cx),
        _ => theme::success(cx),
    };
    div()
        .debug_selector(move || format!("lineage-dot-{dot}"))
        .size(px(6.))
        .flex_shrink_0()
        .rounded_full()
        .bg(color)
        .into_any_element()
}

/// 运行计时文案:运行中 = 墙钟差走秒(菜单开时 lineage_tick 每秒刷新);
/// 已结束 = 起止差定格
fn duration_text(row: &LineageRow) -> String {
    let now_ms = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    };
    let Some(started) = row.started_at else {
        return String::new();
    };
    let end = if row.running {
        now_ms()
    } else {
        row.finished_at.unwrap_or_else(now_ms)
    };
    let secs = (end - started).max(0) / 1000;
    if secs < 60 {
        t!("time.duration_s", v = secs).into_owned()
    } else {
        t!(
            "time.duration_ms",
            mins = secs / 60,
            secs = format!("{:02}", secs % 60)
        )
        .into_owned()
    }
}

/// 子代理任务面板(任务列表卡——与
/// TodoDock 同一视觉语言):标题行(Workflow 图标 + 「子代理」 + 运行
/// 摘要 + 折叠 chevron);展开为行列表 = 「主线」行(仅子会话视图,
/// 一键返回)+ 运行中子代理行(状态点+任务名+走秒计时)+ 当前查看的
/// 子会话行(已结束也高亮定位),点击行切换查看。集合语义:运行中
/// 全部 + 当前查看;全无 → 整卡不渲染。
pub(crate) fn task_bar(store: &Entity<AppStore>, cx: &App) -> Option<gpui_kit::AnyElement> {
    let st = store.read(cx);
    let current = st.state.current_id.clone()?;
    let anchor = st.subagent_anchor_of(&current);
    let viewing_child = anchor != current;
    let rows = st.lineage_rows(&anchor);
    // 集合:运行中全部 + shell job 全生命周期(终态行不删——任务事实
    // 仍可达,但收进「已结束」折叠区,不再平铺越攒越多;有运行中行时
    // 默认收起)+ 驻留待命(idle 可继续;对等寻址面:待命代理仍可收
    // 消息,行副行摘要的价值恰在空闲期)+ 当前查看的子会话补位高亮
    let mut chips: Vec<LineageRow> = rows
        .iter()
        .filter(|r| {
            r.running || r.kind == "shell" || (r.kind != "shell" && r.dot == Some("completed"))
        })
        .cloned()
        .collect();
    if viewing_child
        && !chips.iter().any(|r| r.session_id == current)
        && let Some(row) = rows.iter().find(|r| r.session_id == current)
    {
        chips.push(row.clone());
    }
    if chips.is_empty() {
        return None;
    }
    // 分区:运行中置顶(查看中补位行始终可见);已结束按终态时刻
    // 新→旧排序收进折叠区
    let live: Vec<LineageRow> = chips
        .iter()
        .filter(|r| r.running || r.session_id == current)
        .cloned()
        .collect();
    let mut settled: Vec<LineageRow> = chips
        .iter()
        .filter(|r| !r.running && r.session_id != current)
        .cloned()
        .collect();
    settled.sort_by_key(|r| std::cmp::Reverse(r.finished_at));
    let running_n = chips.iter().filter(|r| r.running).count();
    let standby_n = chips
        .iter()
        .filter(|r| !r.running && r.kind != "shell")
        .count();
    let open = st.subagents.task_bar_open;
    let s = store.clone();
    // 头行随内容组成分流:纯子代理 = 「子代理」,纯 shell job = 「后台
    // 任务」(公文包图标),混合 = 合称——shell job 不是子代理,混排
    // 在单一标题下会把后台命令误报成子代理
    let has_sub = chips.iter().any(|r| r.kind != "shell");
    let has_shell = chips.iter().any(|r| r.kind == "shell");
    let (head_icon, head_title) = match (has_sub, has_shell) {
        (false, true) => (
            LiumaIcon::Briefcase,
            t!("misc.shell_jobs_title").into_owned(),
        ),
        (true, true) => (
            LiumaIcon::Workflow,
            t!("misc.tasks_mixed_title").into_owned(),
        ),
        _ => (LiumaIcon::Workflow, t!("misc.subagents_title").into_owned()),
    };
    // 头行内容(尾部 chevron 由库 Accordion 追加,开合态自驱);
    // 头行几何 / 容器 chrome 见 kits::collapse_strip
    let mut head = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(fixed(head_icon, 14.).text_color(theme::label_2(cx)))
        .child(
            div()
                .text_size(px(12.))
                .text_color(theme::label_2(cx))
                .child(head_title),
        )
        .child(
            div()
                .flex_1()
                .text_size(px(11.))
                .text_color(theme::caption(cx))
                .child(if running_n > 0 {
                    t!("misc.running_n", n = running_n).into_owned()
                } else if standby_n > 0 {
                    t!("misc.standby_n", n = standby_n).into_owned()
                } else {
                    t!("misc.ended").to_string()
                }),
        );
    // 子会话视图:标题行尾缀「主线」返回钮(不挤列表行)
    if viewing_child {
        let s_main = store.clone();
        let main_id = anchor.clone();
        head = head.child(
            div()
                .id("task-chip-main")
                .debug_selector(|| "task-chip-main".to_string())
                .flex()
                .items_center()
                .gap(px(4.))
                .rounded(px(10.))
                .border_1()
                .border_color(theme::brand(cx))
                .px(px(8.))
                .h(px(20.))
                .cursor_pointer()
                .hover(|st| st.bg(theme::dock(cx)))
                .text_size(px(11.))
                .text_color(theme::brand(cx))
                .on_click(move |_, _, cx| {
                    let id = main_id.clone();
                    s_main.update(cx, |st, cx| st.open_session(&id, cx));
                })
                .child(fixed(gpui_kit::component::IconName::ArrowLeft, 11.))
                .child(t!("misc.mainline")),
        );
    }
    // 行渲染闭包:live/settled 两组共用;借用 store/current,逐行克隆
    let render_row = |chip: &LineageRow, cx: &App| -> gpui_kit::AnyElement {
        let s_chip = store.clone();
        let chip_id = chip.session_id.clone();
        let is_shell = chip.kind == "shell";
        // shell 行选择器带 id(终态可见性断言按行定位);子代理行
        // 保持裸 "task-chip"(既有测试按它点击切换)
        let sel_id = chip.session_id.clone();
        let running = chip.running;
        let viewing = chip.session_id == current;
        let timing = duration_text(chip);
        let last_message = chip.last_message.clone();
        div()
            .id(gpui_kit::SharedString::from(format!(
                "task-row-{}",
                chip.session_id
            )))
            .debug_selector(move || {
                if is_shell {
                    format!("task-chip-{sel_id}")
                } else {
                    "task-chip".to_string()
                }
            })
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(8.))
            .px(px(10.))
            .py(px(4.))
            .bg(if viewing {
                theme::dock(cx)
            } else {
                theme::TRANSPARENT
            })
            // shell job 行非会话,无跳转(点击不响应,不显手型)
            .when(chip.kind != "shell", |el| {
                el.cursor_pointer()
                    .hover(|st| st.bg(theme::dock(cx)))
                    .on_click(move |_, _, cx| {
                        let id = chip_id.clone();
                        s_chip.update(cx, |st, cx| st.open_session(&id, cx));
                    })
            })
            .when_some(chip.dot, |el, dot| el.child(state_dot(dot, cx)))
            // shell 行缀公文包图标:与子代理行同列时可辨
            .when(chip.kind == "shell", |el| {
                el.child(fixed(LiumaIcon::Briefcase, 12.).text_color(theme::label_2(cx)))
            })
            // 主列两行:任务名 + 最近入站消息摘要(对等互发呈现,
            // 通知不进转录——行副行承担可见性)
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .v_flex()
                    .when_some(last_message, |el, msg| {
                        el.child(
                            div()
                                .id("task-chip-lastmsg")
                                .min_w(px(0.))
                                .text_size(px(10.))
                                .text_color(theme::caption(cx))
                                .truncate()
                                .child(msg),
                        )
                    })
                    .child(
                        div()
                            .min_w(px(0.))
                            .text_size(px(12.))
                            .text_color(if viewing {
                                theme::label(cx)
                            } else {
                                theme::label_2(cx)
                            })
                            .truncate()
                            .child(chip.label.clone()),
                    ),
            )
            .when(!timing.is_empty(), |el| {
                el.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(theme::caption(cx))
                        .child(timing),
                )
            })
            .when(viewing, |el| {
                el.child(
                    fixed(gpui_kit::component::IconName::Check, 12.).text_color(theme::brand(cx)),
                )
            })
            // 运行中行尾打断钮(豁免冒泡——点击不触发行切换;
            // 打断 = 只停当前 turn,子代理保持可续话)
            .when(running, |el| {
                let s_stop = store.clone();
                let stop_id = chip.session_id.clone();
                let stop_kind = chip.kind.clone();
                el.child(
                    div()
                        .id(gpui_kit::SharedString::from(format!(
                            "task-stop-{}",
                            chip.session_id
                        )))
                        .debug_selector(|| "task-stop".to_string())
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .h(px(20.))
                        .px(px(8.))
                        .rounded(px(10.))
                        .border_1()
                        .border_color(theme::border_2(cx))
                        .cursor_pointer()
                        .hover(|st| st.bg(theme::danger(cx)).border_color(theme::danger(cx)))
                        .text_size(px(11.))
                        .text_color(theme::label_2(cx))
                        .child(t!("misc.interrupt"))
                        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation()
                        })
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            let id = stop_id.clone();
                            let kind = stop_kind.clone();
                            s_stop.update(cx, |st, cx| st.interrupt_task(&kind, &id, cx));
                        }),
                )
            })
            .into_any_element()
    };
    let mut list_rows: Vec<gpui_kit::AnyElement> = live.iter().map(|c| render_row(c, cx)).collect();
    // 已结束折叠区:有运行中行时默认收起(仅计数行),全部结束后默认
    // 展开;手动开合覆盖自动规则(存 store,不随状态漂移)
    if !settled.is_empty() {
        let expanded = st.subagents.settled_open.unwrap_or(running_n == 0);
        let s_settled = store.clone();
        list_rows.push(
            div()
                .id("task-settled-toggle")
                .debug_selector(|| "task-settled-toggle".to_string())
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .py(px(4.))
                .rounded(px(8.))
                .cursor_pointer()
                .hover(|st| st.bg(theme::dock(cx)))
                .text_size(px(11.))
                .text_color(theme::label_2(cx))
                .child(t!("misc.settled_n", n = settled.len()).to_string())
                .child(
                    fixed(
                        if expanded {
                            gpui_kit::component::IconName::ChevronUp
                        } else {
                            gpui_kit::component::IconName::ChevronDown
                        },
                        11.,
                    )
                    .text_color(theme::label_3(cx)),
                )
                .on_click(move |_, _, cx| {
                    s_settled.update(cx, |st, cx| st.set_settled_open(!expanded, cx));
                })
                .into_any_element(),
        );
        if expanded {
            list_rows.extend(settled.iter().map(|c| render_row(c, cx)));
        }
    }
    Some(collapse_strip::strip(
        "task-bar",
        open,
        head,
        8.,
        list_rows,
        move |open, _, cx| {
            s.update(cx, |st, cx| st.set_task_bar_open(open, cx));
        },
        cx,
    ))
}
