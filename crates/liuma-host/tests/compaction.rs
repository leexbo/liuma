//! compaction e2e:超预算触发历史折叠。
//!
//! 断言链:
//! - audit/call(operation=compaction)先落、compaction/summary 后落(记录优先);
//! - 折叠后出网请求 = 摘要消息 + 保留尾部(闸门全程通过——期望侧同一投影);
//! - 重放不重调:新 engine 共享日志再跑 turn,无新 summarize 调用、
//!   摘要消息仍可见(确定性重放);
//! - tool/result 裁剪:超阈值输出在请求中被截断而日志保留全文。

use std::sync::{Arc, Mutex};

use liuma_agent_loop::{LlmEvent, LoopEngine, NoTools, RequestHeader};
use liuma_llm::{FakeProvider, InvariantGate};
use liuma_session::{EventEnvelope, EventLog};
use serde_json::json;

fn header() -> RequestHeader {
    RequestHeader {
        model: "t".into(),
        system: String::new(),
        temperature: 0.0,
        reasoning_effort: None,
        tools: Vec::new(),
    }
}

/// 预置一个大历史会话日志(10 组超长工具往返)
fn big_log() -> Arc<Mutex<EventLog>> {
    let log = Arc::new(Mutex::new(EventLog::new()));
    {
        let mut l = log.lock().unwrap();
        for i in 0..10 {
            l.append(EventEnvelope::new(
                "user/message",
                0,
                json!({ "content": format!("question {i}: {}", "q".repeat(600)) }),
            ))
            .unwrap();
            l.append(EventEnvelope::new(
                "assistant/message",
                0,
                json!({ "content": format!("answer {i}") }),
            ))
            .unwrap();
        }
    }
    log
}

#[tokio::test]
async fn fold_triggers_records_and_replays_without_recall() {
    let log = big_log();
    let mut engine = LoopEngine::new(header(), Arc::clone(&log));
    // 阈值压到 1 token:首 step 必触发折叠(保留尾也压到 1)
    engine.set_fold_thresholds(1, 1);

    let mut provider = FakeProvider::new();
    provider.summaries.push("condensed history".into());
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "ok"
    }))]);
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut sink = |_ev: &EventEnvelope| {};

    engine
        .run_turn(
            "continue",
            None,
            &[],
            &[],
            &[],
            &mut gate,
            &mut NoTools,
            &|| 0_i64,
            &mut sink,
        )
        .await
        .expect("turn");

    // 记录优先:摘要审计先于 compaction/summary 落档
    let (audit_seq, summary_seq, summary_text, through) = {
        let l = log.lock().unwrap();
        let audit = l
            .iter()
            .find(|e| e.r#type == "audit/call" && e.data["operation"] == "compaction")
            .expect("compaction 审计");
        let summary = l
            .iter()
            .find(|e| e.r#type == "compaction/summary")
            .expect("compaction/summary");
        (
            audit.seq,
            summary.seq,
            summary.data["summary"].clone(),
            summary.data["throughSeq"].as_u64().unwrap_or(0),
        )
    };
    assert!(audit_seq < summary_seq, "审计先于折叠事件落档");
    assert_eq!(summary_text, "condensed history");
    assert!(through > 0);

    // 出网请求:含摘要消息,且长度显著小于全历史(闸门比对已通过)
    let (_, messages) = &gate.inner().received[0];
    let s = messages.to_string();
    assert!(s.contains("<compacted-summary>"), "请求首条为摘要消息");
    assert!(s.contains("condensed history"));
    assert!(s.chars().count() < 3000, "折叠后请求应远小于全历史");

    // 重放不重调:新 engine(同日志,预算放大避免新折叠),摘要消息
    // 必须来自日志记录而非重新 summarize
    let mut engine2 = LoopEngine::new(header(), Arc::clone(&log));
    engine2.set_fold_thresholds(u64::MAX, u64::MAX);
    let mut provider2 = FakeProvider::new();
    provider2.summaries.push("SHOULD-NOT-BE-CALLED".into());
    provider2.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "ok2"
    }))]);
    let mut gate2 = InvariantGate::new(provider2, Arc::clone(&log));
    let mut sink2 = |_ev: &EventEnvelope| {};
    engine2
        .run_turn(
            "again",
            None,
            &[],
            &[],
            &[],
            &mut gate2,
            &mut NoTools,
            &|| 0_i64,
            &mut sink2,
        )
        .await
        .expect("replay turn");
    assert_eq!(
        gate2.inner().summaries.len(),
        1,
        "已折叠历史不得重调摘要(读记录;脚本应原封未动)"
    );
    assert_eq!(gate2.inner().summaries[0], "SHOULD-NOT-BE-CALLED");
    let (_, m2) = &gate2.inner().received[0];
    assert!(m2.to_string().contains("condensed history"));
}

