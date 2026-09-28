//! 上下文压缩策略(纯函数层)。
//!
//! 压力阈值与保留尾按上下文窗口的
//! token 预算计(threshold=0.8×窗口,retain=0.16×窗口);压缩范围 = 连续
//! 头部区间,切点回退到 tool 配对平衡处(永不拆散 assistant tool_calls
//! 与其 tool/result);摘要指令以最终 user 消息追加在逐字前缀之后。
//! 持久化词汇(`compaction/summary` 事件)与折叠包装(派生面)归
//! `liuma-session`;本 crate 只做决策与指令构造,不做 IO、不调模型。

#![deny(missing_docs)]

use liuma_session::EventEnvelope;

/// 上下文窗口缺省值(未配置 per-model 窗口时)
pub const DEFAULT_CONTEXT_WINDOW: u64 = 1_000_000;
/// 压力阈值占比:上下文 ≥ 窗口×此值时自动折叠
pub const THRESHOLD_RATIO: f64 = 0.8;
/// 保留尾占比:最近窗口×此值的上下文逐字保留
pub const RETAIN_RATIO: f64 = 0.16;
/// 无真实 usage 时的 token 估算启发式(中文文本 ≈ 4 字符/token)
pub const CHARS_PER_TOKEN: u64 = 4;

/// 压力阈值 token 数(自动折叠触发线;`window` = 当前模型上下文窗口)
pub fn threshold_tokens(window: u64) -> u64 {
    (window as f64 * THRESHOLD_RATIO) as u64
}

/// 保留尾预算 token 数(最近上下文逐字保留的下限;`window` 同上)
pub fn retain_tokens(window: u64) -> u64 {
    (window as f64 * RETAIN_RATIO) as u64
}

/// 当前上下文量测:优先最近一次 LLM 请求的真实 usage
/// (audit/call boundary=llm operation=request-done 的归一 input_tokens),
/// 无则退化为派生字符数÷4 启发式。
pub fn measure_tokens(events: &[EventEnvelope], derived_chars: u64) -> u64 {
    events
        .iter()
        .rev()
        .find(|e| {
            e.r#type == "audit/call"
                && e.data["boundary"] == "llm"
                && e.data["operation"] == "request-done"
        })
        .and_then(|e| e.data["detail"]["usage"]["input_tokens"].as_u64())
        .filter(|t| *t > 0)
        .unwrap_or(derived_chars / CHARS_PER_TOKEN)
}

/// 选出的压缩区间(模型可见面连续头部)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactRange {
    /// 折叠终点:此 seq(含)之前的消息面事件全部进摘要
    pub through_seq: u64,
    /// 折叠遮蔽区间的首条消息事件 seq(退化 = through_seq)
    pub shadowed_start: u64,
    /// 本次折叠的 live 消息条数(上次 checkpoint 之后的消息面事件)
    pub fold_len: usize,
    /// 待摘要前缀在派生面(可见消息数组)里的长度——engine 按它切片。
    /// `prev_through > 0` 时派生面头部多一条旧 checkpoint 占位,故
    /// `prefix_len = fold_len + 1`:前缀末条 == `through_seq` 指向的
    /// 消息;若切 `fold_len` 则丢尾条(旧实现在二次压缩时即此缺陷)。
    pub prefix_len: usize,
    /// 折叠前缀的估算 token(字符÷4;UI 通告与日志载荷用)
    pub estimated_tokens: u64,
}

/// 折叠价值裁定的候选(一个待裁定的旧工具输出)。
///
/// DTO 是本 crate 的公共词汇:裁定端口在 `liuma-agent-loop`(转引),
/// 实现方在宿主侧——两侧都不需要为此牵进决策协议类型。
#[derive(Debug, Clone, PartialEq)]
pub struct ValueCandidate {
    /// tool/result 事件 seq(裁定引用与 `decision/pruned` 的锚点)
    pub seq: u64,
    /// 输出字符数(排序键:大输出优先裁定,省 token 收益最大)
    pub chars: usize,
    /// 输出预览(裁定 state;截断到策略给的 preview_chars)
    pub preview: String,
}

