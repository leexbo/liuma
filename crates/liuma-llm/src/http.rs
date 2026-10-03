//! 真实 LLM HTTP transport:rig 0.43 provider 层 + 自有传输桥。
//!
//! 本层组装「方言声明 + rig provider + [`crate::bridge`] 传输桥」:
//! 请求编码/流解码在 rig-core(连接语义之外的全部 wire 知识归 rig,
//! 方言残留差异在 [`crate::adapters`]);连接语义(连接池/超时/错误
//! 体嗅探)在桥。策略(重试/限额)不在此层——D41 在 engine。
//!
//! 默认 openai-completions 方言([`HttpTransport::new`]);anthropic/
//! responses/chat 各方言经 [`HttpTransport::with_adapter`] 接入。

use std::sync::Arc;
use std::time::Duration;

use liuma_agent_loop::{LlmEvent, LlmTransport, RequestHeader, TransportError};
use serde_json::Value;

use crate::adapters::ProviderAdapter;
use crate::attachments::{AttachmentSource, NoAttachments};

/// reqwest 发送/读体错误归类:超时(连接超时/读超时)→ TIMEOUT,
/// 其余(连接失败/DNS/TLS/流中断)→ TRANSPORT
pub(crate) fn classify_reqwest(e: reqwest::Error) -> TransportError {
    if e.is_timeout() {
        TransportError::Timeout(e.to_string())
    } else {
        TransportError::Transport(e.to_string())
    }
}

/// 400/413 响应体是否「上下文超长」(provider 用词各不同;命中即归
/// CONTEXT_OVERFLOW,由 engine 强制压缩后重试一次而非盲目重发)。
/// 大小写不敏感子串匹配;宁可漏判(回落 INVALID_REQUEST 直通)不误判
/// (误判会对无关 400 触发一次压缩)。
pub(crate) fn looks_like_context_overflow(body: &str) -> bool {
    let b = body.to_ascii_lowercase();
    [
        "context length",
        "context_length",
        "context window",
        "context size",
        "maximum context",
        "max context",
        "context limit",
        "too many tokens",
        "prompt is too long",
        "input is too long",
        "request is too large",
        "exceeds the maximum",
        "reduce the length",
        "truncated",
        // GLM(bigmodel)1261「Prompt 超长」——中文文案与业务码两种形态
        // (官方错误码表 2026-10 核实;引号形态防裸数字误命中)
        "prompt 超长",
        "\"code\":\"1261\"",
        "\"code\":1261",
    ]
    .iter()
    .any(|p| b.contains(p))
}

/// 非 2xx 状态归类(401/403 → AUTH;400/413 → CONTEXT_OVERFLOW(体命中
/// 超长用语)/ INVALID_REQUEST;429 → RATE_LIMIT;≥500 → SERVER;其余
/// 未分类直通)。crate 内共享(bridge/adapters 的错误归类复用状态映射)
pub(crate) fn classify_status(
    status: reqwest::StatusCode,
    retry_after: Option<String>,
    body: String,
) -> TransportError {
    if matches!(
        status,
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
    ) {
        return TransportError::Auth {
            status: status.as_u16(),
            body,
        };
    }
    if matches!(
        status,
        reqwest::StatusCode::BAD_REQUEST | reqwest::StatusCode::PAYLOAD_TOO_LARGE
    ) {
        return if looks_like_context_overflow(&body) {
            TransportError::ContextOverflow {
                status: status.as_u16(),
                body,
            }
        } else {
            TransportError::InvalidRequest {
                status: status.as_u16(),
                body,
            }
        };
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return TransportError::RateLimit {
            retry_after_ms: retry_after.as_deref().and_then(parse_retry_after_ms),
            body,
        };
    }
    if status.is_server_error() {
        return TransportError::Server {
            status: status.as_u16(),
            body,
        };
    }
    TransportError::Other(format!("provider {status}: {body}"))
}

/// Retry-After 解析:整秒数值(毫秒换算,≤0 视为缺失)。
/// HTTP 日期形式不支持(回落 None → 本地退避)
fn parse_retry_after_ms(v: &str) -> Option<u64> {
    v.trim()
        .parse::<u64>()
        .ok()
        .map(|secs| secs * 1000)
        .filter(|ms| *ms > 0)
}

/// 首块是否 SSE 帧(字段行 data:/event:/id:/retry: / 注释行;容忍空白前缀)
pub(crate) fn looks_like_sse(first: &[u8]) -> bool {
    let t = String::from_utf8_lossy(first);
    let t = t.trim_start();
    ["data:", "event:", "id:", "retry:", ":"]
        .iter()
        .any(|p| t.starts_with(p))
}