#[tokio::test]
async fn oversized_tool_output_pruned_in_request_kept_in_log() {
    let log = Arc::new(Mutex::new(EventLog::new()));
    {
        let mut l = log.lock().unwrap();
        l.append(EventEnvelope::new(
            "user/message",
            0,
            json!({ "content": "run it" }),
        ))
        .unwrap();
        l.append(EventEnvelope::new(
            "assistant/message",
            0,
            json!({ "content": "", "tool_calls": [ { "name": "bash", "arguments": {} } ] }),
        ))
        .unwrap();
        l.append(EventEnvelope::new(
            "tool/result",
            0,
            json!({ "call": 3, "id": "", "output": "o".repeat(20_000), "success": true }),
        ))
        .unwrap();
    }

    let mut provider = FakeProvider::new();
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "done"
    }))]);
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut engine = LoopEngine::new(header(), Arc::clone(&log));
    let mut sink = |_ev: &EventEnvelope| {};
    engine
        .run_turn(
            "next",
            None,
            &[],
            &[],
            &[],
            &mut gate,
            &mut NoTools,
            &|| 0_i64,
            &mut sink,
        )
        .await
        .expect("turn");

    // 请求侧:裁剪(闸门期望侧同一投影,比对通过即证明一致)
    let (_, messages) = &gate.inner().received[0];
    let s = messages.to_string();
    assert!(s.contains("[pruned "));
    assert!(s.chars().count() < 12_000);
    // 日志侧:全文保留(审计保真)
    let l = log.lock().unwrap();
    let tr = l
        .iter()
        .find(|e| e.r#type == "tool/result")
        .expect("tool/result");
    assert_eq!(tr.data["output"].as_str().unwrap().len(), 20_000);
}