/// 折叠价值候选选择(纯函数):**折叠区间内**的 `tool/result`、长度
/// 达标、未被历史 `decision/pruned` 引用过(裁定幂等)。
///
/// 区间 = `[shadowed_start, through_seq]`(含两端),退化形态与
/// `compaction/summary` 的 `shadowedRange` 同式(端取 through_seq)——
/// 候选必须落在本次要折叠的前缀里:保留尾内模型正在用的输出不动,
/// 区间外的更动不了(checkpoint 只吃前缀)。
///
/// **不截断**:条数上限由裁定方按策略裁(「评估 M / 共 N」的分母须诚实
/// ——截断在这里发生,调用方就看不到 N 了)。大输出优先排序。
pub fn select_value_candidates<'a>(
    events: impl Iterator<Item = &'a EventEnvelope>,
    range: &CompactRange,
    already_pruned: &std::collections::HashSet<u64>,
    min_chars: usize,
    preview_chars: usize,
) -> Vec<ValueCandidate> {
    let start = if range.shadowed_start == 0 {
        range.through_seq
    } else {
        range.shadowed_start
    };
    let mut out: Vec<ValueCandidate> = events
        .into_iter()
        .filter(|ev| ev.r#type == "tool/result" && ev.seq >= start && ev.seq <= range.through_seq)
        .filter(|ev| !already_pruned.contains(&ev.seq))
        .filter_map(|ev| {
            let output = ev.data["output"].as_str()?;
            let chars = output.chars().count();
            (chars >= min_chars).then(|| ValueCandidate {
                seq: ev.seq,
                chars,
                preview: output.chars().take(preview_chars).collect(),
            })
        })
        .collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.chars));
    out
}

/// 消息面事件的 tool 配对增量:assistant/message 带 tool_calls 计 +
/// N,tool/result 计 −1,其余 0。
fn pairing_delta(ty: &str, data: &serde_json::Value) -> i64 {
    match ty {
        "assistant/message" => data["tool_calls"].as_array().map_or(0, |a| a.len() as i64),
        "tool/result" => -1,
        _ => 0,
    }
}

/// 消息体 token 估算(字符÷4)
fn estimate_tokens(ty: &str, data: &serde_json::Value) -> u64 {
    liuma_session::message_from_event(ty, data)
        .map(|m| m.to_string().chars().count() as u64)
        .unwrap_or(0)
        / CHARS_PER_TOKEN
}

/// 保留尾策略(两入口共用 [`select_range_with`] 的实现)
#[derive(Clone, Copy)]
enum Tail {
    /// 自尾累计到预算 token(自动折叠)
    Budget(u64),
    /// 预算优先;预算吞掉全部 live 历史时退回「保留当前这一轮」(手动)
    BudgetOrCurrentTurn(u64),
}

/// 选段:在「最近一次折叠之后」的消息面事件上,自尾倒序累计估算
/// token(字符÷4)到 `retain` 得保留尾,再把切点向回走到 tool 配对
/// 平衡处(切点前的调用全部已有结果)。保留尾不足以离开区间头、或
/// 回退后无平衡切点 → None(无可压缩)。
///
/// 返回的 [`CompactRange::prefix_len`] 是派生面切片的权威长度
/// (含旧 checkpoint 占位);[`CompactRange::fold_len`] 只计本次折叠
/// 的 live 消息数(UI 统计)。
pub fn select_range(events: &[EventEnvelope], retain: u64) -> Option<CompactRange> {
    select_range_with(events, Tail::Budget(retain))
}