/// 连接建立超时(TCP+TLS)
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 读超时:任意两次字节间的最长等待。覆盖「请求已发出但服务端零
/// 响应」与「流式中途断流」两类悬挂;活跃流式的 chunk 间隔远小于此
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// provider 连接配置
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// API base URL(方言相关:anthropic-messages 形如
    /// `https://api.anthropic.com`;openai 系形如 `https://api.openai.com/v1`)
    pub base_url: String,
    /// API key(鉴权头形状由 rig 方言决定:Bearer / x-api-key)
    pub api_key: String,
}

/// 流式 transport:rig provider 层 + 自有传输桥。
pub struct HttpTransport {
    client: reqwest::Client,
    config: ProviderConfig,
    adapter: Arc<dyn ProviderAdapter>,
    /// 图片附件字节来源(请求期组 data URL;缺省 = 无来源,图片降级占位)
    attachments: Arc<dyn AttachmentSource>,
}

impl HttpTransport {
    /// 以配置构建(默认 openai-completions 方言;既有行为不变)
    pub fn new(config: ProviderConfig) -> Result<Self, String> {
        Self::with_adapter(
            config,
            Box::new(crate::adapters::OpenAiCompletionsAdapter::default()),
        )
    }

    /// 以指定方言 adapter 构建(anthropic / responses / chat 系)
    pub fn with_adapter(
        config: ProviderConfig,
        adapter: Box<dyn ProviderAdapter>,
    ) -> Result<Self, String> {
        Self::with_timeouts(config, adapter, CONNECT_TIMEOUT, READ_TIMEOUT)
    }

    /// 显式超时构建(测试注入短超时;语义同 [`Self::with_adapter`])
    pub fn with_timeouts(
        config: ProviderConfig,
        adapter: Box<dyn ProviderAdapter>,
        connect: Duration,
        read: Duration,
    ) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .connect_timeout(connect)
            .read_timeout(read)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            config,
            adapter: Arc::from(adapter),
            attachments: Arc::new(NoAttachments),
        })
    }

    /// 方言错误体钩子:捕获**本实例**(含自定义行为组;不再按方言名
    /// 反查重建——那会静默丢掉自定义 behaviors)
    fn body_error_hook(&self) -> crate::bridge::BodyErrorHook {
        let adapter = Arc::clone(&self.adapter);
        Arc::new(move |body| adapter.body_error(body))
    }

    /// 注入图片附件字节来源(装配点;registry 传 liuma-host AttachmentStore)
    pub fn with_attachments(mut self, source: Arc<dyn AttachmentSource>) -> Self {
        self.attachments = source;
        self
    }
}

impl LlmTransport for HttpTransport {
    async fn stream(
        &mut self,
        header: &RequestHeader,
        messages: &Value,
    ) -> Result<Vec<LlmEvent>, TransportError> {
        let mut events = Vec::new();
        let mut collect = |e: LlmEvent| {
            events.push(e);
        };
        let body_error = self.body_error_hook();
        crate::engine::stream(
            self.adapter.as_ref(),
            &self.config,
            self.client.clone(),
            header,
            messages,
            self.attachments.as_ref(),
            body_error,
            &mut collect,
        )
        .await?;
        Ok(events)
    }

    /// 流式路径:事件到达即经 channel 下发(chunk 到达即推送,引擎
    /// select 逐条落档广播 → 前端逐 token 渲染)。请求/鉴权同
    /// [`LlmTransport::stream`]。
    async fn stream_events(
        &mut self,
        header: &RequestHeader,
        messages: &Value,
        tx: tokio::sync::mpsc::UnboundedSender<LlmEvent>,
    ) -> Result<(), TransportError> {
        let mut forward = |e: LlmEvent| {
            let _ = tx.send(e);
        };
        let body_error = self.body_error_hook();
        crate::engine::stream(
            self.adapter.as_ref(),
            &self.config,
            self.client.clone(),
            header,
            messages,
            self.attachments.as_ref(),
            body_error,
            &mut forward,
        )
        .await
    }
}

