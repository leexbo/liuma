//! 工具闭环 e2e:模型 tool_calls → 沙箱 bash → tool/result → 下一轮可见。
//!
//! 断言链:
//! - 事件序列含 tool/call 与 tool/result(引用链:tool/result.call = tool/call 的 seq);
//! - bash 命令真实经沙箱执行(echo 输出进入 tool/result.output);
//! - 第二轮出网请求包含 assistant(tool_calls) 与 tool 消息——全部来自日志派生;
//! - 「记录 ⟺ 可见」:出网消息恰为日志前缀派生(不变式闸门强制)。
//!
//! 平台:经 shell 工具真实执行命令的用例,载荷一律取**两套方言都合法**的
//! 形式(如 `echo`),工具名按 `shell::tool_name()` 取,故两端都跑。仍以
//! `#[cfg(unix)]` 排除的是内嵌 POSIX 语法本身的用例(管道重定向、
//! `trap`/`kill`、`test -t 1` 一类)——它们的对应用例在 Windows 侧另写,
//! 而不是把一份用例改成两副面孔。

use std::sync::{Arc, Mutex};

use liuma_agent_loop::{LlmEvent, LoopEngine, RequestHeader};
use liuma_host::JsonlBackend;
use liuma_llm::{FakeProvider, InvariantGate};
use liuma_session::{EventEnvelope, EventLog};
use liuma_tools::BashTool;
use serde_json::json;

#[tokio::test]
async fn tool_round_trip_through_sandbox() {
    let dir = std::env::temp_dir().join(format!("liuma-tool-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let jsonl = JsonlBackend::create(dir.join("tool.jsonl")).unwrap();

    let log = Arc::new(Mutex::new(EventLog::new()));
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

    // 第一步:模型请求 bash 工具;第二步:给出最终答复
    let mut provider = FakeProvider::new();
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "",
        "tool_calls": [
            json!({ "name": liuma_sandbox::shell::tool_name(), "arguments": { "command": "echo tool-ran-ok", "description": "Echo confirmation marker" } })
        ],
    }))]);
    provider.then(vec![
        LlmEvent::Chunk("done".into()),
        LlmEvent::AssistantMessage(json!({ "content": "command finished" })),
        LlmEvent::Done,
    ]);
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut bash = BashTool::new(&dir);
    let clock = || 0_i64;
    let mut sink = |ev: &EventEnvelope| {
        jsonl.append(ev).unwrap();
    };

    let outcome = engine
        .run_turn(
            "run the tool",
            None,
            &[],
            &[],
            &[],
            &mut gate,
            &mut bash,
            &clock,
            &mut sink,
        )
        .await
        .expect("turn");
    assert_eq!(outcome.assistant_message, "command finished");

    // 事件序列与引用链
    let persisted = jsonl.load().unwrap();
    let types: Vec<&str> = persisted.iter().map(|e| e.r#type.as_str()).collect();
    assert_eq!(
        types,
        [
            "turn/start",
            "step/start",
            "user/message",
            "audit/call", // llm 出网审计(E5,归因 user/message)
            "audit/call", // llm 完成审计(时长/用量)
            "assistant/message",
            "tool/call",
            "audit/call", // 工具执行审计(E5,归因 assistant/message)
            "tool/result",
            "audit/call", // 工具完成审计(时长)
            "step/end",
            "step/start",
            "audit/call",
            "assistant/chunk",
            "audit/call", // llm 完成审计
            "assistant/message",
            "step/end",
            "turn/end",
        ]
    );
    let tool_call = persisted
        .iter()
        .find(|e| e.r#type == "tool/call")
        .expect("tool/call");
    let tool_result = persisted
        .iter()
        .find(|e| e.r#type == "tool/result")
        .expect("tool/result");
    assert_eq!(
        tool_result.data["call"], tool_call.seq,
        "引用链:result.call = call.seq"
    );
    // bash 真实经沙箱执行
    assert_eq!(tool_result.data["output"], "tool-ran-ok");
    assert_eq!(tool_result.data["success"], true);
    // 终端详情经渲染意图视图携带(bash settle → Terminal 视图 → 事件 view)
    assert_eq!(tool_result.data["view"]["card"], "terminal");
    assert_eq!(tool_result.data["view"]["exitCode"], 0);
    assert!(
        tool_result.data["view"]["cwd"]
            .as_str()
            .is_some_and(|w| !w.is_empty()),
        "cwd 应在视图内"
    );

    // 第二轮出网请求:含 assistant(tool_calls) 与 tool 结果(全部来自派生)
    let (_, second) = &gate.inner().received[1];
    assert_eq!(
        second,
        &json!([
            { "role": "user", "content": "run the tool" },
            { "role": "assistant", "content": "", "tool_calls": [
                json!({ "name": liuma_sandbox::shell::tool_name(), "arguments": { "command": "echo tool-ran-ok", "description": "Echo confirmation marker" } })
            ]},
            { "role": "tool", "output": "tool-ran-ok", "call": tool_call.seq, "id": "" },
        ]),
        "第二轮请求必须包含完整工具往返(记录 ⟺ 可见)"
    );
}

#[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见文件头)
#[tokio::test]
async fn sandboxed_tool_denies_out_of_root_write() {
    // 工具经沙箱执行:可写根外写被拒 → tool/result.success = false,loop 不中断
    let dir = std::env::temp_dir().join(format!("liuma-tool-deny-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // 根外 = 可写根(workspace + 平台临时区)之外:系统根目录
    // (macOS 只读系统卷 / bwrap ro-bind /,跨平台统一)
    let outside = std::path::PathBuf::from(format!("/liuma-tool-outside-{}", std::process::id()));
    let log = Arc::new(Mutex::new(EventLog::new()));
    let mut engine = LoopEngine::new(
        RequestHeader {
            model: "t".into(),
            system: String::new(),
            temperature: 0.0,
            reasoning_effort: None,
            tools: Vec::new(),
        },
        Arc::clone(&log),
    );
    let mut provider = FakeProvider::new();
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "",
        "tool_calls": [ { "name": "bash",
            "arguments": { "command": format!("touch {}", outside.display()), "description": "Touch file outside workspace" } } ],
    }))]);
    provider.then(vec![LlmEvent::AssistantMessage(
        json!({ "content": "after" }),
    )]);
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut bash = BashTool::new(&dir);
    let clock = || 0_i64;
    let mut sink = |_ev: &EventEnvelope| {};

    engine
        .run_turn(
            "go",
            None,
            &[],
            &[],
            &[],
            &mut gate,
            &mut bash,
            &clock,
            &mut sink,
        )
        .await
        .expect("turn 不因工具失败而中断");

    let log = log.lock().unwrap();
    let result = log
        .iter()
        .find(|e| e.r#type == "tool/result")
        .expect("tool/result");
    // 2026-08 语义:非零退出是结果数据(success=true + 视图 exitCode);
    // 沙箱拒绝以「非干净成功」断言(视图 exitCode 在场且非 0)
    assert!(
        result.data["success"] == serde_json::json!(false)
            || result.data["view"]["exitCode"]
                .as_i64()
                .is_some_and(|c| c != 0),
        "根外写必须失败:success={}",
        result.data["success"]
    );
    drop(log);
    assert!(!outside.exists(), "根外文件不应存在");
}

