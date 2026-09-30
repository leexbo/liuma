//! 决策流集成回归:FakeProvider 驱动模型调用 decide 工具,断言——
//! - `decision/asked` + `decision/answered` 事件对经引擎唯一写入口落档
//!   (take_state_events 单边界规则;id 配对,asked 在 answered 前);
//! - 端口失败:工具软失败(`success:false`)+ 事件对仍收口(ok=false),
//!   turn 正常完成(不中断 loop);
//! - 出网请求的 tool 结果全部来自日志派生(「记录 ⟺ 可见」)。

use std::sync::{Arc, Mutex};

use liuma_agent_loop::{LlmEvent, LoopEngine, RequestHeader, ToolSet};
use liuma_decision::{DecisionError, FakeDecisionPort};
use liuma_llm::{FakeProvider, InvariantGate};
use liuma_session::EventLog;
use serde_json::json;

/// 引擎装配底座:脚本化 provider + decide 工具集 + 事件收集器
struct Harness {
    engine: LoopEngine,
    gate: InvariantGate<FakeProvider>,
    tools: ToolSet,
    events: Arc<Mutex<Vec<liuma_session::EventEnvelope>>>,
}

fn harness(port: Arc<dyn liuma_decision::DecisionPort>) -> Harness {
    let log = Arc::new(Mutex::new(EventLog::new()));
    let mut provider = FakeProvider::new();
    // 第一步:模型调用 decide;第二步:最终答复
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "",
        "tool_calls": [
            json!({ "name": "decide", "arguments": {
                "context": "connection reset by peer after 30s",
                "questions": [
                    { "id": "is_transient", "kind": "noul", "instructions": "Is this error transient?" }
                ]
            } })
        ],
    }))]);
    provider.then(vec![
        LlmEvent::Chunk("done".into()),
        LlmEvent::AssistantMessage(json!({ "content": "it was transient" })),
        LlmEvent::Done,
    ]);
    let gate = InvariantGate::new(provider, Arc::clone(&log));
    let tools = ToolSet::new(vec![Box::new(liuma_decision::DecideTool::new(
        port,
        "jev-latest".into(),
    ))])
    .expect("工具集装配");
    let engine = LoopEngine::new(
        RequestHeader {
            model: "test".into(),
            system: String::new(),
            temperature: 0.0,
            reasoning_effort: None,
            tools: Vec::new(),
        },
        log,
    );
    Harness {
        engine,
        gate,
        tools,
        events: Arc::new(Mutex::new(Vec::new())),
    }
}

async fn run(h: &mut Harness, input: &str) {
    let clock = || 0_i64;
    let sink_events = Arc::clone(&h.events);
    let mut sink = move |ev: &liuma_session::EventEnvelope| {
        sink_events.lock().unwrap().push(ev.clone());
    };
    h.engine
        .run_turn(
            input,
            None,
            &[],
            &[],
            &[],
            &mut h.gate,
            &mut h.tools,
            &clock,
            &mut sink,
        )
        .await
        .expect("turn 完成");
}