/// 一次性摘要调用:保留会话请求的 system 头与 tools(逐字前缀 = 上次
/// 路由请求的前缀,命中 provider KV cache),仅在消息尾追加含 checkpoint
/// 指令的最终 user 消息(liuma_compaction::COMPACTION_INSTRUCTION);
/// 累积流式文本为摘要。`messages` 语义 = 待折叠前缀(逐字,不含指令)。
impl liuma_agent_loop::Summarizer for HttpTransport {
    fn summarize<'a>(
        &'a mut self,
        header: &'a RequestHeader,
        messages: &'a Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>>
    {
        Box::pin(async move {
            let mut noop = |_chars: usize| {};
            self.summarize_stream(header, messages, &mut noop).await
        })
    }

    /// 逐块回调版:走流式路径,边收边累计正文长度(压缩进度条的唯一
    /// 真实数据源;推理段不计入)。
    fn summarize_stream<'a>(
        &'a mut self,
        header: &'a RequestHeader,
        messages: &'a Value,
        on_progress: &'a mut (dyn FnMut(usize) + Send),
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>>
    {
        Box::pin(async move {
            let one_shot = RequestHeader {
                model: header.model.clone(),
                system: header.system.clone(),
                temperature: header.temperature,
                reasoning_effort: header.reasoning_effort.clone(),
                tools: header.tools.clone(),
            };
            // 摘要 = 纯文本面(图块降级占位,不背 base64)
            let payload = crate::attachments::strip_images_for_summary(
                &liuma_compaction::summarization_messages(messages),
            );
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let producer = LlmTransport::stream_events(self, &one_shot, &payload, tx);
            let consumer = async {
                let mut text = String::new();
                let mut chars = 0usize;
                // 定稿优先:纯文本流既发 Chunk 直播增量、又在流末发携带
                // 定稿全文的 AssistantMessage(内容重叠);两者都拼接会把
                // 摘要文本拼两遍。Chunk 仅累积计数,取「定稿,否则累积」
                // (与 engine 侧消费范式一致)。
                let mut final_text: Option<String> = None;
                while let Some(event) = rx.recv().await {
                    match event {
                        LlmEvent::Chunk(delta) => {
                            chars += delta.chars().count();
                            text.push_str(&delta);
                        }
                        LlmEvent::AssistantMessage(m) => {
                            if let Some(c) = m["content"].as_str() {
                                final_text = Some(c.to_string());
                            }
                        }
                        _ => {}
                    }
                    on_progress(chars);
                }
                final_text.unwrap_or(text)
            };
            // 折叠请求上限 300s(手动 /compact 输入可达数百 KB,30s 不够);
            // 超时按折叠失败处理(自动路径降级跳过/手动路径报错)
            let (result, text) = tokio::time::timeout(Duration::from_secs(300), async {
                let (r, t) = tokio::join!(producer, consumer);
                (r, t)
            })
            .await
            .map_err(|_| "summarize timeout after 300s".to_string())?;
            result.map_err(|e| e.to_string())?;
            Ok(text)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use liuma_agent_loop::Summarizer as _;
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn header(tools: Vec<Value>) -> RequestHeader {
        RequestHeader {
            model: "test-model".into(),
            system: "be brief".into(),
            temperature: 0.1,
            reasoning_effort: None,
            tools,
        }
    }

    /// 400 归类:响应体命中超长用语 → CONTEXT_OVERFLOW(engine 走强制
    /// 压缩后重试一次,不盲目退避重发);其余 400 → INVALID_REQUEST 直通。
    /// 回归锁:两类必须互不误判(误判会对无关 400 触发压缩)。
    #[test]
    fn bad_request_separates_context_overflow_from_invalid_request() {
        use liuma_agent_loop::TransportError;
        for body in [
            "This model's maximum context length is 128000 tokens",
            "context_length_exceeded",
            "prompt is too long: 200000 tokens > 128000 maximum",
            // GLM 1261:中文文案与业务码两种形态(官方错误码表 2026-10)
            r#"{"error":{"code":"1261","message":"Prompt 超长"}}"#,
            "您的输入 Prompt 超长,请压缩后重试",
        ] {
            let e = classify_status(reqwest::StatusCode::BAD_REQUEST, None, body.into());
            assert!(e.is_context_overflow(), "应归溢出: {body}");
            assert_eq!(e.code(), "CONTEXT_OVERFLOW");
            assert!(!e.retryable(), "溢出不走盲目重试(engine 专用路径)");
        }
        let e = classify_status(
            reqwest::StatusCode::BAD_REQUEST,
            None,
            "{\"error\":{\"message\":\"invalid tool schema\"}}".into(),
        );
        assert!(
            matches!(e, TransportError::InvalidRequest { .. }),
            "普通 400 仍归 INVALID_REQUEST: {e:?}"
        );
        assert!(!e.is_context_overflow());
    }

    /// 极简 mock 服务器:接受一个连接,读请求,回固定 SSE 体(Connection: close 分帧)
    async fn spawn_sse_server(
        response_body: &'static str,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<u8>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_clone = std::sync::Arc::clone(&captured);
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 16384];
            let n = socket.read(&mut buf).await.unwrap();
            captured_clone.lock().unwrap().extend_from_slice(&buf[..n]);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{response_body}"
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        });
        (format!("http://{addr}"), captured)
    }

