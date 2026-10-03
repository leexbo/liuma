//! rig 引擎驱动:方言声明 + 消息翻译 → rig wire 请求 → 流事件映射回
//! `LlmEvent`。
//!
//! 事件映射约定(与旧三引擎的对齐面,spike 已证载荷级零漂移):
//! - 全家族:`Text` 增量 → Chunk、`Reasoning` 增量 → Reasoning;
//!   rig 的部件生命周期事件(`Start`/`End`/定稿部件重发)过滤——liuma
//!   事件模型无对应物;
//! - 终结(`finish`):usage 上报时先 Usage(五键归一)再定稿
//!   AssistantMessage(`{content, tool_calls}`,内部方言:扁平 calls),
//!   最后 Done;**chat 家言纯文本轮不发 AssistantMessage**(旧
//!   OpenAiChatMapper 约定);usage 未上报(provider 没发)不发 Usage 事件;
//! - TTFT:首个 Chunk/Reasoning 事件时刻,流尾附 `Usage({ttftMs})`——
//!   首 token 判定含推理(纯工具调用步也拿得到);
//! - 错误:rig `ProviderError` → `TransportError`(状态体走 liuma
//!   分类器:AUTH/CONTEXT_OVERFLOW/RATE_LIMIT/SERVER/INVALID_REQUEST;
//!   传输盒装错误原样拆回)。

use std::time::Instant;

use liuma_agent_loop::{LlmEvent, RequestHeader, TransportError};
use rig_core::completion::CompletionRequest;

use rig_core::error::ProviderError;
use rig_core::http_client::Error as HttpError;
use rig_core::providers::anthropic::wire::AnthropicConfig;
use rig_core::providers::openai::wire::OpenAIConfig;
use rig_core::streaming::{Item, StreamEvent};
use serde_json::Value;

use crate::adapters::{ProviderAdapter, RigFamily, partition_tools};
use crate::attachments::AttachmentSource;
use crate::bridge::Bridge;
use crate::http::ProviderConfig;
use crate::translate::to_rig_messages;
use crate::usage::canonical_from_rig;

/// rig `ProviderError` → liuma `TransportError`(状态体经方言精化钩子)
fn map_provider_error(adapter: &dyn ProviderAdapter, e: ProviderError) -> TransportError {
    match &e {
        ProviderError::Http(err) => map_http_error(adapter, err),
        ProviderError::ProviderResponse(r) | ProviderError::InvalidAuthentication(r) => {
            match r.status {
                Some(status) => adapter.refine_error(
                    crate::http::classify_status(
                        status,
                        r.headers
                            .as_ref()
                            .and_then(|h| h.get(reqwest::header::RETRY_AFTER))
                            .and_then(|v| v.to_str().ok())
                            .map(String::from),
                        r.body.clone(),
                    ),
                    &r.body,
                ),
                None => TransportError::Other(r.body.clone()),
            }
        }
        other => TransportError::Other(other.to_string()),
    }
}

/// rig 传输错误:非成功状态(体/头保留)走 liuma 分类器;Instance 里
/// 桥装入的 TransportError 原样拆回;其余 Other
fn map_http_error(adapter: &dyn ProviderAdapter, err: &HttpError) -> TransportError {
    if let Some(status) = err.non_success_status() {
        let retry_after = err
            .non_success_headers()
            .and_then(|h| h.get(reqwest::header::RETRY_AFTER))
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let body = err.non_success_body().unwrap_or_default().to_string();
        return adapter.refine_error(
            crate::http::classify_status(status, retry_after, body.clone()),
            &body,
        );
    }
    if let HttpError::Instance(boxed) = err
        && let Some(te) = boxed.downcast_ref::<TransportError>()
    {
        return te.clone();
    }
    TransportError::Other(err.to_string())
}