/// 手动压缩(/compact 引擎面):无压力阈值门槛即压;summary 落档带
/// 统计载荷;二调无可压缩且不重调 summarize(重放确定性)。
#[tokio::test]
async fn manual_compact_forces_summary_and_replays_without_recall() {
    use liuma_agent_loop::FoldOutcome;

    let log = big_log();
    let mut engine = LoopEngine::new(header(), Arc::clone(&log));
    // 手动路径无压力阈值门槛(compact_now 不看 threshold),但保留尾
    // 预算取引擎 retain——压到 1 使小会话也能压出前缀
    engine.set_fold_thresholds(0, 1);

    let mut provider = FakeProvider::new();
    provider.summaries.push("manual condensed".into());
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut sink = |_ev: &EventEnvelope| {};

    let outcome = engine
        .compact_now(&mut gate, &|| 0_i64, &mut sink)
        .await
        .expect("compact");
    let FoldOutcome::Folded { seq, items, tokens } = outcome else {
        panic!("大会话手动压缩应折叠落档");
    };
    assert!(seq > 0);
    assert!(items > 0);
    assert!(tokens > 0);

    // 落档载荷:summary + 统计(UI 标记行素材)
    {
        let l = log.lock().unwrap();
        let summary = l
            .iter()
            .find(|e| e.r#type == "compaction/summary")
            .expect("compaction/summary");
        assert_eq!(summary.data["summary"], "manual condensed");
        assert_eq!(summary.data["items"], items);
        assert_eq!(summary.data["shadowedTokens"], tokens);
        // 派生面:摘要以 checkpoint 包装进请求前缀
        let visible = liuma_session::derive_visible_messages(l.iter());
        let first = &visible.as_array().unwrap()[0];
        assert!(
            first["content"]
                .as_str()
                .unwrap()
                .contains("This is an automatically generated checkpoint")
                && first["content"]
                    .as_str()
                    .unwrap()
                    .contains("<compacted-summary>\nmanual condensed"),
            "首条消息 = checkpoint 包装的摘要"
        );
    }

    // 二调:保留尾不足 retain → Skipped,且 summaries 已空(未重调;
    // 若重调,FakeProvider 会回退 "[fake summary]" 亦无从进入此分支)
    let outcome2 = engine
        .compact_now(&mut gate, &|| 0_i64, &mut sink)
        .await
        .expect("compact 2");
    assert_eq!(outcome2, FoldOutcome::Skipped);
    let l = log.lock().unwrap();
    assert_eq!(
        l.iter()
            .filter(|e| e.r#type == "compaction/summary")
            .count(),
        1,
        "二调不追加 summary(重放确定性)"
    );
}

/// 小历史手动压缩回归锁(真机:1M 窗口 → 保留尾预算 = 0.16×1M = 16 万
/// token,会话 8% 用量按压缩恒回「暂无可压缩的历史」)。**默认装配**
/// (不调 set_fold_thresholds)下,小历史也必须折叠落档,且只折到当前
/// 这一轮之前。修复前:select_range 见全量不足保留尾即 None → Skipped。
#[tokio::test]
async fn manual_compact_folds_small_history_to_current_turn() {
    use liuma_agent_loop::FoldOutcome;

    // 三轮小消息 + 当前这一轮(估算 token 远低于默认保留尾 16 万)
    let log = Arc::new(Mutex::new(EventLog::new()));
    {
        let mut l = log.lock().unwrap();
        for i in 0..3 {
            l.append(EventEnvelope::new(
                "user/message",
                0,
                json!({ "content": format!("q{i}") }),
            ))
            .unwrap();
            l.append(EventEnvelope::new(
                "assistant/message",
                0,
                json!({ "content": format!("a{i}") }),
            ))
            .unwrap();
        }
        l.append(EventEnvelope::new(
            "user/message",
            0,
            json!({ "content": "q3" }),
        ))
        .unwrap();
    }
    let mut engine = LoopEngine::new(header(), Arc::clone(&log));
    // 刻意不设阈值/保留尾:走真实窗口占比(默认 1M 窗口 = 保留尾 16 万)
    let mut provider = FakeProvider::new();
    provider.summaries.push("small condensed".into());
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut sink = |_ev: &EventEnvelope| {};

    let outcome = engine
        .compact_now(&mut gate, &|| 0_i64, &mut sink)
        .await
        .expect("compact");
    let FoldOutcome::Folded { items, .. } = outcome else {
        panic!("小历史手动压缩应折叠落档(修复前回 Skipped)");
    };
    assert_eq!(items, 6, "折 q0/a0..q2/a2 共 6 条;当前这一轮 q3 留下");
    {
        let l = log.lock().unwrap();
        let visible = liuma_session::derive_visible_messages(l.iter());
        let arr = visible.as_array().expect("数组");
        assert_eq!(arr.len(), 2, "派生面 = checkpoint + 当前这一轮");
        assert!(
            arr[0]["content"]
                .as_str()
                .unwrap()
                .contains("<compacted-summary>\nsmall condensed"),
            "首条 = checkpoint 包装的摘要:{:?}",
            arr[0]["content"]
        );
        assert_eq!(arr[1]["content"], "q3", "保留尾首条 = 当前这一轮");
    }
    // 二调:边界之前已无内容可折 → Skipped(不重调 summarize)
    let outcome2 = engine
        .compact_now(&mut gate, &|| 0_i64, &mut sink)
        .await
        .expect("compact 2");
    assert_eq!(outcome2, FoldOutcome::Skipped);
}

/// 回归锁:同一会话第二次压缩——摘要前缀必须覆盖 `throughSeq` 指向的
/// 尾条消息,且 tool 往返完整。旧实现切派生面 `arr[..fold_len]`,而
/// 派生面头部多一条旧 checkpoint 占位,于是前缀漏掉尾条:本例尾条是
/// tool/result,漏掉后摘要请求以 assistant tool_calls 悬空收尾(provider
/// 直接拒)。锁定 `liuma_compaction::CompactRange::prefix_len` 的切片语义。
#[tokio::test]
async fn second_compaction_prefix_covers_through_seq_message() {
    use liuma_agent_loop::FoldOutcome;

    let log = Arc::new(Mutex::new(EventLog::new()));
    {
        let mut l = log.lock().unwrap();
        let mut push = |ty: &str, data: serde_json::Value| {
            l.append(EventEnvelope::new(ty, 0, data)).unwrap();
        };
        push(
            "user/message",
            json!({ "content": format!("q1: {}", "x".repeat(4000)) }),
        );
        push("assistant/message", json!({ "content": "a1" }));
        push(
            "user/message",
            json!({ "content": format!("q2: {}", "x".repeat(4000)) }),
        );
        push("assistant/message", json!({ "content": "a2" }));
    }

    let mut engine = LoopEngine::new(header(), Arc::clone(&log));
    // 手动路径无压力门槛;保留尾压到 1 token → 每次折叠只留最后一条
    engine.set_fold_thresholds(0, 1);
    let mut provider = FakeProvider::new();
    provider.summaries.push("first".into());
    provider.summaries.push("second".into());
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut sink = |_ev: &EventEnvelope| {};

    let first = engine
        .compact_now(&mut gate, &|| 0_i64, &mut sink)
        .await
        .expect("first compact");
    assert!(matches!(first, FoldOutcome::Folded { .. }));

    // 追加一轮工具往返 + 尾部 user(q4 为保留尾)
    {
        let mut l = log.lock().unwrap();
        let mut push = |ty: &str, data: serde_json::Value| {
            l.append(EventEnvelope::new(ty, 0, data)).unwrap();
        };
        push(
            "user/message",
            json!({ "content": format!("q3: {}", "x".repeat(4000)) }),
        );
        push(
            "assistant/message",
            json!({ "content": "", "tool_calls": [ { "id": "t1", "name": "bash", "arguments": {} } ] }),
        );
        push(
            "tool/result",
            json!({ "call": 6, "id": "t1", "output": "r", "success": true }),
        );
        push(
            "user/message",
            json!({ "content": format!("q4: {}", "x".repeat(4000)) }),
        );
    }

    let second = engine
        .compact_now(&mut gate, &|| 0_i64, &mut sink)
        .await
        .expect("second compact");
    let FoldOutcome::Folded { items, .. } = second else {
        panic!("二次压缩应折叠(仍有可压前缀)");
    };
    assert_eq!(items, 4, "live 前缀 = a2/q3/assistant(tool_calls)/tool");

    let through_seq = {
        let l = log.lock().unwrap();
        l.iter()
            .rfind(|e| e.r#type == "compaction/summary")
            .expect("第二次 compaction/summary")
            .data["throughSeq"]
            .as_u64()
            .unwrap_or(0)
    };

    // 第二次摘要输入(逐字前缀)的末条 == throughSeq 指向的消息
    let inputs = &gate.inner().summary_inputs;
    assert_eq!(inputs.len(), 2, "两次压缩各一次 summarize");
    let prefix = inputs[1].1.as_array().expect("前缀数组");
    let expected = {
        let l = log.lock().unwrap();
        let ev = l
            .iter()
            .find(|e| e.seq == through_seq)
            .expect("throughSeq 事件");
        liuma_session::message_from_event(&ev.r#type, &ev.data).expect("消息面")
    };
    assert_eq!(
        prefix.last(),
        Some(&expected),
        "前缀末条 = throughSeq 指向的消息(旧实现切 fold_len 时缺此条)"
    );
    // 尾条是 tool/result → 其前的 assistant tool_calls 已配对,无悬空
    assert_eq!(prefix.last().unwrap()["role"], "tool");
    assert_eq!(prefix.last().unwrap()["id"], "t1");
    assert_eq!(prefix[prefix.len() - 2]["tool_calls"][0]["id"], "t1");
    // 头部占位 = 上一次 checkpoint(prefix_len 已把该偏移算入)
    assert!(
        prefix[0]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("<compacted-summary>\nfirst"),
        "前缀头部 = 上次 checkpoint(合并语义)"
    );
}

/// 压缩进度相位(真流量):开始(summarize,0)→ 节流增量 → commit →
/// done;字符单调不减、恰好一条终局、序号夹在 audit 与 summary 之间。
/// 回归锁:自动路径此前对桌面完全不可见(无开始信号),失败更是静默。
#[tokio::test]
async fn fold_emits_progress_phases_with_monotone_chars() {
    let log = big_log();
    let mut engine = LoopEngine::new(header(), Arc::clone(&log));
    engine.set_fold_thresholds(1, 1);

    let mut provider = FakeProvider::new();
    // 长摘要:确保节流阈值(≥64 字符)被真实触发,而不只是首末两条
    provider.summaries.push("s".repeat(300));
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "ok"
    }))]);
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut seen: Vec<EventEnvelope> = Vec::new();
    {
        let mut sink = |ev: &EventEnvelope| seen.push(ev.clone());
        engine
            .run_turn(
                "continue",
                None,
                &[],
                &[],
                &[],
                &mut gate,
                &mut NoTools,
                &|| 0_i64,
                &mut sink,
            )
            .await
            .expect("turn");
    }

    let progress: Vec<&EventEnvelope> = seen
        .iter()
        .filter(|e| e.r#type == "compaction/progress")
        .collect();
    assert!(progress.len() >= 4, "至少 开始 + 增量 + commit + done");
    // 相位序:summarize* → commit → done
    let phases: Vec<&str> = progress
        .iter()
        .map(|e| e.data["phase"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(phases[0], "summarize", "首条即开始信号");
    assert_eq!(phases[phases.len() - 2], "commit");
    assert_eq!(phases[phases.len() - 1], "done");
    assert!(
        phases
            .iter()
            .filter(|p| **p == "done" || **p == "failed")
            .count()
            == 1,
        "恰好一条终局: {phases:?}"
    );
    // 字符数单调不减,且增量条真实推进(非只有 0)
    let chars: Vec<u64> = progress
        .iter()
        .map(|e| e.data["generatedChars"].as_u64().unwrap_or(0))
        .collect();
    assert!(
        chars.windows(2).all(|w| w[0] <= w[1]),
        "单调不减: {chars:?}"
    );
    assert!(
        chars[0] == 0 && chars.contains(&300),
        "首条 0、终局带真实总量: {chars:?}"
    );
    // 载荷:自动路径 manual=false,带折中区间与条数
    assert_eq!(progress[0].data["manual"], json!(false));
    assert!(progress[0].data["throughSeq"].as_u64().unwrap_or(0) > 0);
    assert!(progress[0].data["items"].as_u64().unwrap_or(0) > 0);
    assert!(
        progress[0].data["estimatedTokens"].as_u64().unwrap_or(0) > 0,
        "进度分母(前缀估算)随首条进度到达,UI 不必等 summary"
    );
    // 事件序:audit 先于首条进度;summary 在 commit 与 done 之间
    let seqs: Vec<u64> = {
        let l = log.lock().unwrap();
        vec![
            l.iter()
                .find(|e| e.r#type == "audit/call" && e.data["operation"] == "compaction")
                .expect("压缩审计")
                .seq,
            progress[0].seq,
            l.iter()
                .find(|e| e.r#type == "compaction/summary")
                .expect("compaction/summary")
                .seq,
            progress[progress.len() - 1].seq,
        ]
    };
    assert!(
        seqs[0] < seqs[1] && seqs[1] < seqs[2] && seqs[2] < seqs[3],
        "audit < 进度 < summary < done: {seqs:?}"
    );
}

/// 未达阈值 = 压根没开始:零进度事件(不打扰桌面)
#[tokio::test]
async fn fold_skipped_emits_no_progress() {
    let log = Arc::new(Mutex::new(EventLog::new()));
    {
        let mut l = log.lock().unwrap();
        l.append(EventEnvelope::new(
            "user/message",
            0,
            json!({ "content": "hi" }),
        ))
        .unwrap();
    }
    let mut engine = LoopEngine::new(header(), Arc::clone(&log));
    engine.set_fold_thresholds(u64::MAX, u64::MAX);
    let mut provider = FakeProvider::new();
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "ok"
    }))]);
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut sink = |_ev: &EventEnvelope| {};
    engine
        .run_turn(
            "continue",
            None,
            &[],
            &[],
            &[],
            &mut gate,
            &mut NoTools,
            &|| 0_i64,
            &mut sink,
        )
        .await
        .expect("turn");
    let l = log.lock().unwrap();
    assert_eq!(
        l.iter()
            .filter(|e| e.r#type == "compaction/progress")
            .count(),
        0,
        "未触发折叠不得落进度"
    );
}