/// 选段(手动 /compact):除保留尾的兜底外与 [`select_range`] 同规则。
/// 手动路径「显式要求即压」,而预算本身不该成为拒绝的理由:会话还没
/// 长到窗口占比(1M 窗口 = 16 万 token)时,预算把整段历史都算进保留
/// 尾,自动路径据此不动是对的,手动路径据此回「暂无可压缩的历史」则
/// 与显式要求矛盾(真机:8% 用量的会话按压缩无反应)。故预算落空时
/// 退回「保留当前这一轮」——折叠到最后一条**真实** user 消息之前,
/// 其后一切(含注入上下文)照常逐字保留;该边界之前无内容可折 → 仍 None。
pub fn select_range_manual(events: &[EventEnvelope], retain: u64) -> Option<CompactRange> {
    select_range_with(events, Tail::BudgetOrCurrentTurn(retain))
}

fn select_range_with(events: &[EventEnvelope], tail: Tail) -> Option<CompactRange> {
    let prev_through = events
        .iter()
        .rev()
        .find(|e| e.r#type == "compaction/summary")
        .and_then(|e| e.data["throughSeq"].as_u64())
        .unwrap_or(0);
    let live: Vec<&EventEnvelope> = events.iter().filter(|e| e.seq > prev_through).collect();

    // 消息面事件(与派生面同一判定谓词)+ 逐切点配对平衡
    let mut msg_idx: Vec<usize> = Vec::new();
    let mut msg_tokens: Vec<u64> = Vec::new();
    let mut balanced: Vec<bool> = Vec::new();
    let mut in_progress: i64 = 0;
    for ev in &live {
        if liuma_session::message_from_event(&ev.r#type, &ev.data).is_some() {
            msg_idx.push(balanced.len());
            msg_tokens.push(estimate_tokens(&ev.r#type, &ev.data));
        }
        in_progress += pairing_delta(&ev.r#type, &ev.data);
        balanced.push(in_progress == 0);
    }
    if msg_idx.is_empty() {
        return None;
    }

    let keep_from = match tail {
        Tail::Budget(retain) => budget_tail_start(&msg_tokens, retain)?,
        Tail::BudgetOrCurrentTurn(retain) => budget_tail_start(&msg_tokens, retain)
            .or_else(|| current_turn_start(&live, &msg_idx))?,
    };

    // 切点 = 保留尾起点前;回退到配对平衡处(该切点前无未回应用的调用)
    let mut cut = keep_from;
    while cut > 0 && !balanced[msg_idx[cut - 1]] {
        cut -= 1;
    }
    if cut == 0 {
        return None;
    }

    let through_seq = live[msg_idx[cut - 1]].seq;
    let shadowed_start = live[msg_idx[0]].seq;
    let estimated_tokens = msg_tokens[..cut].iter().sum();
    // 派生面头部占位:上次 checkpoint 以合成 user 消息插入头(见
    // liuma_session::derive_visible_messages),摘要前缀须含它——
    // 指令要求「已有 <compacted-summary> 是旧 checkpoint,合并而非丢弃」
    let prior_checkpoint = usize::from(prev_through > 0);
    // 派生面短于消息面:被取代的 skill 目录在派生面没有位置,前缀长要
    // 相应扣减,否则切片会越过切点(多折切点之后的消息——手动兜底路径
    // 下即「把当前这一轮也折进去」)
    let superseded = superseded_catalogs(&live, &msg_idx, cut);
    Some(CompactRange {
        through_seq,
        shadowed_start,
        fold_len: cut,
        prefix_len: cut + prior_checkpoint - superseded,
        estimated_tokens,
    })
}

/// 自尾倒序累计消息 token 到保留预算 → 保留尾起点(消息序下标);
/// 整段历史都在预算内(累计未及预算即耗尽)→ None
fn budget_tail_start(msg_tokens: &[u64], retain: u64) -> Option<usize> {
    let mut accumulated: u64 = 0;
    for (k, &t) in msg_tokens.iter().enumerate().rev() {
        accumulated += t;
        if accumulated >= retain {
            // k = 0:保留尾吞掉整段历史 → 无可压缩(调用方另有兜底策略)
            return (k > 0).then_some(k);
        }
    }
    None
}

/// 当前这轮起点 = 最后一条**真实** user 消息(注入上下文——目录/插件/
/// 指令——不算一轮,来源判定同 trajectory/projection:source.kind 缺省
/// 或 "user")。取其消息序下标为保留尾起点,即折叠它之前的一切;它之后
/// 注入的消息随之逐字保留,不会被折掉。
fn current_turn_start(live: &[&EventEnvelope], msg_idx: &[usize]) -> Option<usize> {
    (0..msg_idx.len()).rev().find(|&k| {
        let ev = live[msg_idx[k]];
        ev.r#type == "user/message"
            && ev.data["source"]["kind"].as_str().unwrap_or("user") == "user"
    })
}

/// 切点前被取代的 skill 目录条数(派生面「只保留最新一条」的策略③):
/// 这些消息在派生面没有对应条目,故不计入前缀长
fn superseded_catalogs(live: &[&EventEnvelope], msg_idx: &[usize], cut: usize) -> usize {
    let Some(last_catalog_seq) = live
        .iter()
        .rev()
        .find(|e| e.r#type == "user/message" && liuma_session::is_skill_catalog(&e.data))
        .map(|e| e.seq)
    else {
        return 0;
    };
    (0..cut)
        .filter(|&k| {
            let ev = live[msg_idx[k]];
            ev.seq < last_catalog_seq
                && ev.r#type == "user/message"
                && liuma_session::is_skill_catalog(&ev.data)
        })
        .count()
}

/// 摘要指令:以最终 user 消息追加
/// 在逐字前缀之后——前缀复用上次路由请求的 system/tools/消息形态,
/// 命中 provider KV cache。
pub const COMPACTION_INSTRUCTION: &str = r#"You are now acting as a compaction engine for this AI coding assistant. Condense the conversation ABOVE into a structured checkpoint that lets another model resume the work with no loss of essential context.

Output EXACTLY the Markdown structure below: keep every section, in order. Use terse bullets, not prose paragraphs. Write "(none)" for an empty section — never drop a section.

## Primary Request and Intent
- [the user's original and evolving goals; quote verbatim where the exact wording matters]

## Key Technical Concepts
- [technologies, frameworks, patterns, and conventions in play]

## Files and Code
- [exact path: why it matters, key changes or snippets]

## Errors and Fixes
- [error: how it was resolved, plus any related user feedback]

## Pending Jobs
- [explicitly requested work not yet completed]

## Current Work
- [precisely what was in progress at this checkpoint]

## Next Step
- [the single next action, directly in line with the most recent request, or "(none)"]

## Critical Context
- [decisions and their rationale, constraints, user preferences, open questions, data needed to continue]

Rules:
- Write concise English engineering prose. Preserve exact file paths, commands, error strings, identifiers, numeric values, function signatures, and syntax fragments.
- Capture user feedback and explicit instructions faithfully, especially corrections.
- Do NOT mention this summarization request or that the context was compacted.
- Output only the checkpoint text: do not call any tool or take any other action.
- If the conversation already contains a <compacted-summary> block, it is a PRIOR checkpoint. Do not copy it forward verbatim: preserve still-true facts, drop stale ones, and merge newer information into a single consolidated summary under the same structure."#;

/// 构造摘要请求的完整输入:逐字前缀 + 追加含指令的最终 user 消息。
pub fn summarization_messages(fold_messages: &serde_json::Value) -> serde_json::Value {
    let mut msgs = fold_messages.as_array().cloned().unwrap_or_default();
    msgs.push(serde_json::json!({
        "role": "user",
        "content": COMPACTION_INSTRUCTION,
    }));
    serde_json::Value::Array(msgs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use liuma_session::{EventLog, derive_visible_messages, message_from_event};

    /// 造带 seq 的事件:经 EventLog append 分配连续 seq
    fn logged(events: &[(&str, serde_json::Value)]) -> Vec<EventEnvelope> {
        let mut log = EventLog::new();
        for (ty, data) in events {
            log.append(EventEnvelope::new(ty, 0, data.clone()))
                .expect("append");
        }
        log.iter().cloned().collect()
    }

    fn user_msg(text: &str) -> (&'static str, serde_json::Value) {
        ("user/message", serde_json::json!({ "content": text }))
    }

    fn assistant_msg(text: &str) -> (&'static str, serde_json::Value) {
        ("assistant/message", serde_json::json!({ "content": text }))
    }

    fn big_user(kchars: usize) -> (&'static str, serde_json::Value) {
        user_msg(&"x".repeat(kchars * 1000))
    }

    #[test]
    fn empty_log_selects_nothing() {
        let all = logged(&[]);
        assert_eq!(select_range(&all, 1), None);
    }

    #[test]
    fn below_retain_selects_nothing() {
        // 全部消息估算 token < retain(160K)→ 无可压缩(小会话 no-op)
        let all = logged(&[user_msg("hi"), assistant_msg("hello")]);
        assert_eq!(
            select_range(&all, retain_tokens(DEFAULT_CONTEXT_WINDOW)),
            None
        );
    }

    /// 目录(注入上下文)事件:source.kind=skill-catalog
    fn catalog(text: &str) -> (&'static str, serde_json::Value) {
        (
            "user/message",
            serde_json::json!({ "content": text, "source": { "kind": "skill-catalog" } }),
        )
    }

    /// 手动兜底锁(真机回归:1M 窗口下头 16 万 token 内 /compact 恒回
    /// 「暂无可压缩的历史」):预算吞掉全部 live 历史时,自动路径不动,
    /// 手动路径折叠到最后一条真实 user 消息之前(保留当前这一轮)
    #[test]
    fn manual_folds_to_current_turn_when_below_retain() {
        let all = logged(&[
            user_msg("q1"),
            assistant_msg("a1"),
            user_msg("q2"),
            assistant_msg("a2"),
            user_msg("q3"),
        ]);
        let retain = retain_tokens(DEFAULT_CONTEXT_WINDOW);
        assert_eq!(select_range(&all, retain), None, "自动路径:预算未越不动");
        let r = select_range_manual(&all, retain).expect("手动应压到当前轮之前");
        assert_eq!(r.fold_len, 4, "折 q1/a1/q2/a2;当前轮 q3 留下");
        assert_eq!(r.through_seq, 4);
        assert_eq!(r.prefix_len, 4, "无旧 checkpoint → 切点即前缀长");
        assert!(r.estimated_tokens > 0);
        // 派生面切片:保留尾首条正是当前轮的 q3
        let visible = derive_visible_messages(all.iter());
        let arr = visible.as_array().expect("数组");
        assert_eq!(arr[r.prefix_len]["content"], "q3");
        assert_eq!(arr.len(), 5, "前缀 + 保留尾 = 全部");
    }

    /// 手动兜底不越过「当前这一轮」:只有一轮(或边界之前没有内容)
    /// 仍回 None —— 「暂无可压缩的历史」在真的没得压时依然成立
    #[test]
    fn manual_without_a_prior_turn_selects_nothing() {
        let retain = retain_tokens(DEFAULT_CONTEXT_WINDOW);
        let one_turn = logged(&[user_msg("hi"), assistant_msg("hello")]);
        assert_eq!(select_range_manual(&one_turn, retain), None, "单轮无可折");
        // 会话开头就是注入上下文 + 当前轮:边界前只有注入,仍算有内容可折
        let injected_first = logged(&[catalog("<cat>"), user_msg("q1"), assistant_msg("a1")]);
        let r = select_range_manual(&injected_first, retain).expect("注入在边界前 → 可折");
        assert_eq!(r.fold_len, 1);
    }

    /// 手动不是「一律更激进」:历史已超预算时与自动同一切点(保留尾
    /// 照旧按窗口占比留)
    #[test]
    fn manual_keeps_the_budget_tail_when_history_exceeds_it() {
        let all = logged(&[
            big_user(500),
            assistant_msg("a"),
            big_user(500),
            assistant_msg("b"),
            big_user(500),
            assistant_msg("c"),
        ]);
        let retain = 100_000;
        assert!(select_range(&all, retain).is_some());
        assert_eq!(
            select_range_manual(&all, retain),
            select_range(&all, retain)
        );
    }

    /// 边界 = 最后一条**真实** user 消息:注入上下文(目录)不算一轮,
    /// 故它之后的保留尾从真实轮起;切点前的被取代目录在派生面无位置,
    /// 前缀长按条数扣减(否则切片越过切点,把当前轮也折进去)
    #[test]
    fn manual_boundary_skips_injected_and_counts_superseded() {
        let retain = retain_tokens(DEFAULT_CONTEXT_WINDOW);
        // 注入目录在末位:不算一轮,随保留尾留下
        let tail_catalog = logged(&[
            user_msg("q1"),
            assistant_msg("a1"),
            user_msg("q2"),
            assistant_msg("a2"),
            catalog("<cat>"),
        ]);
        let r = select_range_manual(&tail_catalog, retain).expect("range");
        assert_eq!(r.fold_len, 2, "折 q1/a1;q2 起保留(含其后的目录)");
        let visible = derive_visible_messages(tail_catalog.iter());
        assert_eq!(
            visible.as_array().expect("数组")[r.prefix_len]["content"],
            "q2"
        );

        // 切点前有被取代目录:派生面少一条,prefix_len 须扣减
        let superseded = logged(&[
            user_msg("q1"),
            assistant_msg("a1"),
            catalog("<cat-1>"),
            user_msg("q2"),
            assistant_msg("a2"),
            catalog("<cat-2>"),
            user_msg("q3"),
        ]);
        let r = select_range_manual(&superseded, retain).expect("range");
        assert_eq!(r.fold_len, 6, "折到 q3 之前(含两条目录)");
        let visible = derive_visible_messages(superseded.iter());
        let arr = visible.as_array().expect("数组");
        assert_eq!(arr.len(), 6, "旧目录在派生面无位置");
        assert_eq!(
            arr[r.prefix_len]["content"], "q3",
            "切片必须正好停在当前轮(prefix_len 未扣减时此处是 a2)"
        );
    }

    #[test]
    fn big_history_selects_head_prefix() {
        // 自尾累计到 retain(100K):最后一条大消息(≈125K)即越过 →
        // 保留尾 = 最后 2 条,前 4 条折叠
        let all = logged(&[
            big_user(500), // ≈125k tok
            assistant_msg("a"),
            big_user(500), // ≈125k tok
            assistant_msg("b"),
            big_user(500), // ≈125k tok
            assistant_msg("c"),
        ]);
        let r = select_range(&all, 100_000).expect("range");
        assert_eq!(r.fold_len, 4);
        assert_eq!(r.prefix_len, r.fold_len, "无旧 checkpoint → 切点即前缀长");
        assert_eq!(r.through_seq, 4, "折叠到第 4 条消息(assistant b)");
        assert_eq!(r.shadowed_start, 1);
        assert!(r.estimated_tokens > 0);
    }

    #[test]
    fn cut_walks_back_to_tool_pairing_balance() {
        // 保留尾起点落在 tool/result 上(其估算 token 越过 retain)→
        // 其前切点在 assistant 调用上不平衡 → 回退到配对平衡处
        let all = logged(&[
            big_user(500), // seq1 ≈125k tok
            (
                "assistant/message",
                serde_json::json!({ "content": "", "tool_calls": [ { "id": "t1" } ] }),
            ), // seq2
            (
                "tool/result",
                serde_json::json!({ "output": "ok" .repeat(500_000 / 2) }),
            ), // seq3 ≈125k tok
            big_user(500), // seq4 ≈125k tok
            assistant_msg("done"), // seq5
        ]);
        // 自尾累计:seq4(125K)<200K → seq3(125K)越过多 retain(200K)
        // → 保留尾起点 = seq3;切点回退:seq2 后余 +1 不平衡 → seq1 后平衡
        let r = select_range(&all, 200_000).expect("range");
        assert_eq!(r.fold_len, 1);
        assert_eq!(r.through_seq, 1);
        assert_eq!(r.shadowed_start, 1);
    }

    #[test]
    fn only_after_last_summary_is_considered() {
        // 已有 compaction/summary:只统计 throughSeq 之后的消息面
        let all = logged(&[
            big_user(500),
            (
                "compaction/summary",
                serde_json::json!({ "summary": "s", "throughSeq": 1 }),
            ),
            assistant_msg("after fold"),
        ]);
        // throughSeq 之后仅 1 条小消息 → 无可压缩
        assert_eq!(
            select_range(&all, retain_tokens(DEFAULT_CONTEXT_WINDOW)),
            None
        );
    }

    /// 回归锁:二次压缩的前缀长度须含派生面头部的旧 checkpoint 占位,
    /// 且前缀末条 == through_seq 指向的消息(engine 按 prefix_len 切片;
    /// 旧实现切 fold_len,二次压缩丢尾条——tool 配对可被拆散)
    #[test]
    fn prefix_len_accounts_for_prior_checkpoint() {
        let all = logged(&[
            big_user(500), // seq1
            (
                "compaction/summary",
                serde_json::json!({ "summary": "prior", "throughSeq": 1 }),
            ),
            big_user(500), // seq3
            assistant_msg("a"),
            big_user(500), // seq5
            assistant_msg("b"),
            big_user(500), // seq7
            assistant_msg("c"),
        ]);
        let r = select_range(&all, 100_000).expect("range");
        assert_eq!(r.prefix_len, r.fold_len + 1, "含头部旧 checkpoint 占位");

        let visible = derive_visible_messages(all.iter());
        let arr = visible.as_array().expect("array");
        let through = all
            .iter()
            .find(|e| e.seq == r.through_seq)
            .expect("through 事件");
        let expected = message_from_event(&through.r#type, &through.data).expect("消息面");
        assert_eq!(
            arr[r.prefix_len - 1],
            expected,
            "前缀末条 = through_seq 指向的消息(切片对齐)"
        );
        assert_ne!(
            arr[r.fold_len - 1],
            expected,
            "切 fold_len 会漏掉尾条(旧缺陷位)"
        );
    }

    #[test]
    fn measure_prefers_real_usage_then_chars_heuristic() {
        let plain = logged(&[user_msg("hi")]);
        assert_eq!(measure_tokens(&plain, 800), 200, "无 usage → 字符÷4");

        let with_usage = logged(&[
            user_msg("hi"),
            (
                "audit/call",
                serde_json::json!({ "boundary": "llm", "operation": "request-done",
                    "detail": { "usage": { "input_tokens": 12345 } } }),
            ),
        ]);
        assert_eq!(measure_tokens(&with_usage, 800), 12345);
    }

    #[test]
    fn summarization_appends_instruction_as_final_user_message() {
        let msgs = serde_json::json!([
            { "role": "user", "content": "q" },
            { "role": "assistant", "content": "a" },
        ]);
        let out = summarization_messages(&msgs);
        let arr = out.as_array().expect("array");
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[2]["role"], "user");
        assert_eq!(arr[2]["content"], COMPACTION_INSTRUCTION);
        // 指令内含结构化八节标题(锚定断言)
        for section in [
            "## Primary Request and Intent",
            "## Files and Code",
            "## Next Step",
            "## Critical Context",
        ] {
            assert!(COMPACTION_INSTRUCTION.contains(section), "{section} 缺席");
        }
    }

    /// 阈值随窗口线性缩放(默认 1M 与 128K 两档锚点)
    #[test]
    fn thresholds_follow_window_ratios() {
        assert_eq!(threshold_tokens(DEFAULT_CONTEXT_WINDOW), 800_000);
        assert_eq!(retain_tokens(DEFAULT_CONTEXT_WINDOW), 160_000);
        // 128K 窗口:阈值 102_400 / 保留尾 20_480(旧硬编码 1M 会晚触发 8 倍)
        assert_eq!(threshold_tokens(128_000), 102_400);
        assert_eq!(retain_tokens(128_000), 20_480);
    }

    /// 造带 tool_calls 的助手消息(配对平衡用)
    fn assistant_calls() -> (&'static str, serde_json::Value) {
        (
            "assistant/message",
            serde_json::json!({ "content": "", "tool_calls": [{ "id": "c1", "name": "bash" }] }),
        )
    }

    /// tool/result(chars 个 x 的输出)
    fn tool_result(chars: usize) -> (&'static str, serde_json::Value) {
        (
            "tool/result",
            serde_json::json!({ "call": 1, "output": "x".repeat(chars) }),
        )
    }

    /// 裁定候选窗口 = 折叠区间(含两端):保留尾内的输出不参选
    /// (模型正在用的不动),也不越到切点之后
    #[test]
    fn value_candidates_stay_inside_the_folded_range() {
        let retain = retain_tokens(DEFAULT_CONTEXT_WINDOW);
        // 折叠区间 [1,4](q1/call/big/a1);保留尾内另有一个大输出
        let all = logged(&[
            user_msg("q1"),
            assistant_calls(),
            tool_result(3_000),
            assistant_msg("a1"),
            user_msg("q2"),
            assistant_calls(),
            tool_result(9_000),
        ]);
        let range = select_range_manual(&all, retain).expect("range");
        assert_eq!((range.shadowed_start, range.through_seq), (1, 4));
        let none = std::collections::HashSet::new();
        let picked = select_value_candidates(all.iter(), &range, &none, 2_000, 600);
        // 保留尾内的 9K 输出(seq7)不参选:区间外
        assert_eq!(picked.iter().map(|c| c.seq).collect::<Vec<_>>(), vec![3]);
        assert_eq!(picked[0].chars, 3_000);
        assert_eq!(picked[0].preview.chars().count(), 600, "预览截断到策略值");

        // 端含入:切点本身是 tool/result 时该条参选
        let ends_on_result = logged(&[
            user_msg("q1"),
            assistant_calls(),
            tool_result(2_500),
            user_msg("q2"),
            assistant_calls(),
            tool_result(8_000),
        ]);
        let range = select_range_manual(&ends_on_result, retain).expect("range");
        assert_eq!((range.shadowed_start, range.through_seq), (1, 3));
        let picked = select_value_candidates(ends_on_result.iter(), &range, &none, 2_000, 600);
        assert_eq!(picked.iter().map(|c| c.seq).collect::<Vec<_>>(), vec![3]);
    }

    /// 候选:最小字符门槛、已裁集合跳过、大输出优先排序
    /// (排序是收益序:条数上限截断时被丢的是收益最小的那些)
    #[test]
    fn value_candidates_filter_and_order_by_size() {
        let range = CompactRange {
            through_seq: 12,
            shadowed_start: 1,
            fold_len: 0,
            prefix_len: 0,
            estimated_tokens: 0,
        };
        // 手造日志:短输出 / 达标 / 已裁 / 未达标边界(恰为门槛)
        let mut log = EventLog::new();
        for chars in [1_999usize, 2_000, 5_000, 9_000, 7_000] {
            log.append(EventEnvelope::new("tool/result", 0, tool_result(chars).1))
                .expect("append");
        }
        let events: Vec<EventEnvelope> = log.iter().cloned().collect();
        let already: std::collections::HashSet<u64> = [4].into(); // 9_000 那条已裁
        let picked = select_value_candidates(events.iter(), &range, &already, 2_000, 600);
        assert_eq!(
            picked.iter().map(|c| (c.seq, c.chars)).collect::<Vec<_>>(),
            vec![(5, 7_000), (3, 5_000), (2, 2_000)],
            "门槛含端点(2_000 入):1_999 出、已裁的 9_000 出、大者在前"
        );
        assert_eq!(
            select_value_candidates(events.iter(), &range, &already, 10_000, 600),
            Vec::new(),
            "门槛之上无候选"
        );
    }

    #[test]
    fn message_predicate_matches_derive() {
        // 选段用的消息判定与派生面同一函数
        let all = logged(&[user_msg("u"), ("tool/call", serde_json::json!({}))]);
        assert!(message_from_event("user/message", &all[0].data).is_some());
        assert!(message_from_event("tool/call", &all[1].data).is_none());
    }
}