/// rig 驱动(宏单态化:Operation trait 为 rig crate 私有,泛型界定不可
/// 外达)。展开于 async 上下文;`sink` 逐事件下发,首内容事件记 TTFT。
macro_rules! rig_drive {
    ($model:expr, $request:expr, $adapter:expr, $sink:expr) => {{
        let started = Instant::now();
        let mut first_content_at: Option<std::time::Duration> = None;
        let chat_family = !matches!($adapter.family(), RigFamily::Anthropic)
            && !matches!($adapter.family(), RigFamily::OpenAi { responses: true, .. });

        let adapter_ref = $adapter;
        let mut stream = $model
            .stream($request)
            .map_err(|e| map_provider_error(adapter_ref, e))?;
        use futures::StreamExt as _;
        // hosted 工具的服务端载荷收集(responses 面的 web_search_call
        // 等未建模 output item 走 Item::Unknown;rig 语义「未建模必达
        // 消费者」——收集进定稿 AssistantMessage 的 server_tools,
        // 日志可见/可回放,不再静默丢弃)
        let mut server_tools: Vec<Value> = Vec::new();
        while let Some(item) = stream.next().await {
            match item {
                Ok(Item::Event(StreamEvent::Text { text, .. })) => {
                    if first_content_at.is_none() {
                        first_content_at = Some(started.elapsed());
                    }
                    $sink(LlmEvent::Chunk(text));
                }
                Ok(Item::Event(StreamEvent::Reasoning { text, .. })) => {
                    if first_content_at.is_none() {
                        first_content_at = Some(started.elapsed());
                    }
                    $sink(LlmEvent::Reasoning(text));
                }
                Ok(Item::Unknown(payload)) => {
                    server_tools.push(payload.value().clone());
                }
                // 部件生命周期(Start/End/定稿部件重发)过滤:定稿由
                // finish() 的 CompletionResponse 承担
                Ok(_) => {}
                Err(e) => return Err(map_provider_error(adapter_ref, e)),
            }
        }
        let response = stream
            .finish()
            .await
            .map_err(|e| map_provider_error(adapter_ref, e))?;
        // usage 上报时:先 Usage(五键归一)再定稿(chat 家族纯文本轮
        // 不发 AssistantMessage;anthropic/responses 恒发,与旧 mapper 一致)
        if response.usage.is_reported() {
            $sink(LlmEvent::Usage(canonical_from_rig(&response.usage)));
        }
        let mut content = String::new();
        let mut tool_calls = Vec::new();
        for part in &response.choice {
            match part {
                rig_core::completion::AssistantContent::Text(t) => {
                    // anthropic 面:hosted 块经 rig 折为空文本部件 +
                    // 元数据(键为 rig pub(crate) 稳定 wire 键,此处
                    // 硬编码取原始块)
                    if let Some(params) = &t.additional_params
                        && let Some(raw) = params.get("anthropic_content")
                    {
                        server_tools.push(raw.clone());
                        continue;
                    }
                    content.push_str(&t.text);
                }
                rig_core::completion::AssistantContent::ToolCall(c) => {
                    tool_calls.push(serde_json::json!({
                        "id": c.id.to_string(),
                        "name": c.function.name.to_string(),
                        "arguments": c.function.arguments.to_string(),
                    }));
                }
                _ => {}
            }
        }
        if !chat_family || !tool_calls.is_empty() {
            // server_tools 仅非空携带(内部方言可选键;translate 侧
            // anthropic 面回放,responses 面仅记录)
            let mut message = serde_json::json!({
                "content": content,
                "tool_calls": tool_calls,
            });
            if !server_tools.is_empty() {
                message["server_tools"] = Value::Array(server_tools);
            }
            $sink(LlmEvent::AssistantMessage(message));
        }
        $sink(LlmEvent::Done);
        if let Some(ttft) = first_content_at {
            $sink(LlmEvent::Usage(
                serde_json::json!({ "ttftMs": ttft.as_millis() as u64 }),
            ));
        }
        Ok(())
    }};
}

/// 一次流式补全:建桥 → 组 rig 请求 → 驱动 → 事件经 `sink` 下发。
/// `sink` 恒发(接收端关闭时丢弃由调用方处理)。`body_error` 由
/// [`crate::http::HttpTransport`] 构造时从自有 adapter 建立(自定义
/// 行为组随之生效;不再按方言名反查重建)。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn stream(
    adapter: &dyn ProviderAdapter,
    config: &ProviderConfig,
    client: reqwest::Client,
    header: &RequestHeader,
    messages: &Value,
    attachments: &dyn AttachmentSource,
    body_error: crate::bridge::BodyErrorHook,
    sink: &mut (dyn FnMut(LlmEvent) + Send),
) -> Result<(), TransportError> {
    let bridge = Bridge { client, body_error };

    let mut request = CompletionRequest::new("");
    request.chat_history = to_rig_messages(header, messages, attachments, adapter);
    request.temperature = Some(header.temperature);
    // 工具分区:function 条目进 rig 工具面;hosted 条目(如
    // {"type":"web_search",…})按方言声明译 wire 形态,经 rig
    // provider_tools 注入(anthropic/responses 两 wire 皆落
    // additional_params.tools 原样透传)。面/模型不提供 → fail-fast
    let (functions, hosted) = partition_tools(header);
    request.tools = functions;
    let mut provider_tools = Vec::new();
    if !hosted.is_empty() {
        for entry in &hosted {
            let kind = entry["type"].as_str().unwrap_or_default();
            let decl = adapter.hosted_tool(kind, &header.model).ok_or_else(|| {
                TransportError::Other(format!(
                    "方言 {} 的模型 {} 不提供 hosted 工具 {kind:?}",
                    adapter.name(),
                    header.model
                ))
            })?;
            // 条目除 type 外的字段原样透传(max_uses/allowed_domains…)
            let mut config_map = entry.as_object().cloned().unwrap_or_default();
            let _ = config_map.remove("type");
            if let Some(name) = decl.wire_name {
                config_map.insert("name".into(), Value::String(name.to_string()));
            }
            provider_tools.push(rig_core::completion::ProviderToolDefinition {
                kind: decl.wire_type.to_string(),
                config: config_map,
            });
        }
    }
    // effort params 先设,provider_tools 后合并(rig 的 provider_tools
    // 增量写入 additional_params.tools 不覆盖已有键;反过来会被整替)
    if let Some(effort) = &header.reasoning_effort
        && let Some(params) = adapter.effort_params(&header.model, effort)
    {
        request.additional_params = Some(params);
    }
    if !provider_tools.is_empty() {
        request = request.provider_tools(provider_tools);
    }

    match adapter.family() {
        RigFamily::OpenAi { dialect, responses } => {
            let mut cfg = OpenAIConfig::with_key(&dialect, &config.api_key);
            cfg.base_url = config.base_url.trim_end_matches('/').to_string();
            let provider = cfg.connect(bridge);
            if responses {
                rig_drive!(
                    provider.responses(header.model.clone()),
                    request,
                    adapter,
                    sink
                )
            } else {
                rig_drive!(provider.chat(header.model.clone()), request, adapter, sink)
            }
        }
        RigFamily::Anthropic => {
            request.max_tokens = Some(u64::from(adapter.max_tokens()));
            let mut cfg = AnthropicConfig::new(config.api_key.clone());
            cfg.base_url = config.base_url.trim_end_matches('/').to_string();
            let provider = cfg.connect(bridge);
            rig_drive!(
                provider.completion(header.model.clone()),
                request,
                adapter,
                sink
            )
        }
    }
}