/// 自动路径摘要失败 = 静默 Skipped(不落 compaction/summary|error):
/// failed 终局是 UI 唯一的清位信号(回归锁:缺它则「正在压缩…」悬死)
#[tokio::test]
async fn summarize_failure_emits_failed_terminal() {
    let log = big_log();
    let mut engine = LoopEngine::new(header(), Arc::clone(&log));
    engine.set_fold_thresholds(1, 1);
    let mut provider = FakeProvider::new();
    provider.summary_errors.push("boom".into());
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "ok"
    }))]);
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut sink = |_ev: &EventEnvelope| {};
    engine
        .run_turn(
            "continue",
            None,
            &[],
            &[],
            &[],
            &mut gate,
            &mut NoTools,
            &|| 0_i64,
            &mut sink,
        )
        .await
        .expect("自动路径失败不上抛");

    let l = log.lock().unwrap();
    let phases: Vec<String> = l
        .iter()
        .filter(|e| e.r#type == "compaction/progress")
        .map(|e| e.data["phase"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(phases.first().map(String::as_str), Some("summarize"));
    assert_eq!(
        phases.last().map(String::as_str),
        Some("failed"),
        "失败路径必须有终局: {phases:?}"
    );
    assert!(
        !l.iter().any(|e| e.r#type == "compaction/summary"),
        "失败不落摘要(历史保持完整)"
    );
}

/// 手动路径:进度必须经 sink 实时出账(sink 即渲染广播位;曾传空闭包,
/// 最久的维护任务反而最不可见)。manual 位随载荷标注。
#[tokio::test]
async fn manual_compact_streams_progress_to_sink() {
    use liuma_agent_loop::FoldOutcome;

    let log = big_log();
    let mut engine = LoopEngine::new(header(), Arc::clone(&log));
    engine.set_fold_thresholds(0, 1);
    let mut provider = FakeProvider::new();
    provider.summaries.push("m".repeat(200));
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut seen: Vec<EventEnvelope> = Vec::new();
    {
        let mut sink = |ev: &EventEnvelope| seen.push(ev.clone());
        let outcome = engine
            .compact_now(&mut gate, &|| 0_i64, &mut sink)
            .await
            .expect("compact");
        assert!(matches!(outcome, FoldOutcome::Folded { .. }));
    }
    let progress: Vec<&EventEnvelope> = seen
        .iter()
        .filter(|e| e.r#type == "compaction/progress")
        .collect();
    assert!(
        progress.len() >= 3,
        "手动路径 sink 必须收到开始/落档/终局: {}",
        progress.len()
    );
    assert_eq!(progress[0].data["phase"], json!("summarize"));
    assert_eq!(progress[0].data["manual"], json!(true));
    assert_eq!(
        progress[progress.len() - 1].data["phase"],
        json!("done"),
        "终局 done"
    );
    // sink 与日志同源:落档的每条进度都在日志里(记录优先)
    let l = log.lock().unwrap();
    for e in &progress {
        assert!(
            l.iter()
                .any(|x| x.seq == e.seq && x.r#type == "compaction/progress"),
            "进度事件必在日志内(seq {})",
            e.seq
        );
    }
}

/// 预置含大工具输出的历史(裁定候选面:tool/result ≥ 2000 字符)
fn big_tool_log() -> Arc<Mutex<EventLog>> {
    let log = Arc::new(Mutex::new(EventLog::new()));
    {
        let mut l = log.lock().unwrap();
        for i in 0..6 {
            l.append(EventEnvelope::new(
                "user/message",
                0,
                json!({ "content": format!("question {i}: {}", "q".repeat(600)) }),
            ))
            .unwrap();
            l.append(EventEnvelope::new(
                "assistant/message",
                0,
                json!({
                    "content": "",
                    "tool_calls": [{ "id": format!("c{i}"), "name": "bash", "arguments": "{}" }],
                }),
            ))
            .unwrap();
            l.append(EventEnvelope::new(
                "tool/result",
                0,
                json!({ "id": format!("c{i}"), "output": "x".repeat(3_000) }),
            ))
            .unwrap();
        }
    }
    log
}

/// 会话里的事件(锁内克隆;测试断言用)
fn events(log: &Arc<Mutex<EventLog>>) -> Vec<EventEnvelope> {
    log.lock().unwrap().iter().cloned().collect()
}

/// 只记录且如实报数的裁定器:回执与计数进载荷,**派生面不动**
/// (advisory;「模型可见 ⟺ 已记录」不因一次咨询而变)
#[tokio::test]
async fn advisory_judge_records_counts_without_touching_the_prefix() {
    use liuma_agent_loop::value_judge::{FoldAdvice, ValueJudge};
    use liuma_decision::fake::ScriptedFoldJudge;

    let log = big_tool_log();
    let mut engine = LoopEngine::new(header(), Arc::clone(&log));
    engine.set_fold_thresholds(0, 1);
    let judge = Arc::new(ScriptedFoldJudge::returning(Ok(FoldAdvice {
        no_value: 2,
        applied: false,
        judged: 2,
        total: 3,
    })));
    engine.set_value_judge(Arc::clone(&judge) as Arc<dyn ValueJudge>);
    let mut provider = FakeProvider::new();
    provider.summaries.push("condensed".into());
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut seen: Vec<EventEnvelope> = Vec::new();
    {
        let mut sink = |ev: &EventEnvelope| seen.push(ev.clone());
        engine
            .compact_now(&mut gate, &|| 0_i64, &mut sink)
            .await
            .expect("compact");
    }
    let all = events(&log);
    // 候选窗口 = 折叠区间内的 tool/result(seq 单调、大小达标)
    let summary = all
        .iter()
        .find(|e| e.r#type == "compaction/summary")
        .expect("summary 落档");
    let through = summary.data["throughSeq"].as_u64().unwrap();
    let batches = judge.take_seen();
    assert_eq!(batches.len(), 1, "折叠区间有候选 → 咨询一次");
    let expected: Vec<u64> = all
        .iter()
        .filter(|e| e.r#type == "tool/result" && e.seq <= through)
        .map(|e| e.seq)
        .collect();
    assert!(!expected.is_empty(), "夹具须含区间内的大输出");
    assert_eq!(
        batches[0].iter().map(|c| c.seq).collect::<Vec<_>>(),
        expected,
        "候选 = 折叠区间内的全部 tool/result(大输出优先序)"
    );
    assert!(
        batches[0].windows(2).all(|w| w[0].chars >= w[1].chars),
        "大输出优先"
    );
    // 计数进载荷(诚实分母);只记录 → prunedItems 恒 0
    assert_eq!(summary.data["noValueCandidates"], json!(2));
    assert_eq!(summary.data["judgedCandidates"], json!(2));
    assert_eq!(summary.data["totalCandidates"], json!(3));
    assert_eq!(summary.data["prunedItems"], json!(0));
    // 只落 receipt:无 decision/pruned(派生面不动)
    assert!(
        !all.iter().any(|e| e.r#type == "decision/pruned"),
        "advisory 不得落 pruned"
    );
    // 相位序:judge 在 summarize 之前(真实边界)
    let phases: Vec<String> = seen
        .iter()
        .filter(|e| e.r#type == "compaction/progress")
        .filter_map(|e| e.data["phase"].as_str().map(str::to_string))
        .collect();
    assert_eq!(
        phases.first().map(String::as_str),
        Some("judge"),
        "{phases:?}"
    );
    assert!(phases.contains(&"summarize".to_string()), "{phases:?}");
    assert_eq!(phases.last().map(String::as_str), Some("done"));
}

/// 口径不变锁(advisory):裁前裁后摘要输入逐字相同——占位符不出现、
/// 正文仍在、前缀长度不变。裁定若顺手改了派生面,此处必红
#[tokio::test]
async fn advisory_keeps_the_summarize_input_verbatim() {
    use liuma_agent_loop::value_judge::{FoldAdvice, ValueJudge};
    use liuma_decision::fake::ScriptedFoldJudge;

    let log = big_tool_log();
    let mut engine = LoopEngine::new(header(), Arc::clone(&log));
    engine.set_fold_thresholds(0, 1);
    let judge = Arc::new(ScriptedFoldJudge::returning(Ok(FoldAdvice {
        no_value: 6,
        applied: false,
        judged: 6,
        total: 6,
    })));
    engine.set_value_judge(Arc::clone(&judge) as Arc<dyn ValueJudge>);
    let mut provider = FakeProvider::new();
    provider.summaries.push("condensed".into());
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    {
        let mut sink = |_ev: &EventEnvelope| {};
        engine
            .compact_now(&mut gate, &|| 0_i64, &mut sink)
            .await
            .expect("compact");
    }
    let sent = gate
        .inner()
        .summary_inputs
        .first()
        .expect("摘要调用")
        .1
        .clone();
    let text = sent.to_string();
    assert!(
        !text.contains(liuma_session::events::PRUNED_TOOL_PLACEHOLDER),
        "advisory 下摘要输入不得出现占位符"
    );
    assert!(text.contains(&"x".repeat(3_000)), "正文逐字进摘要输入");
    // 摘要输入 == **未折叠**派生面的头部逐条相同(裁前裁后同源;占位符
    // 一旦落进派生面,此处必红)。指令由 transport 追加(`summarization_
    // messages` 在 http 侧),故此处恰为 prefix_len 条
    let pre_fold = liuma_session::events::derive_visible_messages(
        events(&log)
            .iter()
            .filter(|e| e.r#type != "compaction/summary"),
    );
    let pre_arr = pre_fold.as_array().unwrap();
    assert_eq!(pre_arr.len(), 18, "6 轮 × 3 条消息(日志原文全在)");
    let sent_arr = sent.as_array().expect("数组");
    assert_eq!(sent_arr.len(), 16, "前缀 = 切点前的派生面");
    assert_eq!(
        &pre_arr[..16],
        sent_arr.as_slice(),
        "裁前裁后前缀逐条相等(口径不变锁)"
    );
    for ev in events(&log).iter().filter(|e| e.r#type == "tool/result") {
        let body = ev.data["output"].as_str().unwrap();
        assert_eq!(body.chars().count(), 3_000, "日志原文不动");
    }
}

/// fail-open 三连:端口缺席(不咨询)、候选为空(不咨询)、裁定 Err
/// (照常折叠)——三条路径折叠结果一致,`FoldOutcome` 不变
#[tokio::test]
async fn judge_failures_fail_open_into_the_same_fold() {
    use liuma_agent_loop::value_judge::ValueJudge;
    use liuma_decision::fake::ScriptedFoldJudge;

    async fn fold_with(judge: Option<Arc<dyn ValueJudge>>) -> (usize, usize, Vec<String>) {
        use liuma_agent_loop::FoldOutcome;
        let log = big_tool_log();
        let mut engine = LoopEngine::new(header(), Arc::clone(&log));
        engine.set_fold_thresholds(0, 1);
        if let Some(j) = judge {
            engine.set_value_judge(j);
        }
        let mut provider = FakeProvider::new();
        provider.summaries.push("condensed".into());
        let mut gate = InvariantGate::new(provider, Arc::clone(&log));
        let mut phases: Vec<String> = Vec::new();
        {
            let mut sink = |ev: &EventEnvelope| {
                if ev.r#type == "compaction/progress"
                    && let Some(p) = ev.data["phase"].as_str()
                {
                    phases.push(p.to_string());
                }
            };
            let outcome = engine
                .compact_now(&mut gate, &|| 0_i64, &mut sink)
                .await
                .expect("fail-open:折叠照常完成");
            assert!(matches!(outcome, FoldOutcome::Folded { .. }), "仍折叠");
        }
        let all = events(&log);
        let summaries = all
            .iter()
            .filter(|e| e.r#type == "compaction/summary")
            .count();
        let pruned = all.iter().filter(|e| e.r#type == "decision/pruned").count();
        assert_eq!(summaries, 1, "摘要照落");
        assert_eq!(pruned, 0);
        (summaries, pruned, phases)
    }

    // ①端口缺席:直通,无 judge 相位
    let (_, _, phases) = fold_with(None).await;
    assert!(!phases.contains(&"judge".to_string()), "{phases:?}");
    // ②裁定 Err:照常折叠(judge 相位在场,无 failed 终局)
    let failing = Arc::new(ScriptedFoldJudge::returning(Err("port down".into())));
    let (_, _, phases) = fold_with(Some(Arc::clone(&failing) as Arc<dyn ValueJudge>)).await;
    assert_eq!(
        phases.first().map(String::as_str),
        Some("judge"),
        "{phases:?}"
    );
    assert!(!phases.contains(&"failed".to_string()), "{phases:?}");
    assert_eq!(phases.last().map(String::as_str), Some("done"));
}

/// 候选为空(区间内无达标 tool/result)= 不咨询:省一次出网,载荷计数全零
#[tokio::test]
async fn empty_candidate_set_skips_the_judge() {
    use liuma_agent_loop::value_judge::{FoldAdvice, ValueJudge};
    use liuma_decision::fake::ScriptedFoldJudge;

    let log = big_log(); // 只有 user/assistant 消息,无工具输出
    let mut engine = LoopEngine::new(header(), Arc::clone(&log));
    engine.set_fold_thresholds(0, 1);
    let judge = Arc::new(ScriptedFoldJudge::returning(Ok(FoldAdvice {
        no_value: 9,
        applied: false,
        judged: 9,
        total: 9,
    })));
    engine.set_value_judge(Arc::clone(&judge) as Arc<dyn ValueJudge>);
    let mut provider = FakeProvider::new();
    provider.summaries.push("condensed".into());
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut phases: Vec<String> = Vec::new();
    {
        let mut sink = |ev: &EventEnvelope| {
            if ev.r#type == "compaction/progress"
                && let Some(p) = ev.data["phase"].as_str()
            {
                phases.push(p.to_string());
            }
        };
        engine
            .compact_now(&mut gate, &|| 0_i64, &mut sink)
            .await
            .expect("compact");
    }
    assert!(judge.take_seen().is_empty(), "无候选 → 不咨询");
    assert!(!phases.contains(&"judge".to_string()), "{phases:?}");
    let all = events(&log);
    let summary = all
        .iter()
        .find(|e| e.r#type == "compaction/summary")
        .expect("summary");
    assert_eq!(summary.data["totalCandidates"], json!(0));
    assert_eq!(summary.data["judgedCandidates"], json!(0));
    assert_eq!(summary.data["noValueCandidates"], json!(0));
}

/// enforce e2e:裁定生效 → 摘要输入含占位符不含正文、日志原文仍在、
/// 派生面**长度不变 + 配对不破**(策略④是 1:1 替换——口径不变锁)
#[tokio::test]
async fn enforced_fold_replaces_outputs_in_the_prefix_only() {
    use liuma_decision::types::{Answer, DecisionAnswers};
    use liuma_session::events::{DANGLING_TOOL_PLACEHOLDER, PRUNED_TOOL_PLACEHOLDER};

    let log = big_tool_log();
    // 夹具:tool/result 落在 seq 3/6/9/12/15/18;保留尾从 seq 17 起
    let droppable = [3_u64, 6, 9, 12, 15];
    let mut engine = LoopEngine::new(header(), Arc::clone(&log));
    engine.set_fold_thresholds(0, 1);
    let receipts: liuma_decision::scenarios::ReceiptSink = {
        let log = Arc::clone(&log);
        Arc::new(move |ty: &str, data: serde_json::Value| {
            // 宿主汇的最小形态:落档(生产宿主另推轨迹增量)
            let _ = log.lock().unwrap().append(EventEnvelope::new(ty, 0, data));
        })
    };
    let port = Arc::new(liuma_decision::fake::FakeDecisionPort::scripted(vec![Ok(
        DecisionAnswers {
            model: "fake".into(),
            answers: droppable
                .iter()
                .map(|s| (format!("s{s}"), Answer::Noul { noul: 0.02 }))
                .collect(),
            usage: Default::default(),
            request_id: None,
        },
    )]));
    let judge = liuma_decision::scenarios::fold::FoldJudge::new(
        Arc::clone(&port) as Arc<dyn liuma_decision::DecisionPort>,
        "m".into(),
        Arc::clone(&log),
        receipts,
        true,
    );
    engine.set_value_judge(Arc::new(judge));
    let mut provider = FakeProvider::new();
    provider.summaries.push("condensed".into());
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    {
        let mut sink = |_ev: &EventEnvelope| {};
        engine
            .compact_now(&mut gate, &|| 0_i64, &mut sink)
            .await
            .expect("compact");
    }
    let all = events(&log);
    // 裁决落档:stage=fold + 恰好那 5 条
    let pruned: Vec<EventEnvelope> = all
        .iter()
        .filter(|e| e.r#type == "decision/pruned")
        .cloned()
        .collect();
    assert_eq!(pruned.len(), 1, "一次裁定一条 pruned");
    let seqs: Vec<u64> = pruned[0].data["pruned"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            assert_eq!(p["stage"], json!("fold"), "阶段标记必须显式带上");
            p["seq"].as_u64().unwrap()
        })
        .collect();
    assert_eq!(seqs, droppable, "裁掉的是评估过的候选");
    // 摘要输入:占位符在场、正文退场
    let sent = gate
        .inner()
        .summary_inputs
        .first()
        .expect("摘要调用")
        .1
        .clone();
    let text = sent.to_string();
    assert!(text.contains(PRUNED_TOOL_PLACEHOLDER), "前缀换成占位符");
    assert!(!text.contains(&"x".repeat(3_000)), "正文不得进摘要输入");
    // 日志原文不动(审计保真)
    for seq in droppable {
        let ev = all.iter().find(|e| e.seq == seq).expect("tool/result");
        assert_eq!(ev.data["output"].as_str().unwrap().chars().count(), 3_000);
    }
    // 口径不变:前缀长度仍是 16(1:1 替换,条目不删)
    assert_eq!(sent.as_array().unwrap().len(), 16);
    // 派生面:切点之后照常,配对不破(无悬空占位补插)
    let derived = liuma_session::events::derive_visible_messages(all.iter());
    let arr = derived.as_array().unwrap();
    assert_eq!(arr.len(), 3, "占位 checkpoint + 保留尾两条");
    assert!(
        !derived.to_string().contains(DANGLING_TOOL_PLACEHOLDER),
        "配对不破"
    );
    // 载荷:已生效条数 = 裁定条数(shadow 下 prunedItems 恒 0)
    let summary = all
        .iter()
        .find(|e| e.r#type == "compaction/summary")
        .expect("summary");
    assert_eq!(summary.data["prunedItems"], json!(5));
    assert_eq!(summary.data["noValueCandidates"], json!(5));
    assert_eq!(summary.data["totalCandidates"], json!(5));
}