#[tokio::test]
async fn decide_round_trip_persists_paired_receipts() {
    let port = Arc::new(FakeDecisionPort::scripted(vec![Ok(
        FakeDecisionPort::noul_answers("is_transient", 0.93),
    )]));
    let mut h = harness(port);
    run(&mut h, "diagnose the error").await;

    // 事件序列:decision 对恰一次,asked 在 answered 前,scenario/id 配对
    let events = h.events.lock().unwrap();
    let positions: Vec<(usize, &str)> = events
        .iter()
        .enumerate()
        .filter_map(|(i, ev)| {
            let ty = ev.r#type.as_str();
            ty.starts_with("decision/").then_some((i, ty))
        })
        .collect();
    assert_eq!(
        positions,
        vec![
            (positions[0].0, "decision/asked"),
            (positions[1].0, "decision/answered")
        ],
        "恰一对且有序:{positions:?}"
    );
    let asked = &events[positions[0].0].data;
    let answered = &events[positions[1].0].data;
    assert_eq!(asked["scenario"], "tool");
    assert_eq!(asked["model"], "jev-latest");
    assert_eq!(asked["questions"][0]["kind"], "noul");
    assert_eq!(asked["id"], answered["id"], "审计对以 id 配对");
    assert_eq!(answered["ok"], true);
    let noul = answered["answers"]["is_transient"]["noul"]
        .as_f64()
        .unwrap();
    assert!((noul - 0.93).abs() < 1e-9, "noul={noul}");
    // 隐私面:asked 只带摘要,不带 state 原文
    assert!(asked["stateDigest"].is_string());
    assert!(asked.get("state").is_none(), "state 本体不落档");

    // receipt 在 tool/result 之后(单边界规则:引擎取走时序)
    let tool_result_pos = events
        .iter()
        .position(|ev| ev.r#type == "tool/result")
        .expect("tool/result 在场");
    assert!(
        tool_result_pos < positions[0].0,
        "decision 对在 tool/result 后落档"
    );
}

#[tokio::test]
async fn failing_port_soft_fails_and_closes_pair() {
    let port = Arc::new(FakeDecisionPort::failing(DecisionError::Timeout));
    let mut h = harness(port);
    run(&mut h, "diagnose the error").await;

    let events = h.events.lock().unwrap();
    let answered = events
        .iter()
        .find(|ev| ev.r#type == "decision/answered")
        .expect("失败也收口");
    assert_eq!(answered.data["ok"], false);
    assert_eq!(answered.data["error"], "decision timeout");
    // turn 正常闭合(turn/end 在场;软失败不中断 loop)
    assert!(events.iter().any(|ev| ev.r#type == "turn/end"));
    // 工具结果为失败文本回灌(success=false 语义)
    let tool_result = events
        .iter()
        .find(|ev| ev.r#type == "tool/result")
        .expect("tool/result 在场");
    assert!(
        tool_result.data["output"]
            .as_str()
            .unwrap()
            .contains("decide unavailable"),
        "{}",
        tool_result.data["output"]
    );
}

/// Stop 哨兵经引擎 on_stop 生效:低证据概率 → Continue 强制续跑一次
/// (第二轮请求含哨兵 reason),自限与引擎协同后正常收尾。
#[tokio::test]
async fn stop_sentinel_continues_once_then_settles() {
    let log = Arc::new(Mutex::new(EventLog::new()));
    let mut provider = FakeProvider::new();
    // 第一步:模型声称完成(无证据);第二步:补证据后再次收尾
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "all done and fully verified",
    }))]);
    provider.then(vec![
        LlmEvent::Chunk("evidence".into()),
        LlmEvent::AssistantMessage(json!({
            "content": "the build output above printed ok (exit 0)"
        })),
        LlmEvent::Done,
    ]);
    let gate = InvariantGate::new(provider, Arc::clone(&log));
    // 哨兵:第一次问 → 缺证据 0.95(Continue);第二次问 → 脚本耗尽走
    // fallback Timeout → fail-open Pass,turn 收尾
    let decision_port = Arc::new(FakeDecisionPort::scripted(vec![Ok(
        FakeDecisionPort::noul_answers("lacks_evidence", 0.95),
    )]));
    let sentinel_port = Arc::clone(&decision_port);
    let sink_seen: Arc<Mutex<Vec<(String, serde_json::Value)>>> = Arc::default();
    let sink_clone = Arc::clone(&sink_seen);
    let sink: liuma_decision::scenarios::ReceiptSink = Arc::new(move |ty, data| {
        sink_clone.lock().unwrap().push((ty.to_string(), data));
    });
    let sentinel = liuma_decision::scenarios::stop::StopSentinel::new(
        sentinel_port,
        "m".into(),
        Arc::clone(&log),
        sink,
    );

    let mut engine = LoopEngine::new(
        RequestHeader {
            model: "test".into(),
            system: String::new(),
            temperature: 0.0,
            reasoning_effort: None,
            tools: Vec::new(),
        },
        Arc::clone(&log),
    );
    engine.set_hook_port(Arc::new(sentinel));
    // Continue 经 steer 通道续跑(宿主装配面;裸引擎须显式给缓冲)
    engine.set_steer_buf(Arc::new(Mutex::new(std::collections::VecDeque::new())));
    let clock = || 0_i64;
    let events_sink: Arc<Mutex<Vec<liuma_session::EventEnvelope>>> = Arc::default();
    let events_clone = Arc::clone(&events_sink);
    let mut sink = move |ev: &liuma_session::EventEnvelope| {
        events_clone.lock().unwrap().push(ev.clone());
    };
    let mut gate_ref = gate;
    let mut no_tools = liuma_agent_loop::NoTools;
    engine
        .run_turn(
            "finish the task",
            None,
            &[],
            &[],
            &[],
            &mut gate_ref,
            &mut no_tools,
            &clock,
            &mut sink,
        )
        .await
        .expect("turn 完成");

    // turn 收尾,且经历了哨兵 Continue(两次 LLM 请求)
    let events = events_sink.lock().unwrap();
    assert!(events.iter().any(|ev| ev.r#type == "turn/end"));
    assert_eq!(
        events.iter().filter(|ev| ev.r#type == "step/start").count(),
        2,
        "Continue 强制了第二个 step"
    );
    // 哨兵 receipt:第一对 ok=true(Continue 依据),第二对 fail-open 收口
    let receipts = sink_seen.lock().unwrap();
    assert_eq!(receipts.len(), 4, "两对 asked/answered:{receipts:?}");
    assert_eq!(receipts[0].1["scenario"], "stop");
    assert_eq!(receipts[1].1["ok"], true);
    assert_eq!(receipts[3].1["ok"], false, "第二次介入 fail-open 放行");
}

/// 上下文裁判端到端:压力达标 → 批量裁决 → pruned 落档 → 派生层占位。
/// 同时验证幂等(已修剪 seq 不再参选)与未启用的零出网。
/// 窗口 10000:压力阈 6000 tok(用量 15000 ✓);保留尾预算 6400 chars
/// (seq4 5000 + seq3 5000 超出 → seq2 可裁)。
#[tokio::test]
async fn context_judge_prunes_and_derivation_replaces_output() {
    use liuma_app::DecisionSettings;

    let log = Arc::new(Mutex::new(EventLog::new()));
    {
        let mut l = log.lock().unwrap();
        let append = |l: &mut EventLog, ty: &str, data: serde_json::Value| {
            let mut ev = liuma_session::EventEnvelope::new(ty, 0, data);
            ev.seq = l.high_water() + 1;
            l.append(ev).unwrap();
        };
        append(
            &mut l,
            "user/message",
            json!({ "content": "build the thing", "source": { "kind": "user" } }),
        ); // seq1
        append(
            &mut l,
            "tool/result",
            json!({ "call": 1, "output": "old ".repeat(1250) }),
        ); // seq2 ≈5000 chars(可裁)
        append(
            &mut l,
            "tool/result",
            json!({ "call": 2, "output": "mid ".repeat(1250) }),
        ); // seq3 ≈5000 chars(保留尾界)
        append(
            &mut l,
            "tool/result",
            json!({ "call": 3, "output": "fresh ".repeat(1000) }),
        ); // seq4 ≈6000 chars(保留尾内)
        append(&mut l, "assistant/message", json!({ "content": "done" })); // seq5
        // 真实用量记录(measure_tokens 优先读它):15000 tok > 0.6×10000
        append(
            &mut l,
            "audit/call",
            json!({
                "boundary": "llm", "operation": "request-done",
                "detail": { "usage": { "input_tokens": 15000 } }
            }),
        ); // seq6
    }
    let settings = DecisionSettings {
        enabled: true,
        context: liuma_app::DecisionScenario {
            enabled: true,
            enforce: true,
            ..Default::default()
        },
        ..Default::default()
    };

    // 未启用:零出网
    {
        let disabled = DecisionSettings::default();
        let port = Arc::new(FakeDecisionPort::new());
        let out = liuma_app::judge_context(
            &log,
            &(port.clone() as Arc<dyn liuma_decision::DecisionPort>),
            &disabled,
            10_000,
            &liuma_app::log_only_receipt_sink(&log),
        )
        .await
        .unwrap();
        assert_eq!(out, None);
        assert!(port.take_received().is_empty(), "未启用零调用");
    }

    // 启用:压力达标,批量裁决(s2=无价值 0.95;s3=仍有价值 0.05)
    let port = Arc::new(FakeDecisionPort::scripted(vec![Ok({
        let mut m = std::collections::BTreeMap::new();
        m.insert(
            "s2".to_string(),
            liuma_decision::Answer::Noul { noul: 0.95 },
        );
        m.insert(
            "s3".to_string(),
            liuma_decision::Answer::Noul { noul: 0.05 },
        );
        liuma_decision::DecisionAnswers {
            model: "fake".into(),
            answers: m,
            usage: Default::default(),
            request_id: None,
        }
    })]));
    let out = liuma_app::judge_context(
        &log,
        &(port.clone() as Arc<dyn liuma_decision::DecisionPort>),
        &settings,
        10_000,
        &liuma_app::log_only_receipt_sink(&log),
    )
    .await
    .unwrap();
    assert_eq!(out, Some(1), "恰一条修剪落档");

    // pruned 落档 → 派生层策略④:seq2 输出替换占位符,seq3/seq4 原样
    let events: Vec<_> = log.lock().unwrap().iter().collect();
    assert!(events.iter().any(|e| e.r#type == "decision/pruned"));
    let derived = liuma_session::derive_visible_messages(events.iter());
    let tools: Vec<&serde_json::Value> = derived
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "tool")
        .collect();
    assert_eq!(tools.len(), 3, "消息本体保留(配对不破)");
    assert_eq!(
        tools[0]["output"],
        liuma_session::PRUNED_TOOL_PLACEHOLDER,
        "被修剪 → 占位符"
    );
    assert!(
        tools[1]["output"].as_str().unwrap().starts_with("mid"),
        "未修剪 → 原样"
    );

    // 幂等:新压力源后再次裁决,s2 已修剪不参选(s3/s4 在保留尾外可问)
    {
        let mut l = log.lock().unwrap();
        let append = |l: &mut EventLog, ty: &str, data: serde_json::Value| {
            let mut ev = liuma_session::EventEnvelope::new(ty, 0, data);
            ev.seq = l.high_water() + 1;
            l.append(ev).unwrap();
        };
        append(
            &mut l,
            "tool/result",
            json!({ "call": 4, "output": "new ".repeat(1600) }),
        ); // seq10(尾界)
        append(
            &mut l,
            "audit/call",
            json!({
                "boundary": "llm", "operation": "request-done",
                "detail": { "usage": { "input_tokens": 16000 } }
            }),
        ); // seq11
    }
    let port2 = Arc::new(FakeDecisionPort::scripted(vec![Ok({
        let mut m = std::collections::BTreeMap::new();
        m.insert(
            "s3".to_string(),
            liuma_decision::Answer::Noul { noul: 0.05 },
        );
        m.insert(
            "s4".to_string(),
            liuma_decision::Answer::Noul { noul: 0.05 },
        );
        liuma_decision::DecisionAnswers {
            model: "fake".into(),
            answers: m,
            usage: Default::default(),
            request_id: None,
        }
    })]));
    let out = liuma_app::judge_context(
        &log,
        &(port2.clone() as Arc<dyn liuma_decision::DecisionPort>),
        &settings,
        10_000,
        &liuma_app::log_only_receipt_sink(&log),
    )
    .await
    .unwrap();
    assert_eq!(out, Some(0), "s3/s4 仍有价值 → 零修剪");
    let received = port2.take_received();
    assert_eq!(received.len(), 1);
    let ids: Vec<&str> = received[0]
        .questions
        .iter()
        .map(|(id, _)| id.as_str())
        .collect();
    assert!(!ids.contains(&"s2"), "已修剪 seq 不再参选:{ids:?}");
    assert!(
        ids.contains(&"s3") && ids.contains(&"s4"),
        "保留尾外可问:{ids:?}"
    );
}

/// 上下文裁判 fail-open:端口恒错 → None、无 pruned、receipt 失败收口
#[tokio::test]
async fn context_judge_fails_open() {
    use liuma_app::DecisionSettings;

    let log = Arc::new(Mutex::new(EventLog::new()));
    {
        let mut l = log.lock().unwrap();
        let append = |l: &mut EventLog, ty: &str, data: serde_json::Value| {
            let mut ev = liuma_session::EventEnvelope::new(ty, 0, data);
            ev.seq = l.high_water() + 1;
            l.append(ev).unwrap();
        };
        append(
            &mut l,
            "user/message",
            json!({ "content": "task", "source": { "kind": "user" } }),
        ); // seq1
        append(
            &mut l,
            "tool/result",
            json!({ "call": 1, "output": "x".repeat(5000) }),
        ); // seq2(可裁)
        append(
            &mut l,
            "tool/result",
            json!({ "call": 2, "output": "y".repeat(5000) }),
        ); // seq3(尾界)
        append(
            &mut l,
            "tool/result",
            json!({ "call": 3, "output": "z".repeat(5000) }),
        ); // seq4(尾内)
        append(
            &mut l,
            "audit/call",
            json!({
                "boundary": "llm", "operation": "request-done",
                "detail": { "usage": { "input_tokens": 15000 } }
            }),
        ); // seq5
    }
    let settings = DecisionSettings {
        enabled: true,
        context: liuma_app::DecisionScenario {
            enabled: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let port = Arc::new(FakeDecisionPort::failing(DecisionError::Timeout));
    let out = liuma_app::judge_context(
        &log,
        &(port.clone() as Arc<dyn liuma_decision::DecisionPort>),
        &settings,
        10_000,
        &liuma_app::log_only_receipt_sink(&log),
    )
    .await
    .unwrap();
    assert_eq!(out, None, "fail-open");
    let events: Vec<_> = log.lock().unwrap().iter().collect();
    assert!(!events.iter().any(|e| e.r#type == "decision/pruned"));
    let answered = events
        .iter()
        .find(|e| e.r#type == "decision/answered")
        .expect("失败也收口");
    assert_eq!(answered.data["ok"], false);
}
