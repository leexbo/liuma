//! Provider 方言契约测试(rig 引擎):anthropic / responses / chat 三家族。
//!
//! 断言面(每方言,经真实 HTTP 出网捕获):
//! - wire:内部消息方言 → 出网 body 形状(system 位置、tools 形状、
//!   工具调用往返的 assistant/tool 翻译、思考强度注入);
//! - 事件:真实 SSE 事件序列 → `LlmEvent`(载荷级;usage 折叠为终结
//!   单帧——rig 家族约定);
//! - HTTP:endpoint 拼接与鉴权头(anthropic x-api-key / openai Bearer)。
//!
//! 夹具为官方完整形态(rig 解码严格按官方 schema,必填字段缺即拒收
//! ——spike 期间三度验证,属 rig 防偏差资产)。

use std::sync::{Arc, Mutex};

use liuma_agent_loop::{LlmEvent, LlmTransport, RequestHeader};
use liuma_llm::{HttpTransport, ProviderConfig};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn header(system: &str, tools: Vec<Value>) -> RequestHeader {
    RequestHeader {
        model: "test-model".into(),
        system: system.into(),
        temperature: 0.0,
        reasoning_effort: None,
        tools,
    }
}

fn bash_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "bash",
            "description": "run a command",
            "parameters": { "type": "object", "required": ["command"] },
        },
    })
}

fn tool_roundtrip_messages() -> Value {
    json!([
        { "role": "user", "content": "run it" },
        { "role": "assistant", "content": "", "tool_calls": [
            { "id": "toolu_1", "name": "bash", "arguments": "{\"command\":\"echo hi\"}" },
        ]},
        { "role": "tool", "output": "hi", "call": 6, "id": "toolu_1" },
    ])
}

/// anthropic 面最小合法 SSE 响应(单文本块一轮;wire 锁测试共用)
const ANTHROPIC_MIN_SSE: &str = concat!(
    "event: message_start\n",
    "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"test-model\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n\n",
    "event: content_block_start\n",
    "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
    "event: content_block_delta\n",
    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n",
    "event: content_block_stop\n",
    "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
    "event: message_delta\n",
    "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
    "event: message_stop\n",
    "data: {\"type\":\"message_stop\"}\n\n",
);

/// mock SSE 服务器:捕获原始请求,回固定 SSE 体
async fn spawn_sse_server(response_body: &'static str) -> (String, Arc<Mutex<Vec<u8>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let cap = Arc::clone(&captured);
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        // 完整捕获:读到头结束且 body 达 Content-Length(并行测试下
        // 请求可能分多段到达,单次 read 会截断捕获)
        let mut buf = Vec::new();
        let mut chunk = [0u8; 16384];
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let want_body = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|h| {
                let head = String::from_utf8_lossy(&buf[..h]).to_ascii_lowercase();
                let cl = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                h + 4 + cl
            });
            if let Some(want) = want_body
                && buf.len() >= want
            {
                break;
            }
            let n = tokio::time::timeout_at(deadline, socket.read(&mut chunk))
                .await
                .map(|r| r.unwrap_or(0))
                .unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        cap.lock().unwrap().extend_from_slice(&buf);
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{response_body}"
        );
        socket.write_all(reply.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
    });
    (format!("http://{addr}"), captured)
}

fn sent_body(captured: &Arc<Mutex<Vec<u8>>>) -> Value {
    let raw = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
    let start = raw.find("\r\n\r\n").expect("body 分隔") + 4;
    serde_json::from_str(&raw[start..]).expect("出网 body 应为 JSON")
}

fn sent_raw(captured: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8(captured.lock().unwrap().clone()).unwrap()
}

fn transport(base_url: String, key: &str, dialect: &str) -> HttpTransport {
    HttpTransport::with_adapter(
        ProviderConfig {
            base_url,
            api_key: key.into(),
        },
        liuma_llm::adapter_by_name(dialect).unwrap(),
    )
    .unwrap()
}

// ============================================================
// 注册表与错误体归类
// ============================================================

#[test]
fn adapter_by_name_resolves_and_rejects() {
    for name in [
        "deepseek-responses",
        "openai-responses",
        "glm-responses",
        "deepseek-chat",
        "openai-completions",
        "anthropic-messages",
    ] {
        assert!(liuma_llm::adapter_by_name(name).is_some(), "{name}");
    }
    // 未知方言拒绝(fail-fast,不静默回退)
    assert!(liuma_llm::adapter_by_name("nope").is_none());
    assert!(
        liuma_llm::adapter_by_name("deepseek-anthropic").is_none(),
        "不轻扩张:原生方言 + base_url 即接入"
    );
}