#[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见文件头)
#[tokio::test]
async fn cancel_token_aborts_running_tool_and_turn() {
    // 出口门:取消 token 中止工具(软取消 select → kill_with_grace)
    use liuma_agent_loop::{CancelToken, LoopError};

    let dir = std::env::temp_dir().join(format!("liuma-tool-cancel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let token = CancelToken::new();
    let log = Arc::new(Mutex::new(EventLog::new()));
    let mut engine = LoopEngine::new(
        RequestHeader {
            model: "t".into(),
            system: String::new(),
            temperature: 0.0,
            reasoning_effort: None,
            tools: Vec::new(),
        },
        Arc::clone(&log),
    );
    engine.set_cancel(token.clone());

    let mut provider = FakeProvider::new();
    // 第一步请求长命令;取消后不再有下一步
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "",
        "tool_calls": [ { "name": "bash",
            "arguments": { "command": "sleep 30 && echo never", "description": "Sleep past cancel window" } } ],
    }))]);
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut bash = BashTool::new(&dir).with_cancel(token.clone());
    let mut sink = |_ev: &EventEnvelope| {};

    // 200ms 后触发软取消
    let delayed = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        delayed.cancel();
    });
    let started = std::time::Instant::now();
    let result = engine
        .run_turn(
            "go",
            None,
            &[],
            &[],
            &[],
            &mut gate,
            &mut bash,
            &|| 0_i64,
            &mut sink,
        )
        .await;
    let elapsed = started.elapsed();
    assert!(
        matches!(result, Err(LoopError::Cancelled)),
        "软取消必须中止 turn"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "取消应及时(实际 {elapsed:?})"
    );

    // 日志:turn/end 带 cancelled 归因;无 "never" 输出
    let l = log.lock().unwrap();
    let end = l.iter().find(|e| e.r#type == "turn/end").expect("turn/end");
    assert_eq!(end.data["cancelled"], "token");
    let tool_result = l
        .iter()
        .find(|e| e.r#type == "tool/result")
        .expect("tool/result");
    assert_eq!(tool_result.data["output"], "cancelled");
}

#[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见文件头)
#[tokio::test]
async fn pty_tool_runs_and_reports_tty() {
    // PTY 工具路径:test -t 1 在 PTY 下为真(沙箱经 argv 包装)
    let dir = std::env::temp_dir().join(format!("liuma-tool-pty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = Arc::new(Mutex::new(EventLog::new()));
    let mut engine = LoopEngine::new(
        RequestHeader {
            model: "t".into(),
            system: String::new(),
            temperature: 0.0,
            reasoning_effort: None,
            tools: Vec::new(),
        },
        Arc::clone(&log),
    );
    let mut provider = FakeProvider::new();
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "",
        "tool_calls": [ { "name": "bash",
            "arguments": { "command": "test -t 1 && echo is-tty", "description": "Check stdout is a TTY" } } ],
    }))]);
    provider.then(vec![LlmEvent::AssistantMessage(
        json!({ "content": "done" }),
    )]);
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut bash = BashTool::new(&dir).with_pty();
    let mut sink = |_ev: &EventEnvelope| {};

    engine
        .run_turn(
            "go",
            None,
            &[],
            &[],
            &[],
            &mut gate,
            &mut bash,
            &|| 0_i64,
            &mut sink,
        )
        .await
        .expect("turn");
    let l = log.lock().unwrap();
    let tool_result = l
        .iter()
        .find(|e| e.r#type == "tool/result")
        .expect("tool/result");
    // landlock-only 机器:spawn 拒绝(fail-closed)也是合法结果
    let output = tool_result.data["output"].as_str().unwrap_or_default();
    assert!(
        output.contains("is-tty")
            || output.contains("fail-closed")
            || output.contains("pty spawn failed"),
        "got: {output}"
    );
}

#[tokio::test]
async fn file_tools_round_trip_through_toolset() {
    // ToolSet 按名分发文件三件套;file_edit → file_read 闭环,
    // 读写往返全部经 tool/result 事件进入下一轮派生(记录 ⟺ 可见)
    use liuma_agent_loop::ToolSet;
    use liuma_tools::FileTools;

    let dir = std::env::temp_dir().join(format!("liuma-file-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let jsonl = JsonlBackend::create(dir.join("file.jsonl")).unwrap();

    let log = Arc::new(Mutex::new(EventLog::new()));
    let mut engine = LoopEngine::new(
        RequestHeader {
            model: "t".into(),
            system: String::new(),
            temperature: 0.0,
            reasoning_effort: None,
            tools: Vec::new(),
        },
        Arc::clone(&log),
    );

    // 第一步:edit 写入;第二步:read 验证;第三步:收尾
    let mut provider = FakeProvider::new();
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "",
        "tool_calls": [ { "name": "file_edit", "arguments": {
            "path": "note.md", "old_text": "draft", "new_text": "final" } } ],
    }))]);
    // edit 前置内容由夹具准备;第二步读取同一文件
    std::fs::write(dir.join("note.md"), "this is a draft\n").unwrap();
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "",
        "tool_calls": [ { "name": "file_read", "arguments": { "path": "note.md" } } ],
    }))]);
    provider.then(vec![LlmEvent::AssistantMessage(
        json!({ "content": "verified" }),
    )]);

    let mut tools = ToolSet::new(vec![Box::new(FileTools::new(&dir))]).unwrap();
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut sink = |ev: &EventEnvelope| {
        jsonl.append(ev).unwrap();
    };

    let outcome = engine
        .run_turn(
            "edit then verify",
            None,
            &[],
            &[],
            &[],
            &mut gate,
            &mut tools,
            &|| 0_i64,
            &mut sink,
        )
        .await
        .expect("turn");
    assert_eq!(outcome.assistant_message, "verified");

    // 文件真实被改写
    assert_eq!(
        std::fs::read_to_string(dir.join("note.md")).unwrap(),
        "this is a final\n"
    );

    // 两个工具结果都落日志且成功;read 输出含替换后的文本
    let persisted = jsonl.load().unwrap();
    let results: Vec<&EventEnvelope> = persisted
        .iter()
        .filter(|e| e.r#type == "tool/result")
        .collect();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|r| r.data["success"] == true));
    assert!(
        results[1].data["output"]
            .as_str()
            .expect("output")
            .contains("this is a final")
    );

    // 日志可重放:重建后派生面一致(第三轮请求含两个工具往返)
    let (_, last) = &gate.inner().received[2];
    assert!(
        serde_json::to_string(last)
            .unwrap()
            .contains("this is a final")
    );
}