    /// openai-completions(chat wire)纯文本流:Chunk 直播 + Done;
    /// 纯文本轮不发 AssistantMessage、无 usage 帧不发 Usage(旧引擎同约)
    #[tokio::test]
    async fn streams_openai_sse_over_http() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n\
                    data: [DONE]\n\n";
        let (base_url, captured) = spawn_sse_server(body).await;

        let mut transport = HttpTransport::new(ProviderConfig {
            base_url,
            api_key: "sk-test".into(),
        })
        .unwrap();
        let events = transport
            .stream(
                &header(vec![]),
                &json!([{ "role": "user", "content": "hi" }]),
            )
            .await
            .expect("stream");
        assert_eq!(
            &events[..events.len() - 1],
            &[
                LlmEvent::Chunk("Hel".into()),
                LlmEvent::Chunk("lo".into()),
                // usage 帧(include_usage 语义)→ 归一五键
                LlmEvent::Usage(json!({ "input_tokens": 3, "output_tokens": 2 })),
                LlmEvent::Done,
            ][..],
            "{events:?}"
        );
        // transport 附带的首 token 指标(mock 往返亚秒;负载下毫秒级
        // 抖动,锁存在性 + 上界,不锁精确 0ms)
        assert!(
            events.iter().rev().any(|e| match e {
                LlmEvent::Usage(u) => u
                    .get("ttftMs")
                    .and_then(Value::as_u64)
                    .is_some_and(|ms| ms < 5000),
                _ => false,
            }),
            "{events:?}"
        );