/// GLM(bigmodel)错误面 wire 锁(2026-09-12 实测取证):坏 key 时端点
/// 以 200 + application/json 顶层错误体应答,401 → AUTH(不可重试)
#[test]
fn glm_body_error_top_level_shape_is_auth() {
    use liuma_agent_loop::TransportError;
    use liuma_llm::adapters::ProviderAdapter as _;
    let glm = liuma_llm::adapters::GlmResponsesAdapter::default();
    let err = glm
        .body_error(r#"{"code":401,"msg":"令牌已过期或验证不正确","success":false}"#)
        .expect("顶层错误体应归类");
    match err {
        TransportError::Auth { status, .. } => assert_eq!(status, 401),
        other => panic!("应归类 AUTH,实得 {other:?}"),
    }
    let err = glm
        .body_error(r#"{"code":1002,"msg":"鉴权失败","success":false}"#)
        .expect("平台码应归类");
    assert!(matches!(err, TransportError::Other(_)));
    assert!(!err.retryable());
}

/// GLM 官方错误契约锁(2026-10 官方错误码表):
/// - 官方形态 = HTTP 状态 + 嵌套 {"error":{code,message}}(默认钩子路径);
///   实测形态(200 + 顶层 {code,msg})仍识别,两形态并收
/// - 业务码精化:1261「Prompt 超长」→ CONTEXT_OVERFLOW(强制压缩路径);
///   1113/1309/1311/1314/1315(欠费/到期/无权限)429 皮不可重试芯;
///   1308 等真实配额窗口保持限流可重试
#[test]
fn glm_official_error_contract() {
    use liuma_agent_loop::TransportError;
    use liuma_llm::adapters::ProviderAdapter as _;
    let glm = liuma_llm::adapters::GlmResponsesAdapter::default();
    // 官方嵌套形态(200 兜底路径;401 由 HTTP 状态承载,兜底只见
    // 平台码 1001 → Other 带消息,不可重试)
    let err = glm
        .body_error(r#"{"error":{"code":"1001","message":"Header 中未收到 Authentication 参数"}}"#)
        .expect("官方嵌套形态应归类");
    assert!(matches!(err, TransportError::Other(_)), "{err:?}");
    assert!(!err.retryable());
    // 实测顶层形态保留
    assert!(matches!(
        glm.body_error(r#"{"code":401,"msg":"令牌已过期或验证不正确","success":false}"#),
        Some(TransportError::Auth { .. })
    ));
    // 1261 → 溢出(不改判则落 INVALID_REQUEST,压缩重试路径失活)
    let refined = glm.refine_error(
        TransportError::InvalidRequest {
            status: 400,
            body: String::new(),
        },
        r#"{"error":{"code":"1261","message":"Prompt 超长"}}"#,
    );
    assert!(refined.is_context_overflow(), "{refined:?}");
    assert_eq!(refined.code(), "CONTEXT_OVERFLOW");
    // 套餐到期(1309):429 皮 → 不可重试
    let refined = glm.refine_error(
        TransportError::RateLimit {
            retry_after_ms: None,
            body: String::new(),
        },
        r#"{"error":{"code":"1309","message":"您的 GLM Coding Plan 套餐已到期"}}"#,
    );
    assert!(!refined.retryable(), "套餐到期不可重试: {refined:?}");
    // 真实配额窗口(1308):保持限流可重试
    let kept = glm.refine_error(
        TransportError::RateLimit {
            retry_after_ms: None,
            body: String::new(),
        },
        r#"{"error":{"code":"1308","message":"已达到使用上限"}}"#,
    );
    assert!(kept.retryable(), "配额窗口应保持可重试: {kept:?}");
}

/// OpenAI 兼容系默认错误面:嵌套 error.code;非错误 JSON → None
#[test]
fn openai_default_body_error_nested_shape() {
    use liuma_agent_loop::TransportError;
    use liuma_llm::adapters::ProviderAdapter as _;
    let openai = liuma_llm::adapters::OpenAiResponsesAdapter::default();
    let err = openai
        .body_error(r#"{"error":{"code":"401","message":"bad key"}}"#)
        .expect("嵌套错误体应归类");
    assert!(matches!(err, TransportError::Auth { status: 401, .. }));
    assert_eq!(openai.body_error(r#"{"ok":true}"#), None);
}

// ============================================================
// Anthropic Messages
// ============================================================

/// anthropic 全量 wire 锁:system 顶层、max_tokens、扁平 tools、
/// assistant → tool_use 块、tool → tool_result 块、鉴权头与端点
#[tokio::test]
async fn anthropic_request_shape_and_auth() {
    let (base_url, captured) = spawn_sse_server(ANTHROPIC_MIN_SSE).await;
    let mut t = transport(base_url, "sk-ant-test", "anthropic-messages");
    let events = t
        .stream(
            &header("be brief", vec![bash_tool()]),
            &tool_roundtrip_messages(),
        )
        .await
        .expect("stream");

    // 端点 /v1/messages;鉴权头 x-api-key + anthropic-version(rig 方言)
    let raw = sent_raw(&captured);
    let request_line = raw.lines().next().unwrap();
    assert!(request_line.contains("/v1/messages"), "got: {request_line}");
    assert!(raw.contains("x-api-key: sk-ant-test"), "{raw}");
    assert!(raw.contains("anthropic-version"), "{raw}");

    // wire:system 顶层;max_tokens 必带;tools 扁平 input_schema;
    // assistant → tool_use;tool → user 角色的 tool_result
    let body = sent_body(&captured);
    assert!(
        body["system"].is_array() || body["system"].is_string(),
        "{body}"
    );
    assert!(body["max_tokens"].as_u64() == Some(4096), "{body}");
    assert_eq!(body["tools"][0]["name"], "bash");
    assert!(body["tools"][0].get("input_schema").is_some(), "{body}");
    let msgs = body["messages"].as_array().unwrap();
    let tool_use = msgs
        .iter()
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .find(|b| b["type"] == "tool_use")
        .expect("assistant 应译为 tool_use 块");
    assert_eq!(tool_use["name"], "bash");
    assert_eq!(
        tool_use["input"],
        json!({"command": "echo hi"}),
        "{tool_use}"
    );
    let tool_result = msgs
        .iter()
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .find(|b| b["type"] == "tool_result")
        .expect("tool 消息应译为 tool_result 块");
    assert_eq!(tool_result["tool_use_id"], "toolu_1", "{tool_result}");

    // 事件:Chunk 直播;usage 折叠为终结单帧(input+output 互补并一);
    // 定稿 AssistantMessage + Done + ttft 尾帧
    assert_eq!(
        &events[..events.len() - 1],
        &[
            LlmEvent::Chunk("hi".into()),
            LlmEvent::Usage(json!({"input_tokens": 10, "output_tokens": 2})),
            LlmEvent::AssistantMessage(json!({
                "content": "hi", "tool_calls": []
            })),
            LlmEvent::Done
        ][..],
        "{events:?}"
    );
    assert!(ttft_within_bounds(&events), "ttft 尾帧缺失: {events:?}");
}

/// 并行工具调用 wire 锁:assistant 同条多 tool_use 时,连续内部 tool
/// 消息必须合并为**紧随的一条** user 消息(全部 tool_result 块同条)。
/// Anthropic 系校验「每个 tool_use 的 tool_result 须在下一消息」——
/// 拆多条 user 必 400(实机 deepseek /anthropic 端点复现)
#[tokio::test]
async fn anthropic_parallel_tool_results_share_one_user_message() {
    let (base_url, captured) = spawn_sse_server(ANTHROPIC_MIN_SSE).await;
    let mut t = transport(base_url, "sk-ant-test", "anthropic-messages");
    let messages = json!([
        { "role": "user", "content": "check the repo" },
        { "role": "assistant", "content": "", "tool_calls": [
            { "id": "toolu_1", "name": "bash", "arguments": "{\"command\":\"git status\"}" },
            { "id": "toolu_2", "name": "bash", "arguments": "{\"command\":\"cat Cargo.toml\"}" },
        ]},
        { "role": "tool", "output": "clean", "call": 6, "id": "toolu_1" },
        { "role": "tool", "output": "workspace", "call": 7, "id": "toolu_2" },
    ]);
    t.stream(&header("be brief", vec![bash_tool()]), &messages)
        .await
        .expect("stream");

    let body = sent_body(&captured);
    let msgs = body["messages"].as_array().unwrap();
    // assistant 条目:两个 tool_use 块
    let ai = msgs
        .iter()
        .position(|m| {
            m["role"] == "assistant"
                && m["content"]
                    .as_array()
                    .is_some_and(|c| c.iter().filter(|b| b["type"] == "tool_use").count() == 2)
        })
        .expect("assistant 应含两个 tool_use 块");
    // 紧随其后的一条 user 消息同时含两个 tool_result(顺序保持)
    let next = &msgs[ai + 1];
    assert_eq!(
        next["role"], "user",
        "tool_result 须紧随 assistant: {msgs:?}"
    );
    let results: Vec<&Value> = next["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["type"] == "tool_result")
        .collect();
    let ids: Vec<&str> = results
        .iter()
        .filter_map(|b| b["tool_use_id"].as_str())
        .collect();
    assert_eq!(ids, vec!["toolu_1", "toolu_2"], "{next}");
    // 全消息流不再有第二条携带 tool_result 的消息(无拆分)
    let result_messages = msgs
        .iter()
        .filter(|m| {
            m["content"]
                .as_array()
                .is_some_and(|c| c.iter().any(|b| b["type"] == "tool_result"))
        })
        .count();
    assert_eq!(result_messages, 1, "tool_result 不得拆多条消息: {msgs:?}");
}

/// ttft 断言辅助:尾帧 ttftMs 存在且 < 5000(mock 下应为亚秒;负载下
/// 允许毫秒级抖动,精确 0ms 在并行测试下偶发为 1ms)
fn ttft_within_bounds(events: &[LlmEvent]) -> bool {
    events.iter().rev().any(|e| match e {
        LlmEvent::Usage(u) => u
            .get("ttftMs")
            .and_then(Value::as_u64)
            .is_some_and(|ms| ms < 5000),
        _ => false,
    })
}

/// 扩展思考流:thinking_delta → Reasoning(signature_delta 忽略),
/// 正文不被思考文本污染(回归锁)
#[tokio::test]
async fn anthropic_thinking_delta_reasons_without_polluting_content() {
    let body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"test-model\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"先想\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"一下\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig==\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"答案\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":9}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    let (base_url, _captured) = spawn_sse_server(body).await;
    let mut t = transport(base_url, "sk-ant-test", "anthropic-messages");
    let events = t
        .stream(
            &header("", vec![]),
            &json!([{ "role": "user", "content": "q" }]),
        )
        .await
        .expect("stream");
    assert_eq!(
        &events[..events.len() - 1],
        &[
            LlmEvent::Reasoning("先想".into()),
            LlmEvent::Reasoning("一下".into()),
            LlmEvent::Chunk("答案".into()),
            LlmEvent::Usage(json!({"input_tokens": 1, "output_tokens": 9})),
            LlmEvent::AssistantMessage(json!({
                "content": "答案", "tool_calls": []
            })),
            LlmEvent::Done,
        ][..],
        "{events:?}"
    );
    assert!(ttft_within_bounds(&events), "{events:?}");
}

/// anthropic 兼容面思考档位锁(按模型分派;一手依据 = DeepSeek 官方
/// harness serialize.ts + ZCode builtin 配置矩阵,2026-10-02):
/// glm-5.3/deepseek/qwen3.8 = thinking:enabled + output_config:{effort};
/// glm 老模型 = 仅 thinking 开关;kimi = 仅 output_config;
/// 矩阵外(官方 claude)= 不发(rig additional_params flatten 落 body 顶层)
#[tokio::test]
async fn anthropic_thinking_params_by_model() {
    let body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"test\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    let case = |model: String, effort: Option<String>| async move {
        let (base_url, captured) = spawn_sse_server(body).await;
        let mut t = transport(base_url, "k", "anthropic-messages");
        let mut h = header("", vec![]);
        h.model = model;
        h.reasoning_effort = effort;
        let _ = t
            .stream(&h, &json!([{ "role": "user", "content": "hi" }]))
            .await;
        sent_body(&captured)
    };
    // glm-5.3:两字段
    let sent = case("glm-5.3-flash".into(), Some("max".into())).await;
    assert_eq!(sent["thinking"], json!({"type": "enabled"}), "{sent}");
    assert_eq!(sent["output_config"], json!({"effort": "max"}), "{sent}");
    // deepseek:同形(dsh 一手实现)
    let sent = case("deepseek-flash".into(), Some("high".into())).await;
    assert_eq!(sent["thinking"], json!({"type": "enabled"}), "{sent}");
    assert_eq!(sent["output_config"], json!({"effort": "high"}), "{sent}");
    // glm 老模型:仅开关
    let sent = case("glm-4.7".into(), Some("max".into())).await;
    assert_eq!(sent["thinking"], json!({"type": "enabled"}), "{sent}");
    assert!(sent.get("output_config").is_none(), "{sent}");
    // kimi:仅档位
    let sent = case("kimi-k3".into(), Some("low".into())).await;
    assert_eq!(sent["output_config"], json!({"effort": "low"}), "{sent}");
    assert!(sent.get("thinking").is_none(), "{sent}");
    // 矩阵外(官方 claude):不发
    let sent = case("claude-sonnet-5".into(), Some("max".into())).await;
    assert!(sent.get("thinking").is_none(), "{sent}");
    assert!(sent.get("output_config").is_none(), "{sent}");
    // 未配置档位:什么都不发(现状语义)
    let sent = case("glm-5.3-flash".into(), None).await;
    assert!(sent.get("thinking").is_none(), "{sent}");
}

/// 工具调用流:input_json_delta 累积 → 定稿 AssistantMessage(内部方言
/// 扁平 tool_calls)
#[tokio::test]
async fn anthropic_stream_with_tool_use() {
    let body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"test-model\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Let me\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"bash\",\"input\":{}}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"command\\\":\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"echo hi\\\"}\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":42}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    let (base_url, _captured) = spawn_sse_server(body).await;
    let mut t = transport(base_url, "sk-ant-test", "anthropic-messages");
    let events = t
        .stream(
            &header("", vec![]),
            &json!([{ "role": "user", "content": "q" }]),
        )
        .await
        .expect("stream");
    let assistant = events
        .iter()
        .find_map(|e| match e {
            LlmEvent::AssistantMessage(m) => Some(m.clone()),
            _ => None,
        })
        .expect("工具调用应定稿");
    assert_eq!(
        assistant,
        json!({
            "content": "Let me",
            "tool_calls": [ {
                "id": "toolu_1", "name": "bash",
                "arguments": "{\"command\":\"echo hi\"}"
            } ],
        })
    );
    // usage 折叠单帧:input(message_start)+output(message_delta)并一
    assert!(
        events.contains(&LlmEvent::Usage(json!({
            "input_tokens": 10, "output_tokens": 42
        }))),
        "{events:?}"
    );
    assert!(events.iter().any(|e| matches!(e, LlmEvent::Done)));
}

// ============================================================
// Responses 家族
// ============================================================

/// deepseek-responses HTTP 层锁:打到 `{base}/responses`、Bearer 鉴权、
/// reasoning/usage 事件穿透(fixtures 官方完整形态)
#[tokio::test]
async fn deepseek_responses_http_endpoint_and_events() {
    let body = concat!(
        "data: {\"type\":\"response.reasoning_text.delta\",\"item_id\":\"rs_1\",\"output_index\":0,\"content_index\":0,\"sequence_number\":1,\"delta\":\" hmm\"}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_1\",\"output_index\":1,\"content_index\":0,\"sequence_number\":2,\"delta\":\"hi\"}\n\n",
        "data: {\"type\":\"response.completed\",\"sequence_number\":3,\"response\":{\"id\":\"resp_1\",\"object\":\"response\",\"created_at\":0,\"model\":\"test-model\",\"status\":\"completed\",\"output\":[{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"hi\"}]}],\"usage\":{\"input_tokens\":7,\"output_tokens\":2,\"total_tokens\":9}}}\n\n",
    );
    let (base_url, captured) = spawn_sse_server(body).await;
    let mut t = transport(base_url, "sk-ds-test", "deepseek-responses");
    let events = t
        .stream(
            &header("", vec![]),
            &json!([{ "role": "user", "content": "hi" }]),
        )
        .await
        .expect("stream");

    let raw = sent_raw(&captured);
    let request_line = raw.lines().next().unwrap();
    assert!(
        request_line.contains("POST /responses"),
        "got: {request_line}"
    );
    assert!(
        raw.contains("authorization: Bearer sk-ds-test"),
        "Bearer 缺失"
    );
    let body = sent_body(&captured);
    assert!(body.get("input").is_some(), "responses 请求体缺 input");
    assert!(
        body.get("stream_options").is_none(),
        "responses 不发 stream_options"
    );

    assert_eq!(
        &events[..events.len() - 1],
        &[
            LlmEvent::Reasoning(" hmm".into()),
            LlmEvent::Chunk("hi".into()),
            LlmEvent::Usage(json!({"input_tokens": 7, "output_tokens": 2})),
            LlmEvent::AssistantMessage(json!({"content": "hi", "tool_calls": []})),
            LlmEvent::Done,
        ][..],
        "{events:?}"
    );
    assert!(ttft_within_bounds(&events), "{events:?}");
}

/// deepseek-responses wire 锁:思考强度 = `reasoning:{effort}`(与
/// OpenAI 同形);temperature 必发;instructions 承载 system
#[tokio::test]
async fn deepseek_responses_request_shape() {
    let body = "data: {\"type\":\"response.completed\",\"sequence_number\":1,\"response\":{\"id\":\"r\",\"object\":\"response\",\"created_at\":0,\"model\":\"m\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n";
    let (base_url, captured) = spawn_sse_server(body).await;
    let mut t = transport(base_url, "k", "deepseek-responses");
    let mut h = header("be brief", vec![]);
    h.reasoning_effort = Some("max".into());
    h.temperature = 0.7;
    let _ = t
        .stream(&h, &json!([{ "role": "user", "content": "hi" }]))
        .await;
    let body = sent_body(&captured);
    assert_eq!(body["reasoning"]["effort"], "max", "{body}");
    assert_eq!(body["temperature"], 0.7);
    assert_eq!(body["instructions"], "be brief");
    assert_eq!(body["stream"], true);
    assert!(
        body.get("output_config").is_none(),
        "effort 表达是 reasoning 对象"
    );
}

/// 首 token 判定锁:首个非空 delta(含推理);纯 reasoning 流也合成
/// ttftMs 尾帧(88 步工具重会话 86 步无 TTFT 的事故锁)
#[tokio::test]
async fn reasoning_only_stream_still_yields_ttft() {
    let body = concat!(
        "data: {\"type\":\"response.reasoning_text.delta\",\"item_id\":\"rs_1\",\"output_index\":0,\"content_index\":0,\"sequence_number\":1,\"delta\":\" think\"}\n\n",
        "data: {\"type\":\"response.reasoning_text.delta\",\"item_id\":\"rs_1\",\"output_index\":0,\"content_index\":0,\"sequence_number\":2,\"delta\":\"ing\"}\n\n",
        "data: {\"type\":\"response.completed\",\"sequence_number\":3,\"response\":{\"id\":\"r\",\"object\":\"response\",\"created_at\":0,\"model\":\"m\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":5,\"output_tokens\":1,\"total_tokens\":6}}}\n\n",
    );
    let (base_url, _captured) = spawn_sse_server(body).await;
    let mut t = transport(base_url, "sk-ds-test", "deepseek-responses");
    let events = t
        .stream(
            &header("", vec![]),
            &json!([{ "role": "user", "content": "hi" }]),
        )
        .await
        .expect("stream");
    let tail = events.iter().rev().find_map(|e| match e {
        LlmEvent::Usage(u) if u.get("ttftMs").is_some() => Some(u.clone()),
        _ => None,
    });
    assert!(
        tail.is_some(),
        "纯 reasoning 流也应合成 ttftMs 尾帧: {events:?}"
    );
}

/// responses 工具往返 wire:assistant → function_call item;
/// tool → function_call_output item;tools 扁平
#[tokio::test]
async fn responses_tool_roundtrip_wire_shape() {
    let body = "data: {\"type\":\"response.completed\",\"sequence_number\":1,\"response\":{\"id\":\"r\",\"object\":\"response\",\"created_at\":0,\"model\":\"m\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n";
    let (base_url, captured) = spawn_sse_server(body).await;
    let mut t = transport(base_url, "k", "openai-responses");
    let _ = t
        .stream(
            &header("be brief", vec![bash_tool()]),
            &tool_roundtrip_messages(),
        )
        .await;
    let body = sent_body(&captured);
    assert_eq!(body["instructions"], "be brief");
    assert_eq!(body["tools"][0]["name"], "bash");
    assert!(
        body["tools"][0].get("function").is_none(),
        "Responses 扁平形状"
    );
    let input = body["input"].as_array().unwrap();
    let fc = input
        .iter()
        .find(|i| i["type"] == "function_call")
        .expect("assistant 应译为 function_call item");
    assert_eq!(fc["call_id"], "toolu_1", "{fc}");
    assert_eq!(fc["arguments"], json!("{\"command\":\"echo hi\"}"), "{fc}");
    let fco = input
        .iter()
        .find(|i| i["type"] == "function_call_output")
        .expect("tool 应译为 function_call_output item");
    assert_eq!(fco["output"], "hi", "{fco}");
}

/// Responses 用户图片:全家族(OpenAI/GLM/DeepSeek)→ input_image
/// data-URL。deepseek 降级逻辑已于 2026-10-02 移除:官方 Responses
/// 文档与 deepseek-flash 实际能力均支持 user 面图片(旧「纯文本模型」
/// 结论是 deepseek-chat 时代的过时断言)
#[tokio::test]
async fn responses_user_image_wire_shape() {
    use liuma_llm::attachments::AttachmentSource;
    struct Fixed;
    impl AttachmentSource for Fixed {
        fn image_bytes(&self, _id: &str) -> Option<Vec<u8>> {
            Some(vec![1, 2, 3])
        }
    }
    let messages = json!([
        { "role": "user", "content": [
            { "type": "text", "text": "如图" },
            { "type": "image", "mediaType": "image/png", "data": "",
              "attachment": { "attachmentId": "sha256:abc",
                  "mediaType": "image/png", "bytes": 3 } },
        ]},
    ]);
    let body = "data: {\"type\":\"response.completed\",\"sequence_number\":1,\"response\":{\"id\":\"r\",\"object\":\"response\",\"created_at\":0,\"model\":\"m\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n";

    // (方言, 模型, 是否携带图)——模型级能力:deepseek-v4-pro 未编目
    // 能力外 → 占位降级(安全缺省);flash 系两厂都编目为有图
    for (dialect, model, has_image) in [
        ("openai-responses", "gpt-5.6", true),
        ("glm-responses", "glm-5.3-flash", true),
        ("deepseek-responses", "deepseek-flash", true),
        ("deepseek-responses", "deepseek-v4-pro", false),
    ] {
        let (base_url, captured) = spawn_sse_server(body).await;
        let mut t = HttpTransport::with_adapter(
            ProviderConfig {
                base_url,
                api_key: "k".into(),
            },
            liuma_llm::adapter_by_name(dialect).unwrap(),
        )
        .unwrap()
        .with_attachments(Arc::new(Fixed));
        let mut h = header("s", vec![]);
        h.model = model.into();
        let _ = t.stream(&h, &messages).await;
        let sent = sent_body(&captured);
        // rig 把 user 内容部件各自包成独立 input item,图片跨 item 扫描
        let content = sent["input"][0]["content"].clone();
        let img = sent["input"].as_array().and_then(|items| {
            items
                .iter()
                .flat_map(|i| i["content"].as_array().cloned().unwrap_or_default())
                .find(|p| p["type"] == "input_image")
        });
        if has_image {
            let img = img.unwrap_or_else(|| panic!("{dialect}/{model} 应含 input_image: {sent}"));
            assert_eq!(img["image_url"], "data:image/png;base64,AQID", "{dialect}");
            assert_eq!(content[0]["type"], "input_text", "{dialect}");
        } else {
            // 能力外模型 → 占位降级,不误发 input_image
            assert!(img.is_none(), "{model} 不应含 input_image: {sent}");
            let all_text = sent["input"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .flat_map(|i| i["content"].as_array().cloned().unwrap_or_default())
                        .map(|b| b["text"].as_str().unwrap_or_default().to_string())
                        .collect::<String>()
                })
                .unwrap_or_default();
            assert!(
                all_text.contains(liuma_llm::OFFLOADED_IMAGE_TEXT),
                "{model} 应降级占位: {sent}"
            );
        }
    }
}

/// 端到端:glm-responses 对含图 user 消息,出网请求体必须含
/// input_image data-URL 而非占位文本(真机误报回归锁)
#[tokio::test]
async fn glm_responses_http_request_carries_input_image() {
    use liuma_llm::attachments::AttachmentSource;
    struct Fixed;
    impl AttachmentSource for Fixed {
        fn image_bytes(&self, _id: &str) -> Option<Vec<u8>> {
            Some(vec![1, 2, 3])
        }
    }
    let body = "data: {\"type\":\"response.completed\",\"sequence_number\":1,\"response\":{\"id\":\"r\",\"object\":\"response\",\"created_at\":0,\"model\":\"m\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":3,\"output_tokens\":1,\"total_tokens\":4}}}\n\n";
    let (base_url, captured) = spawn_sse_server(body).await;
    let mut t = HttpTransport::with_adapter(
        ProviderConfig {
            base_url,
            api_key: "glm-test".into(),
        },
        liuma_llm::adapter_by_name("glm-responses").unwrap(),
    )
    .unwrap()
    .with_attachments(Arc::new(Fixed));
    let messages = json!([
        { "role": "user", "content": [
            { "type": "image", "attachment": {
                "attachmentId": "sha256:abc", "mediaType": "image/png",
                "bytes": 3, "width": 1, "height": 1 } },
            { "type": "text", "text": "如图" },
        ]},
    ]);
    // 模型级能力:glm-5.3-flash 编目为有图(ZCode modelRules 998);
    // 未编目模型名会按 text-only 缺省降级
    let mut h = header("", vec![]);
    h.model = "glm-5.3-flash".into();
    t.stream(&h, &messages).await.expect("stream");
    let raw = sent_raw(&captured);
    let json_body = raw.split("\r\n\r\n").nth(1).unwrap_or("");
    assert!(json_body.contains("input_image"), "实际 {json_body}");
    assert!(
        !json_body.contains(liuma_llm::OFFLOADED_IMAGE_TEXT),
        "不应含占位文本: {json_body}"
    );
}

// ============================================================
// Chat 家族
// ============================================================

/// deepseek-chat 思考控制 wire 锁(8d1e722 事故锁迁移):`thinking` 与
/// `reasoning_effort` 是两个顶层字段;include_usage 流选项必发
#[tokio::test]
async fn deepseek_chat_wire_thinking_and_usage() {
    let body =
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
    let (base_url, captured) = spawn_sse_server(body).await;
    let mut t = transport(base_url, "k", "deepseek-chat");
    let mut h = header("s", vec![]);
    h.reasoning_effort = Some("max".into());
    let _ = t
        .stream(&h, &json!([{ "role": "user", "content": "hi" }]))
        .await;
    let body = sent_body(&captured);
    assert_eq!(body["thinking"], json!({"type": "enabled"}), "{body}");
    assert_eq!(body["reasoning_effort"], "max");
    assert_eq!(
        body["thinking"]["reasoning_effort"],
        Value::Null,
        "不得嵌进 thinking"
    );
    assert_eq!(body["stream_options"]["include_usage"], true);
}

/// 净版 chat 方言(剥私参):带 effort 也不发方言私有字段
#[tokio::test]
async fn openai_completions_plain_strips_private_fields() {
    let body =
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
    let (base_url, captured) = spawn_sse_server(body).await;
    let mut t = transport(base_url, "k", "openai-completions");
    let mut h = header("s", vec![]);
    h.reasoning_effort = Some("max".into());
    let _ = t
        .stream(&h, &json!([{ "role": "user", "content": "hi" }]))
        .await;
    let body = sent_body(&captured);
    assert_eq!(body["thinking"], Value::Null, "{body}");
    assert_eq!(body["reasoning_effort"], Value::Null);
    assert_eq!(body["stream_options"]["include_usage"], true);
}

// ============================================================
// 真实 GLM 流终裁(spike 遗产:2026-10-02 实抓,bigmodel glm-5.3-flash)
// ============================================================

/// 真实 GLM 流(60 帧 reasoning + 定稿)必须经 rig 严格解码零失败,
/// 且载荷级事件齐备(Reasoning/Chunk/Usage/Assistant/Done + ttft)
#[tokio::test]
async fn real_glm_stream_decodes_end_to_end() {
    const REAL: &str = include_str!("fixtures/glm-responses-real.sse");
    let (base_url, _captured) = spawn_sse_server(REAL).await;
    let mut t = transport(base_url, "glm-test", "glm-responses");
    let events = t
        .stream(
            &header("", vec![]),
            &json!([{ "role": "user", "content": "1+1" }]),
        )
        .await
        .expect("真实 GLM 流应完整解码");
    // 载荷齐备性(内容断言不锁文本,真机流内容会变)
    assert!(
        events.iter().any(|e| matches!(e, LlmEvent::Reasoning(_))),
        "{events:?}"
    );
    assert!(events.iter().any(|e| matches!(e, LlmEvent::Chunk(_))));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, LlmEvent::Usage(u) if u.get("input_tokens") == Some(&json!(22)))),
        "usage 应含真实 input_tokens: {events:?}"
    );
    assert!(events.iter().any(|e| matches!(e, LlmEvent::Done)));
}

// ============================================================
// 模型能力标记(providers 层)
// ============================================================

/// 能力解析锁(一手转录表;模型名即键):deepseek flash 系有图/
/// v4-pro 纯文本;glm-5.3-flash 有图而 glm-5.3 纯文本(Responses 面);
/// anthropic 面 glm 站点级全开(覆盖厂商目录结论)
#[test]
fn model_caps_resolution() {
    use liuma_llm::providers::model_caps;
    // deepseek(Responses 面):flash 系图,其余纯文本(dsh 目录)
    assert!(
        model_caps("deepseek-responses", "deepseek-flash")
            .unwrap()
            .image
    );
    assert!(
        model_caps("deepseek-responses", "deepseek-v4.1-flash")
            .unwrap()
            .image
    );
    assert!(
        !model_caps("deepseek-responses", "deepseek-v4-pro")
            .unwrap()
            .image
    );
    // glm(Responses 面):5.3-flash 图(ZCode 998),5.3 纯文本(978)
    assert!(model_caps("glm-responses", "glm-5.3-flash").unwrap().image);
    assert!(!model_caps("glm-responses", "glm-5.3").unwrap().image);
    // anthropic 面:GLM 站点级覆盖全开(providerSiteRules 4011-4047)
    assert!(model_caps("anthropic-messages", "glm-5.3").unwrap().image);
    assert!(model_caps("anthropic-messages", "glm-4.7").unwrap().image);
    // anthropic 面:kimi k3-256k 仅图(k3 通配先被特例挡住)
    let k = model_caps("anthropic-messages", "kimi-k3").unwrap();
    assert!(k.image && k.video);
    let k256 = model_caps("anthropic-messages", "k3-256k").unwrap();
    assert!(k256.image && !k256.video, "k3-256k 特例先于 k3 通配");
    // 未知方言 → None(调用方 fail-fast)
    assert!(model_caps("nope", "m").is_none());
}

// ============================================================
// hosted 工具(请求透传 + 响应记录 + 历史回放)
// ============================================================

/// anthropic 面 hosted 请求锁:通用 kind → 版本化 wire 形态
/// (glm = web_search_2026_02_09(ZCode);deepseek = web_search_20250305
/// (dsh));config 字段透传;function 工具同面共存
#[tokio::test]
async fn anthropic_hosted_web_search_wire() {
    let body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"t\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    for (model, wire_type) in [
        ("glm-5.3", "web_search_2026_02_09"),
        ("deepseek-flash", "web_search_20250305"),
    ] {
        let (base_url, captured) = spawn_sse_server(body).await;
        let mut t = transport(base_url, "k", "anthropic-messages");
        let mut h = header(
            "",
            vec![
                bash_tool(),
                json!({
                    "type": "web_search", "max_uses": 3, "allowed_domains": ["rust-lang.org"],
                }),
            ],
        );
        h.model = model.into();
        let _ = t
            .stream(&h, &json!([{ "role": "user", "content": "q" }]))
            .await;
        let sent = sent_body(&captured);
        let tools = sent["tools"].as_array().unwrap();
        // function 条目与 hosted 条目同面共存
        assert_eq!(tools[0]["name"], "bash", "{model}");
        let hosted = &tools[1];
        assert_eq!(hosted["type"], wire_type, "{model}: {hosted}");
        assert_eq!(hosted["name"], "web_search", "{model}: {hosted}");
        assert_eq!(hosted["max_uses"], 3, "{model}: {hosted}");
        assert_eq!(
            hosted["allowed_domains"],
            json!(["rust-lang.org"]),
            "{model}"
        );
    }
}

/// responses 面 hosted 请求锁:无版本化 type、无 name 字段
/// (rig `ResponsesToolDefinition::web_search()` 同款形态)
#[tokio::test]
async fn responses_hosted_web_search_wire() {
    let body = "data: {\"type\":\"response.completed\",\"sequence_number\":1,\"response\":{\"id\":\"r\",\"object\":\"response\",\"created_at\":0,\"model\":\"m\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n";
    let (base_url, captured) = spawn_sse_server(body).await;
    let mut t = transport(base_url, "k", "openai-responses");
    let mut h = header("", vec![json!({ "type": "web_search", "max_uses": 2 })]);
    h.model = "gpt-5.6".into();
    let _ = t
        .stream(&h, &json!([{ "role": "user", "content": "q" }]))
        .await;
    let sent = sent_body(&captured);
    let hosted = &sent["tools"][0];
    assert_eq!(hosted["type"], "web_search", "{sent}");
    assert!(
        hosted.get("name").is_none(),
        "responses 面 hosted 无 name: {hosted}"
    );
    assert_eq!(hosted["max_uses"], 2);
    // function 形态专属的 strict 不得出现在 hosted 条目(400 风险)
    assert!(hosted.get("strict").is_none(), "{hosted}");
}

/// 面/模型不提供 hosted 工具 → fail-fast(chat 面无声明表;
/// 未知 kind 同路),不得静默发出空定义
#[tokio::test]
async fn hosted_tool_unsupported_fails_fast() {
    let (base_url, _captured) = spawn_sse_server("").await;
    let mut t = transport(base_url, "k", "openai-completions");
    let h = header("", vec![json!({ "type": "web_search" })]);
    let err = t
        .stream(&h, &json!([{ "role": "user", "content": "q" }]))
        .await
        .expect_err("chat 面不提供 hosted 工具");
    assert!(err.to_string().contains("hosted"), "{err}");
}

/// anthropic 面 hosted 响应锁:server_tool_use 与 web_search_tool_result
/// 块收集进定稿 AssistantMessage.server_tools(原始块形状),
/// 正文不被污染;经 Item 元数据通道,不再静默丢弃
#[tokio::test]
async fn anthropic_hosted_results_recorded() {
    let body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"t\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"server_tool_use\",\"id\":\"srvu_1\",\"name\":\"web_search\",\"input\":{}}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"query\\\":\\\"rust\\\"}\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"web_search_tool_result\",\"tool_use_id\":\"srvu_1\",\"content\":[]}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"text_delta\",\"text\":\"找到了\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":2}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":9}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    let (base_url, _captured) = spawn_sse_server(body).await;
    let mut t = transport(base_url, "k", "anthropic-messages");
    let events = t
        .stream(
            &header("", vec![]),
            &json!([{ "role": "user", "content": "q" }]),
        )
        .await
        .expect("stream");
    let assistant = events
        .iter()
        .find_map(|e| match e {
            LlmEvent::AssistantMessage(m) => Some(m.clone()),
            _ => None,
        })
        .expect("应定稿 AssistantMessage");
    // 正文不被 hosted 块污染
    assert_eq!(assistant["content"], "找到了", "{assistant}");
    // server_tools 收集:server_tool_use(含 input_json_delta 组装的
    // query)与 web_search_tool_result 两块
    let st = assistant["server_tools"]
        .as_array()
        .expect("server_tools 应在场");
    assert_eq!(st.len(), 2, "{assistant}");
    assert_eq!(st[0]["type"], "server_tool_use", "{assistant}");
    assert_eq!(st[0]["input"]["query"], "rust", "{assistant}");
    assert_eq!(st[1]["type"], "web_search_tool_result", "{assistant}");
    assert_eq!(st[1]["tool_use_id"], "srvu_1", "{assistant}");
}