#[tokio::test]
async fn todo_write_events_flow_through_engine_and_restore() {
    // todo_write 整表替换 → engine 在 tool/result 后追加 todo/write(单边界);
    // 新实例共享日志懒恢复 = 崩溃恢复语义;todo/write 不进消息面(闸门仍过)
    use liuma_agent_loop::ToolSet;
    use liuma_tools::TodoWriteTool;

    let dir = std::env::temp_dir().join(format!("liuma-todo-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let jsonl = JsonlBackend::create(dir.join("todo.jsonl")).unwrap();

    let log = Arc::new(Mutex::new(EventLog::new()));
    let mut engine = LoopEngine::new(
        RequestHeader {
            model: "t".into(),
            system: String::new(),
            temperature: 0.0,
            reasoning_effort: None,
            tools: Vec::new(),
        },
        Arc::clone(&log),
    );

    let mut provider = FakeProvider::new();
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "",
        "tool_calls": [ { "name": "todo_write", "arguments": {
            "todos": [ { "content": "write todo tests", "status": "pending" } ] } } ],
    }))]);
    provider.then(vec![LlmEvent::AssistantMessage(
        json!({ "content": "tracked" }),
    )]);

    let mut tools = ToolSet::new(vec![Box::new(TodoWriteTool::new(Arc::clone(&log)))]).unwrap();
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    let mut sink = |ev: &EventEnvelope| {
        jsonl.append(ev).unwrap();
    };

    engine
        .run_turn(
            "track my work",
            None,
            &[],
            &[],
            &[],
            &mut gate,
            &mut tools,
            &|| 0_i64,
            &mut sink,
        )
        .await
        .expect("turn");

    // 事件顺序:tool/result 紧随其后是 todo/write;type 已登记
    let persisted = jsonl.load().unwrap();
    let idx = |t: &str| {
        persisted
            .iter()
            .position(|e| e.r#type == t)
            .unwrap_or(usize::MAX)
    };
    assert!(idx("tool/call") < idx("tool/result"));
    assert!(
        idx("tool/result") < idx("todo/write"),
        "todo/write 必须在 tool/result 之后追加"
    );
    let state = persisted.iter().find(|e| e.r#type == "todo/write").unwrap();
    assert_eq!(state.data["todos"][0]["content"], "write todo tests");
    // 条目无 id({content, status})
    assert!(state.data["todos"][0].get("id").is_none());

    // 跨实例恢复:新 TodoWriteTool 共享同一日志,整表覆写即见恢复态(崩溃恢复语义)
    let mut revived = TodoWriteTool::new(Arc::clone(&log));
    use liuma_agent_loop::{ToolCallRequest, ToolPort};
    let out = ToolPort::execute(
        &mut revived,
        &ToolCallRequest {
            name: "todo_write".into(),
            arguments: json!({ "todos": [ { "content": "write todo tests", "status": "completed" } ] }),
        },
    )
    .await;
    assert_eq!(
        out.output,
        "Updated todo list: 0 pending, 0 in progress, 1 completed."
    );
    let events = ToolPort::take_state_events(&mut revived);
    assert_eq!(events.last().unwrap().1["todos"][0]["status"], "completed");

    // 消息面不含 todo/write(非 surface;闸门比对全程通过即证明)
    let l = log.lock().unwrap();
    let snap: Vec<EventEnvelope> = l.iter().collect();
    let msgs = liuma_session::derive_messages(snap.iter());
    let s = serde_json::to_string(&msgs).unwrap();
    assert!(!s.contains("todo/write"));
    assert!(s.contains("write todo tests"), "工具结果本身仍可见");
}

#[tokio::test]
async fn plan_mode_in_turn_review_flow() {
    // 计划模式 in-turn 评审状态机(turn 内阻塞:结果作为 tool/result 回传)。
    // 标准态拒绝;plan 态经评审 port 批准/拒绝/关闭三种终局文本;
    // 批准(plan/approved + 回标准态)后再提交被拒;全程事件入日志可重放
    use liuma_agent_loop::ToolSet;
    use liuma_plan::{PlanReviewDecision, PlanReviewPort, PlanTool};

    let dir = std::env::temp_dir().join(format!("liuma-plan-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let jsonl = JsonlBackend::create(dir.join("plan.jsonl")).unwrap();

    let log = Arc::new(Mutex::new(EventLog::new()));
    let mut engine = LoopEngine::new(
        RequestHeader {
            model: "t".into(),
            system: String::new(),
            temperature: 0.0,
            reasoning_effort: None,
            tools: Vec::new(),
        },
        Arc::clone(&log),
    );
    let mut sink = |ev: &EventEnvelope| {
        jsonl.append(ev).unwrap();
    };
    let clock = || 0_i64;

    /// 脚本化评审 port(终局事件落档是宿主面职责,在 liuma-core 锁;此处只回决定)
    struct ReviewPort(Option<Result<PlanReviewDecision, String>>);
    impl PlanReviewPort for ReviewPort {
        fn review(
            &self,
            _session_id: &str,
            _plan: &str,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<PlanReviewDecision, String>> + Send>>
        {
            let result = self.0.clone().unwrap_or(Err("no script".into()));
            Box::pin(async move { result })
        }
    }
    let port = |r| Some(Arc::new(ReviewPort(Some(r))) as Arc<dyn PlanReviewPort>);

    // 标准态:exit_plan_mode 被拒
    let mut provider = FakeProvider::new();
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "", "tool_calls": [
            { "name": "exit_plan_mode", "arguments": { "plan": "# p" } } ],
    }))]);
    provider.then(vec![LlmEvent::AssistantMessage(
        json!({ "content": "done0" }),
    )]);
    let mut tools = ToolSet::new(vec![Box::new(PlanTool::new(
        Arc::clone(&log),
        port(Ok(PlanReviewDecision::Approve)),
        "s",
    ))])
    .unwrap();
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    engine
        .run_turn(
            "go",
            None,
            &[],
            &[],
            &[],
            &mut gate,
            &mut tools,
            &clock,
            &mut sink,
        )
        .await
        .expect("turn");
    {
        let l = log.lock().unwrap();
        let result = l
            .iter()
            .find(|e| e.r#type == "tool/result")
            .expect("tool/result");
        assert_eq!(result.data["success"], false, "标准态不得提交计划");
    }

    // 进入 plan 态:批准终局 = 成功结果携带「carry out」指令(逐字断言)
    engine
        .commit_session_event("session/mode", json!({ "mode": "plan" }), &clock, &mut sink)
        .unwrap();
    let mut provider = FakeProvider::new();
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "", "tool_calls": [
            { "name": "exit_plan_mode", "arguments": { "plan": "# fix\n1. step" } } ],
    }))]);
    provider.then(vec![LlmEvent::AssistantMessage(
        json!({ "content": "carry out" }),
    )]);
    let mut tools = ToolSet::new(vec![Box::new(PlanTool::new(
        Arc::clone(&log),
        port(Ok(PlanReviewDecision::Approve)),
        "s",
    ))])
    .unwrap();
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    engine
        .run_turn(
            "plan it",
            None,
            &[],
            &[],
            &[],
            &mut gate,
            &mut tools,
            &clock,
            &mut sink,
        )
        .await
        .expect("turn");
    {
        let l = log.lock().unwrap();
        let result = l
            .iter()
            .rev()
            .find(|e| e.r#type == "tool/result")
            .expect("tool/result");
        assert_eq!(result.data["success"], true);
        assert!(
            result.data["output"]
                .as_str()
                .is_some_and(|t| t.contains("carry out the plan starting with your next step")),
            "批准结果即开工指令"
        );
    }

    // 拒绝终局:错误结果携带反馈(留在 plan 模式;模型修订重提)
    let mut provider = FakeProvider::new();
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "", "tool_calls": [
            { "name": "exit_plan_mode", "arguments": { "plan": "# v2" } } ],
    }))]);
    provider.then(vec![LlmEvent::AssistantMessage(
        json!({ "content": "revise" }),
    )]);
    let mut tools = ToolSet::new(vec![Box::new(PlanTool::new(
        Arc::clone(&log),
        port(Ok(PlanReviewDecision::Decline {
            feedback: Some("use OAuth".into()),
        })),
        "s",
    ))])
    .unwrap();
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    engine
        .run_turn(
            "revise",
            None,
            &[],
            &[],
            &[],
            &mut gate,
            &mut tools,
            &clock,
            &mut sink,
        )
        .await
        .expect("turn");
    {
        let l = log.lock().unwrap();
        let result = l
            .iter()
            .rev()
            .find(|e| e.r#type == "tool/result")
            .expect("tool/result");
        assert_eq!(result.data["success"], false);
        assert!(
            result.data["output"]
                .as_str()
                .is_some_and(|t| t.contains("keep planning") && t.contains("use OAuth")),
            "反馈经工具错误结果回传"
        );
        // 拒绝不切模式:仍是 plan 态
        let mode = l
            .iter()
            .rev()
            .find(|e| e.r#type == "session/mode")
            .expect("session/mode");
        assert_eq!(mode.data["mode"], "plan", "拒绝留在 plan 模式");
    }

    // 关闭评审终局:错误结果 = 「等待用户说话」(逐字断言)
    let mut provider = FakeProvider::new();
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "", "tool_calls": [
            { "name": "exit_plan_mode", "arguments": { "plan": "# v3" } } ],
    }))]);
    provider.then(vec![LlmEvent::AssistantMessage(
        json!({ "content": "wait" }),
    )]);
    let mut tools = ToolSet::new(vec![Box::new(PlanTool::new(
        Arc::clone(&log),
        port(Err(liuma_plan::DISMISSED_REVIEW_ERROR.to_string())),
        "s",
    ))])
    .unwrap();
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    engine
        .run_turn(
            "dismiss",
            None,
            &[],
            &[],
            &[],
            &mut gate,
            &mut tools,
            &clock,
            &mut sink,
        )
        .await
        .expect("turn");
    {
        let l = log.lock().unwrap();
        let result = l
            .iter()
            .rev()
            .find(|e| e.r#type == "tool/result")
            .expect("tool/result");
        assert_eq!(result.data["success"], false);
        assert!(
            result.data["output"]
                .as_str()
                .is_some_and(|t| t.contains("stay in plan mode")),
            "关闭评审 = 停在原地等待用户消息"
        );
    }

    // 批准:plan/approved + 回标准态(宿主面落档);此后再提交被拒(读日志态)
    engine
        .commit_session_event(
            "plan/approved",
            json!({ "plan": "# fix\n1. step" }),
            &clock,
            &mut sink,
        )
        .unwrap();
    engine
        .commit_session_event(
            "session/mode",
            json!({ "mode": "standard" }),
            &clock,
            &mut sink,
        )
        .unwrap();
    let mut provider = FakeProvider::new();
    provider.then(vec![LlmEvent::AssistantMessage(json!({
        "content": "", "tool_calls": [
            { "name": "exit_plan_mode", "arguments": { "plan": "# again" } } ],
    }))]);
    provider.then(vec![LlmEvent::AssistantMessage(
        json!({ "content": "done2" }),
    )]);
    let mut tools = ToolSet::new(vec![Box::new(PlanTool::new(
        Arc::clone(&log),
        port(Ok(PlanReviewDecision::Approve)),
        "s",
    ))])
    .unwrap();
    let mut gate = InvariantGate::new(provider, Arc::clone(&log));
    engine
        .run_turn(
            "implement",
            None,
            &[],
            &[],
            &[],
            &mut gate,
            &mut tools,
            &clock,
            &mut sink,
        )
        .await
        .expect("turn");
    let l = log.lock().unwrap();
    let result = l
        .iter()
        .rev()
        .find(|e| e.r#type == "tool/result")
        .expect("tool/result");
    assert_eq!(result.data["success"], false, "批准回标准态后不得再提交");
    // 重放:JSONL 重建后 mode/plan 状态可恢复(decode 全部通过即证)
    drop(l);
    let persisted = jsonl.load().unwrap();
    assert!(persisted.iter().any(|e| e.r#type == "session/mode"));
    assert!(persisted.iter().any(|e| e.r#type == "plan/approved"));
}

#[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见文件头)
#[tokio::test]
async fn subagent_runs_own_session_and_reports_back() {
    // 子代理独立日志 + 能力束窄化 + 报告回主日志。
    // 直接驱动工具面(不经主 engine;主面往返已有其余测试覆盖)
    use liuma_agent_loop::{ToolCallRequest, ToolPort};
    use liuma_tools::SubagentTool;

    let dir = std::env::temp_dir().join(format!("liuma-sub-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // 子代理传输:一次 bash echo + 一次收尾(子代理内部也有工具循环)。
    // 传输为工厂形态——每次调用产一个携全脚本的 provider(FakeProvider
    // 不 Clone;单子代理场景每次 factory 调用即一次完整脚本重放)
    let responses = vec![
        vec![LlmEvent::AssistantMessage(json!({
            "content": "", "tool_calls": [
                { "name": "bash", "arguments": { "command": "echo child-at-work", "description": "Echo child marker" } } ],
        }))],
        vec![LlmEvent::AssistantMessage(json!({
            "content": "child finished: all done"
        }))],
    ];
    let factory: liuma_tools::subagent::TransportFactory<FakeProvider> = Arc::new(move || {
        let mut p = FakeProvider::new();
        for r in &responses {
            p.then(r.clone());
        }
        Ok(p)
    });

    let mut tool = SubagentTool::new(&dir, factory, "test-model".into());
    let registry = tool.registry.clone();
    let out = ToolPort::execute(
        &mut tool,
        &ToolCallRequest {
            name: "subagent".into(),
            arguments: json!({ "task": "echo something" }),
        },
    )
    .await;
    assert!(out.success, "{}", out.output);
    assert!(out.output.contains("child finished: all done"));
    assert!(out.output.contains("session"));

    // 子会话日志真实存在且可重放
    let rec = registry.lock().unwrap()[0].clone();
    assert_eq!(rec.status, "done");
    let child_events =
        liuma_host::persistence::jsonl::load_jsonl(std::path::Path::new(&rec.session_path))
            .unwrap_or_default();
    assert!(!child_events.is_empty(), "子会话事件必须落盘");
    assert!(child_events.iter().any(|e| e.r#type == "turn/start"));
    assert!(
        child_events
            .iter()
            .any(|e| e.r#type == "tool/result" && e.data["output"] == "child-at-work"),
        "子代理的 bash 真实执行"
    );
    // 能力束窄化:子可写根 = .liuma/subagents/1(+ 平台临时区);
    // 根外(系统根目录,macOS 只读系统卷 / bwrap ro-bind /)写被拒
    // (非干净成功 = 失败或非零退出,退出详情经 Terminal 视图携带)
    let child_root = dir.join(".liuma/subagents/1");
    let escape = std::path::PathBuf::from(format!("/liuma-tool-escape-{}", std::process::id()));
    let mut bash = BashTool::new(&child_root);
    let denied = ToolPort::execute(
        &mut bash,
        &ToolCallRequest {
            name: "bash".into(),
            arguments: json!({ "command": format!("touch {}", escape.display()), "description": "Touch escaped path" }),
        },
    )
    .await;
    let denied_exit = match denied.view {
        Some(liuma_agent_loop::ToolView::Terminal { exit_code, .. }) => exit_code,
        _ => None,
    };
    assert!(
        !denied.success || denied_exit.is_some_and(|c| c != 0),
        "子可写根外必须拒绝"
    );
    assert!(!escape.exists());

    // list 工具可见注册表(list_agents 经寻址面;前台一次性子代理
    // 不列入清单——不可续话,模型永不需要选它)
    let mut msg_tool = liuma_tools::AgentMessageTool::new(
        "",
        None,
        Arc::new(RegistryAddressingForE2e { registry }),
    );
    let list = ToolPort::execute(
        &mut msg_tool,
        &ToolCallRequest {
            name: "list_agents".into(),
            arguments: json!({}),
        },
    )
    .await;
    assert!(list.output.contains("(no addressable agents)"));
}

/// e2e 用注册表路由寻址 port:与宿主阶梯第 2 步同一实现
/// (deliver_to_registry_record)+ 注册表行 roster
struct RegistryAddressingForE2e {
    registry: liuma_tools::SubagentRegistry,
}

impl liuma_tools::AddressingPort for RegistryAddressingForE2e {
    fn deliver(
        &self,
        _sender: &str,
        target: &str,
        text: &str,
        source: serde_json::Value,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<liuma_tools::DeliveryKind, String>> + Send + '_>,
    > {
        let out =
            liuma_tools::subagent::deliver_to_registry_record(&self.registry, target, text, source);
        Box::pin(std::future::ready(out))
    }

    fn roster(&self, _sender: &str) -> Vec<liuma_tools::RosterEntry> {
        Vec::new()
    }
}

#[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见文件头)
#[tokio::test]
async fn subagent_cancel_propagates_from_parent() {
    // 父令牌取消 → 子令牌 → 子引擎安全点收尾(cancelled)
    use liuma_agent_loop::{CancelToken, ToolCallRequest, ToolPort};
    use liuma_tools::SubagentTool;

    let dir = std::env::temp_dir().join(format!("liuma-sub-cancel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let parent = CancelToken::new();

    // 子代理传输:子代理跑一个长 sleep bash,父令牌中途取消(工厂形态)
    let responses = vec![
        vec![LlmEvent::AssistantMessage(json!({
            "content": "", "tool_calls": [
                { "name": "bash", "arguments": { "command": "sleep 30 && echo never", "description": "Sleep past cancel window" } } ],
        }))],
        vec![LlmEvent::AssistantMessage(
            json!({ "content": "unreached" }),
        )],
    ];
    let factory: liuma_tools::subagent::TransportFactory<FakeProvider> = Arc::new(move || {
        let mut p = FakeProvider::new();
        for r in &responses {
            p.then(r.clone());
        }
        Ok(p)
    });
    let mut tool = SubagentTool::new(&dir, factory, "m".into()).with_cancel(parent.clone());
    let delayed = parent.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        delayed.cancel();
    });
    let started = std::time::Instant::now();
    let out = ToolPort::execute(
        &mut tool,
        &ToolCallRequest {
            name: "subagent".into(),
            arguments: json!({ "task": "long task" }),
        },
    )
    .await;
    assert!(!out.success);
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert!(!out.output.contains("unreached"));
    let rec = &tool.registry.lock().unwrap()[0];
    assert_eq!(rec.status, "cancelled");
}

#[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见文件头)
#[tokio::test]
async fn background_job_runs_reads_and_stops() {
    // run_in_background 立即返回 job id;输出落盘;跨调用可读;
    // stop 杀长任务;jobs 工具全程可见
    use liuma_agent_loop::{ToolCallRequest, ToolPort};
    use liuma_tools::{JobTool, JobsRegistry};

    let dir = std::env::temp_dir().join(format!("liuma-jobs-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let registry = JobsRegistry::new();
    let mut bash = BashTool::new(&dir).with_jobs(registry.clone());
    let mut jobs = JobTool::new(registry.clone());

    fn bash_call(args: serde_json::Value) -> ToolCallRequest {
        ToolCallRequest {
            name: "bash".into(),
            arguments: args,
        }
    }

    // 后台启动:立即返回
    let out = ToolPort::execute(
        &mut bash,
        &bash_call(json!({ "command": "echo bg-output && sleep 1", "description": "Echo background output", "run_in_background": true })),
    )
    .await;
    assert!(out.success, "{}", out.output);
    assert!(out.output.contains("started job 1"));

    // 长任务(用于 stop)
    let out = ToolPort::execute(
        &mut bash,
        &bash_call(json!({ "command": "sleep 30", "description": "Sleep for stop test", "run_in_background": true })),
    )
    .await;
    assert!(out.output.contains("started job 2"));

    // list:两个 running
    let list = ToolPort::execute(
        &mut jobs,
        &ToolCallRequest {
            name: "jobs".into(),
            arguments: json!({ "action": "list" }),
        },
    )
    .await;
    assert!(list.output.contains("1. [running]"), "{}", list.output);
    assert!(list.output.contains("2. [running]"));

    // 等 job 1 完成(watcher 落盘 + 收尾)
    let mut job1_done = false;
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let done = registry
            .lock()
            .unwrap()
            .iter()
            .find(|j| j.id == 1)
            .is_some_and(|j| j.status == "done");
        if done {
            job1_done = true;
            break;
        }
    }
    assert!(job1_done, "job 1 应在 5s 内完成");

    // read:输出已落盘
    let read = ToolPort::execute(
        &mut jobs,
        &ToolCallRequest {
            name: "jobs".into(),
            arguments: json!({ "action": "read", "id": 1 }),
        },
    )
    .await;
    assert!(read.output.contains("bg-output"), "{}", read.output);
    // 输出文件即持久事实
    assert!(dir.join(".liuma/jobs/1.log").exists());

    // stop job 2:快速终止
    let started = std::time::Instant::now();
    let stop = ToolPort::execute(
        &mut jobs,
        &ToolCallRequest {
            name: "jobs".into(),
            arguments: json!({ "action": "stop", "id": 2 }),
        },
    )
    .await;
    assert!(stop.success, "{}", stop.output);
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    let status2 = registry
        .lock()
        .unwrap()
        .iter()
        .find(|j| j.id == 2)
        .unwrap()
        .status
        .clone();
    assert!(
        status2 == "stopped",
        "job 2 状态应为 stopped,实际 {status2}"
    );

    // 未启用后台的装配:run_in_background 被拒
    let mut plain = BashTool::new(&dir);
    let denied = ToolPort::execute(
        &mut plain,
        &bash_call(json!({ "command": "echo x", "run_in_background": true })),
    )
    .await;
    assert!(!denied.success);
}

/// 流式落盘:任务运行中 `jobs read` 即有内容(watcher 逐块写,
/// 不等 EOF 收尾)
#[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见文件头)
#[tokio::test]
async fn jobs_read_streams_output_while_running() {
    use liuma_agent_loop::{ToolCallRequest, ToolPort};
    use liuma_tools::{JobTool, JobsRegistry};

    let dir = std::env::temp_dir().join(format!("liuma-jobs-stream-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let registry = JobsRegistry::new();
    let mut bash = BashTool::new(&dir).with_jobs(registry.clone());
    let mut jobs = JobTool::new(registry.clone());

    let out = ToolPort::execute(
        &mut bash,
        &ToolCallRequest {
            name: "bash".into(),
            arguments: json!({
                "command": "echo part1 && sleep 2 && echo part2",
                "description": "Emit output then linger",
                "run_in_background": true
            }),
        },
    )
    .await;
    assert!(out.success, "{}", out.output);

    // 停留窗内(sleep 2)读取:已产出的 part1 应可见
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(
        registry
            .lock()
            .unwrap()
            .iter()
            .any(|j| j.status == "running"),
        "任务应仍在运行"
    );
    let read = ToolPort::execute(
        &mut jobs,
        &ToolCallRequest {
            name: "jobs".into(),
            arguments: json!({ "action": "read", "id": 1 }),
        },
    )
    .await;
    assert!(
        read.output.contains("part1"),
        "运行中 read 应含已流式落盘的输出: {}",
        read.output
    );

    // 收尾:stop 终止残留任务,状态落定 stopped
    let stop = ToolPort::execute(
        &mut jobs,
        &ToolCallRequest {
            name: "jobs".into(),
            arguments: json!({ "action": "stop", "id": 1 }),
        },
    )
    .await;
    assert!(stop.success, "{}", stop.output);
}

/// 前台超时移交:预算耗尽 → 登记 job + 移交文案立即返回,进程不杀,
/// 已读前缀随移交落盘,后续输出继续流式写入;stop 收口
#[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见文件头)
#[tokio::test]
async fn foreground_timeout_moves_command_to_background() {
    use liuma_agent_loop::{ToolCallRequest, ToolPort};
    use liuma_tools::{JobTool, JobsRegistry};

    let dir = std::env::temp_dir().join(format!("liuma-jobs-mv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let registry = JobsRegistry::new();
    let mut bash = BashTool::new(&dir).with_jobs(registry.clone());
    let mut jobs = JobTool::new(registry.clone());

    let started = std::time::Instant::now();
    let out = ToolPort::execute(
        &mut bash,
        &ToolCallRequest {
            name: "bash".into(),
            arguments: json!({
                "command": "echo before-timeout && sleep 30",
                "description": "Outlive the budget then linger",
                "timeout_ms": 300
            }),
        },
    )
    .await;
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "移交应立即返回(等了 {:?})",
        started.elapsed()
    );
    assert!(out.success, "{}", out.output);
    assert!(
        out.output
            .contains("moved to background: job 1 after 300ms"),
        "{}",
        out.output
    );
    assert!(out.output.contains(".liuma/jobs/1.log"), "{}", out.output);
    assert!(
        registry
            .lock()
            .unwrap()
            .iter()
            .any(|j| j.status == "running"),
        "移交后 job 应为 running"
    );
    // 已读前缀随移交落盘
    let mut prefix_seen = false;
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Ok(text) = std::fs::read_to_string(dir.join(".liuma/jobs/1.log"))
            && text.contains("before-timeout")
        {
            prefix_seen = true;
            break;
        }
    }
    assert!(prefix_seen, "已读前缀应随移交写入日志");

    let stop = ToolPort::execute(
        &mut jobs,
        &ToolCallRequest {
            name: "jobs".into(),
            arguments: json!({ "action": "stop", "id": 1 }),
        },
    )
    .await;
    assert!(stop.success, "{}", stop.output);
}

/// 竞态回归锁:预算与命令完成几乎同刻,两种胜负都合法——正常落定,
/// 或移交后台(移交分支的 job 秒级结算且日志含完整输出)。
/// 断言的真正不变式:预算耗尽绝不杀一个 jobs 在场的快速命令
#[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见文件头)
#[tokio::test]
async fn tiny_timeout_never_kills_a_fast_command() {
    use liuma_agent_loop::{ToolCallRequest, ToolPort};
    use liuma_tools::JobsRegistry;

    let dir = std::env::temp_dir().join(format!("liuma-jobs-race-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let registry = JobsRegistry::new();
    let mut bash = BashTool::new(&dir).with_jobs(registry.clone());

    let out = ToolPort::execute(
        &mut bash,
        &ToolCallRequest {
            name: "bash".into(),
            arguments: json!({
                "command": "echo race-ok",
                "description": "Complete around the deadline",
                "timeout_ms": 1
            }),
        },
    )
    .await;
    assert!(out.success, "两种胜负都不得失败: {}", out.output);
    if out.output.contains("moved to background") {
        // 移交分支:job 在秒级内结算 done,日志含完整输出
        let mut settled = String::new();
        for _ in 0..100 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let done = registry.lock().unwrap().iter().any(|j| j.status == "done");
            if done {
                settled =
                    std::fs::read_to_string(dir.join(".liuma/jobs/1.log")).unwrap_or_default();
                break;
            }
        }
        assert!(
            registry.lock().unwrap().iter().any(|j| j.status == "done"),
            "移交的快速命令应秒级结算"
        );
        assert!(settled.contains("race-ok"), "日志应含完整输出: {settled}");
    } else {
        assert_eq!(out.output, "race-ok");
    }
}

/// 降级路径:无 jobs 装配时预算耗尽杀进程,交出部分输出 + 如实文案
#[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见文件头)
#[tokio::test]
async fn foreground_timeout_without_jobs_degrades_to_kill() {
    use liuma_agent_loop::{ToolCallRequest, ToolPort};

    let dir = std::env::temp_dir().join(format!("liuma-jobs-deg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut bash = BashTool::new(&dir);
    let started = std::time::Instant::now();
    let out = ToolPort::execute(
        &mut bash,
        &ToolCallRequest {
            name: "bash".into(),
            arguments: json!({
                "command": "sleep 30",
                "description": "Outlive the budget with no jobs support",
                "timeout_ms": 300
            }),
        },
    )
    .await;
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "降级杀应在宽限内返回(等了 {:?})",
        started.elapsed()
    );
    assert!(!out.success);
    assert!(
        out.output.contains("timed out after 300ms"),
        "{}",
        out.output
    );
    assert!(out.output.contains("killed"), "{}", out.output);
}

/// PTY 超时移交:与 pipes 同一裁决——预算耗尽转后台 job(不杀),
/// 前缀随移交落盘,stop 走 PTY killer
#[cfg(unix)] // PTY 用例见 pty.rs 测试头:嵌套沙箱内 openpty 被拒,环境性跳过
#[tokio::test]
async fn pty_timeout_moves_command_to_background() {
    use liuma_agent_loop::{ToolCallRequest, ToolPort};
    use liuma_tools::{JobTool, JobsRegistry};

    if std::process::Command::new("/usr/bin/sandbox-exec")
        .args(["-p", "(version 1)", "true"])
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(false)
    {
        eprintln!("嵌套沙箱内 openpty 不可用:环境性跳过断言");
        return;
    }
    let dir = std::env::temp_dir().join(format!("liuma-jobs-pty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let registry = JobsRegistry::new();
    let mut bash = BashTool::new(&dir).with_jobs(registry.clone()).with_pty();
    let mut jobs = JobTool::new(registry.clone());

    let out = ToolPort::execute(
        &mut bash,
        &ToolCallRequest {
            name: "bash".into(),
            arguments: json!({
                "command": "echo pty-prefix && sleep 30",
                "description": "Outlive the pty budget then linger",
                "timeout_ms": 300
            }),
        },
    )
    .await;
    assert!(out.success, "{}", out.output);
    assert!(
        out.output
            .contains("moved to background: job 1 after 300ms"),
        "{}",
        out.output
    );
    let mut prefix_seen = false;
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Ok(text) = std::fs::read_to_string(dir.join(".liuma/jobs/1.log"))
            && text.contains("pty-prefix")
        {
            prefix_seen = true;
            break;
        }
    }
    assert!(prefix_seen, "PTY 前缀应随移交写入日志");

    let stop = ToolPort::execute(
        &mut jobs,
        &ToolCallRequest {
            name: "jobs".into(),
            arguments: json!({ "action": "stop", "id": 1 }),
        },
    )
    .await;
    assert!(stop.success, "{}", stop.output);
}

/// 后台 job 结算通知端到端:自然完成与 stop 都投 notice
/// (kind=shell-job-settled,文案含 job id/退出语义/输出尾部)
#[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见文件头)
#[tokio::test]
async fn background_job_settles_with_notice() {
    use liuma_agent_loop::{ToolCallRequest, ToolPort};
    use liuma_tools::{JobTool, JobsRegistry, SettlementNotificationPort};
    use std::future::Future;
    use std::pin::Pin;

    /// 录制型通知 port:记录 (text, source)
    #[derive(Default, Clone)]
    struct RecordingNotify {
        calls: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    }
    impl SettlementNotificationPort for RecordingNotify {
        fn notify(
            &self,
            _parent: &str,
            text: String,
            source: serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.calls.lock().unwrap().push((text, source));
            Box::pin(std::future::ready(()))
        }
    }

    let dir = std::env::temp_dir().join(format!("liuma-jobs-ntc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let registry = JobsRegistry::new();
    let recorder = RecordingNotify::default();
    let mut bash = BashTool::new(&dir)
        .with_jobs(registry.clone())
        .with_job_notify(Arc::new(recorder.clone()), "session-a");
    let mut jobs = JobTool::new(registry.clone());

    // job 1:自然完成(带输出尾部)
    let out = ToolPort::execute(
        &mut bash,
        &ToolCallRequest {
            name: "bash".into(),
            arguments: json!({
                "command": "echo notice-tail",
                "description": "Emit tail for the notice",
                "run_in_background": true
            }),
        },
    )
    .await;
    assert!(out.success, "{}", out.output);
    assert!(
        out.output.contains("you will be notified when it settles"),
        "启动文案应承诺通知: {}",
        out.output
    );
    for _ in 0..1000 {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        if !recorder.calls.lock().unwrap().is_empty() {
            break;
        }
    }
    {
        let calls = recorder.calls.lock().unwrap();
        let (text, source) = &calls[0];
        assert_eq!(source["kind"], "shell-job-settled");
        assert_eq!(source["jobId"], 1);
        assert!(text.contains("finished successfully"), "{text}");
        assert!(text.contains("notice-tail"), "通知应带输出尾部: {text}");
    }

    // job 2:stop 通知
    let out = ToolPort::execute(
        &mut bash,
        &ToolCallRequest {
            name: "bash".into(),
            arguments: json!({
                "command": "sleep 30",
                "description": "Linger for the stop notice",
                "run_in_background": true
            }),
        },
    )
    .await;
    assert!(out.success, "{}", out.output);
    let stop = ToolPort::execute(
        &mut jobs,
        &ToolCallRequest {
            name: "jobs".into(),
            arguments: json!({ "action": "stop", "id": 2 }),
        },
    )
    .await;
    assert!(stop.success, "{}", stop.output);
    for _ in 0..1000 {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        if recorder.calls.lock().unwrap().len() >= 2 {
            break;
        }
    }
    {
        let calls = recorder.calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "每个 job 恰投一次结算通知");
        let (text, source) = &calls[1];
        assert_eq!(source["kind"], "shell-job-settled");
        assert_eq!(source["jobId"], 2);
        assert!(text.contains("was stopped"), "{text}");
    }
}

/// 对等互发 e2e(计划 peer-agent-messaging §3):同一父的两个驻留子代理,
/// A 任务中途经寻址面向 B 发消息(idle 续话路径),B 开新 turn 消费并
/// 回复;A/B 各自结算,通知均回投父。寻址 port = 注册表路由(与宿主
/// 阶梯第 ② 步同一实现);权限面由 liuma-core L2 覆盖。
#[cfg(unix)] // 平台沙箱与壳就位前仅 Unix 真跑(见文件头)
#[tokio::test]
async fn sibling_agents_message_each_other() {
    use liuma_agent_loop::{ToolCallRequest, ToolPort};
    use liuma_tools::subagent::{SubagentRegistry, SubagentTool, TransportFactory};
    use liuma_tools::{AddressingPort, DeliveryKind, RosterEntry};

    /// 注册表路由寻址 port(与宿主阶梯第 ② 步同一实现)
    struct SiblingAddressing {
        registry: SubagentRegistry,
    }
    impl AddressingPort for SiblingAddressing {
        fn deliver(
            &self,
            _sender: &str,
            target: &str,
            text: &str,
            source: serde_json::Value,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<DeliveryKind, String>> + Send + '_>>
        {
            let out = liuma_tools::subagent::deliver_to_registry_record(
                &self.registry,
                target,
                text,
                source,
            );
            Box::pin(std::future::ready(out))
        }
        fn roster(&self, _sender: &str) -> Vec<RosterEntry> {
            Vec::new()
        }
    }

    let dir = std::env::temp_dir().join(format!("liuma-sibling-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // B 的会话 id 共享槽:A 的脚本在工厂调用时(驻留启动即刻)读取,
    // 此时 B 已起且 idle(测试控制启动顺序)
    let b_slot = Arc::new(Mutex::new(None::<String>));
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let factory: TransportFactory<FakeProvider> = {
        let b_slot = b_slot.clone();
        let calls = calls.clone();
        Arc::new(move || {
            use std::sync::atomic::Ordering;
            let n = calls.fetch_add(1, Ordering::SeqCst);
            let mut p = FakeProvider::new();
            match n {
                // B 组:初始 turn 回复;A 的消息到达后(续话)再回复
                0 => {
                    p.then(vec![LlmEvent::AssistantMessage(json!({
                        "content": "b started"
                    }))]);
                    p.then(vec![LlmEvent::AssistantMessage(json!({
                        "content": "b replied to sibling"
                    }))]);
                }
                // A 组:先 send_message 到 B(参数在调用时组装,读 B id 槽),
                // 再收尾
                1 => {
                    let b_id = b_slot.lock().unwrap().clone();
                    p.then(vec![LlmEvent::AssistantMessage(json!({
                        "content": "",
                        "tool_calls": [ {
                            "name": "send_message",
                            "arguments": {
                                "agent_id": b_id.unwrap_or_default(),
                                "message": "handoff from a"
                            }
                        } ]
                    }))]);
                    p.then(vec![LlmEvent::AssistantMessage(json!({
                        "content": "a done"
                    }))]);
                }
                _ => {}
            }
            Ok(p)
        })
    };
    // 录制型结算通知(断言回投父)
    #[derive(Default, Clone)]
    struct RecNotify {
        got: Arc<Mutex<Vec<(String, String, serde_json::Value)>>>,
    }
    impl liuma_tools::subagent::SettlementNotificationPort for RecNotify {
        fn notify(
            &self,
            parent: &str,
            text: String,
            source: serde_json::Value,
        ) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.got
                .lock()
                .unwrap()
                .push((parent.to_string(), text, source));
            Box::pin(std::future::ready(()))
        }
    }
    let notify = RecNotify::default();
    // 寻址 port 与工具共享同一注册表(兄弟 = 同一父注册表内的记录)
    let registry = SubagentRegistry::default();
    let mut tool = SubagentTool::new(&dir, factory, "m".into())
        .with_notify(Arc::new(notify.clone()))
        .with_parent_id("parent-e2e")
        .with_registry(registry.clone())
        .with_addressing(Arc::new(SiblingAddressing { registry }));

    // 起 B(工厂第 1 次调用),等 idle
    let out = ToolPort::execute(
        &mut tool,
        &ToolCallRequest {
            name: "subagent".into(),
            arguments: json!({ "description": "B worker", "prompt": "run b" }),
        },
    )
    .await;
    assert!(out.success, "{}", out.output);
    let b_id = out
        .output
        .strip_prefix("started subagent ")
        .unwrap()
        .to_string();
    *b_slot.lock().unwrap() = Some(b_id.clone());
    for _ in 0..1000 {
        if tool
            .registry
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.session_id == b_id && r.status == "idle")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    // 起 A(工厂第 2 次调用;脚本读 B id 槽组装 send_message 参数)
    let out = ToolPort::execute(
        &mut tool,
        &ToolCallRequest {
            name: "subagent".into(),
            arguments: json!({ "description": "A worker", "prompt": "run a" }),
        },
    )
    .await;
    assert!(out.success, "{}", out.output);
    let a_id = out
        .output
        .strip_prefix("started subagent ")
        .unwrap()
        .to_string();

    // 等:A 结算 + B 收到消息后回复再结算(共 3 次通知)
    for _ in 0..2000 {
        if notify.got.lock().unwrap().len() >= 3 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    {
        let got = notify.got.lock().unwrap();
        assert_eq!(got.len(), 3, "A 一次 + B 两次结算,均回投父:{got:#?}");
        assert!(
            got.iter().all(|(p, _, _)| p == "parent-e2e"),
            "通知父会话恒定"
        );
        let texts: Vec<&String> = got.iter().map(|(_, t, _)| t).collect();
        assert!(texts.iter().any(|t| t.contains("a done")), "{texts:?}");
        assert!(
            texts.iter().any(|t| t.contains("b replied to sibling")),
            "{texts:?}"
        );
    }

    // B 的子会话日志:来自 A 的消息 = 染色 user/message(sender = A),
    // 且 B 跑了两个 turn(初始 + 续话)
    let events = {
        let path = tool
            .registry
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.session_id == b_id)
            .map(|r| r.session_path.clone())
            .unwrap();
        liuma_host::persistence::jsonl::load_jsonl(std::path::Path::new(&path)).unwrap()
    };
    let inbound = events.iter().find(|e| {
        e.r#type == "user/message"
            && e.data["content"]
                .as_str()
                .is_some_and(|c| c.contains("handoff from a"))
    });
    let inbound = inbound.expect("B 必须收到 A 的消息(染色 user/message)");
    assert_eq!(inbound.data["source"]["kind"], "subagent-message");
    assert_eq!(inbound.data["source"]["senderSessionId"], a_id.as_str());
    assert_eq!(
        events.iter().filter(|e| e.r#type == "turn/start").count(),
        2,
        "B = 初始 turn + 续话 turn"
    );
}