        // 请求体断言:system 头部注入 + model/stream/temperature
        let raw = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        let body_start = raw.find("\r\n\r\n").expect("body") + 4;
        let sent: Value = serde_json::from_str(&raw[body_start..]).expect("json body");
        assert_eq!(sent["model"], "test-model");
        assert_eq!(sent["stream"], true);
        assert_eq!(sent["messages"][0]["role"], "system");
        assert_eq!(sent["messages"][1]["role"], "user");
        assert!(sent.get("tools").is_none(), "无工具声明时不带 tools 键");
        assert_eq!(
            sent["stream_options"]["include_usage"], true,
            "chat 方言 usage 流选项(rig Quirks 收敛面)"
        );
    }

    /// 工具闭环的 HTTP 侧:请求体带 tools/tool_choice(嵌套形);
    /// 流式 tool_calls 增量 → 定稿 AssistantMessage(扁平内部方言)
    #[tokio::test]
    async fn sends_tools_and_parses_streamed_tool_calls() {
        let body = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\
                    \"type\":\"function\",\"function\":{\"name\":\"bash\",\"arguments\":\"{\\\"co\"}}]}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":\
                    {\"arguments\":\"mmand\\\":\\\"echo hi\\\"}\"}}]}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n\
                    data: [DONE]\n\n";
        let (base_url, captured) = spawn_sse_server(body).await;

        let mut transport = HttpTransport::new(ProviderConfig {
            base_url,
            api_key: "sk-test".into(),
        })
        .unwrap();
        let events = transport
            .stream(
                &header(vec![json!({
                    "type": "function",
                    "function": {
                        "name": "bash",
                        "parameters": { "type": "object" },
                    },
                })]),
                &json!([{ "role": "user", "content": "run it" }]),
            )
            .await
            .expect("stream");

        // 请求体:tools(嵌套形)+ tool_choice(rig 形态 {type:auto})
        let raw = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        let body_start = raw.find("\r\n\r\n").expect("body") + 4;
        let sent: Value = serde_json::from_str(&raw[body_start..]).expect("json body");
        assert_eq!(sent["tools"][0]["function"]["name"], "bash");
        // tool_choice:rig chat 省缺省(默认即 auto;anthropic 家族发
        // {type:auto})——等价形态,不锁形
        let tc = &sent["tool_choice"];
        assert!(
            tc.is_null() || tc == &json!({"type": "auto"}),
            "tool_choice: {tc}"
        );

        // 响应流:累积为完整工具调用消息(无正文 → chat 家族不发
        // AssistantMessage 于文本面;tool_calls 在则发)
        let assistant = events
            .iter()
            .find_map(|e| match e {
                LlmEvent::AssistantMessage(m) => Some(m.clone()),
                _ => None,
            })
            .expect("工具调用应定稿为 AssistantMessage");
        assert_eq!(assistant["tool_calls"][0]["name"], "bash", "{assistant}");
        assert!(
            assistant["tool_calls"][0]["arguments"]
                .as_str()
                .is_some_and(|a| a.contains("echo hi")),
            "arguments 定稿为 JSON 串: {assistant}"
        );
        assert!(events.iter().any(|e| matches!(e, LlmEvent::Done)));
    }

    /// 回归锁:summarize 的出网请求必须保留会话 header 的 system 与
    /// tools(逐字前缀命中 KV cache),且以含 checkpoint 指令的最终
    /// user 消息收尾
    #[tokio::test]
    async fn summarize_replays_system_tools_and_appends_instruction() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"ckpt\"}}]}\n\n\
                    data: [DONE]\n\n";
        let (base_url, captured) = spawn_sse_server(body).await;

        let mut transport = HttpTransport::new(ProviderConfig {
            base_url,
            api_key: "sk-test".into(),
        })
        .unwrap();
        let h = RequestHeader {
            model: "test-model".into(),
            system: "session system prompt".into(),
            temperature: 0.3,
            reasoning_effort: None,
            tools: vec![json!({ "type": "function", "function": { "name": "bash" } })],
        };
        let prefix = json!([
            { "role": "user", "content": "old question" },
            { "role": "assistant", "content": "old answer" },
        ]);
        let summary = transport.summarize(&h, &prefix).await.expect("summary");
        assert_eq!(summary, "ckpt");

        let raw = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        let body_start = raw.find("\r\n\r\n").expect("body") + 4;
        let sent: Value = serde_json::from_str(&raw[body_start..]).expect("json body");
        assert_eq!(sent["model"], "test-model");
        assert_eq!(sent["temperature"], 0.3);
        // system 与 tools 原样保留(KV cache 前缀)
        assert_eq!(sent["messages"][0]["role"], "system");
        assert_eq!(
            sent["messages"][0]["content"][0]["text"],
            "session system prompt"
        );
        assert_eq!(sent["tools"][0]["function"]["name"], "bash");
        // 消息 = 逐字前缀 + 指令尾注(最终 user 消息)
        assert_eq!(sent["messages"][1]["content"], "old question");
        assert_eq!(sent["messages"][2]["content"][0]["text"], "old answer");
        assert_eq!(sent["messages"][3]["role"], "user");
        assert_eq!(
            sent["messages"][3]["content"],
            liuma_compaction::COMPACTION_INSTRUCTION
        );
        assert_eq!(sent["messages"].as_array().unwrap().len(), 4);
    }

    /// 悬挂服务端(接受连接后永不回包):读超时必须把 stream() 在
    /// 秒级转成 Err——否则 turn 永久悬挂零反馈
    #[tokio::test]
    async fn stalled_server_times_out() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let mut transport = HttpTransport::with_timeouts(
            ProviderConfig {
                base_url: format!("http://{addr}"),
                api_key: "sk-test".into(),
            },
            Box::new(crate::adapters::OpenAiCompletionsAdapter::default()),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap();
        let started = std::time::Instant::now();
        let result = transport
            .stream(
                &header(vec![]),
                &json!([{ "role": "user", "content": "hi" }]),
            )
            .await;
        assert!(result.is_err(), "悬挂服务端必须以 Err 终止: {result:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "读超时应秒级触发,实际 {:?}",
            started.elapsed()
        );
    }

    /// 非 2xx 状态经桥保留 → engine 分类器归类(401 → AUTH)
    #[tokio::test]
    async fn error_status_surfaced() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 8192];
            let _ = socket.read(&mut buf).await;
            let response =
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope";
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
        let mut transport = HttpTransport::new(ProviderConfig {
            base_url: format!("http://{addr}"),
            api_key: "bad".into(),
        })
        .unwrap();
        let err = transport
            .stream(
                &header(vec![]),
                &json!([{ "role": "user", "content": "hi" }]),
            )
            .await
            .expect_err("401 必须报错");
        assert_eq!(err.code(), "AUTH", "401 归类鉴权失败: {err:?}");
        assert!(err.to_string().contains("401"), "got: {err}");
    }
}