/// hosted 历史回放锁:上一轮 AssistantMessage.server_tools 在下一轮
/// 请求中经 rig 元数据通道回放为 wire 块(anthropic 面)
#[tokio::test]
async fn anthropic_hosted_results_replayed_in_history() {
    let body = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"t\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    let (base_url, captured) = spawn_sse_server(body).await;
    let mut t = transport(base_url, "k", "anthropic-messages");
    let messages = json!([
        { "role": "user", "content": "搜一下" },
        { "role": "assistant", "content": "找到了",
          "server_tools": [
            { "type": "server_tool_use", "id": "srvu_1", "name": "web_search",
              "input": { "query": "rust" } },
            { "type": "web_search_tool_result", "tool_use_id": "srvu_1", "content": [] },
        ]},
        { "role": "user", "content": "继续" },
    ]);
    let _ = t.stream(&header("", vec![]), &messages).await;
    let raw = sent_raw(&captured);
    // 回放块出现在出网 body(rig anthropic_content 通道重建)
    assert!(raw.contains("server_tool_use"), "回放缺失: {raw}");
    assert!(raw.contains("web_search_tool_result"), "回放缺失: {raw}");
    assert!(
        raw.contains("\"query\":\"rust\"") || raw.contains("\\\"query\\\":\\\"rust\\\""),
        "input 回放: {raw}"
    );
}
