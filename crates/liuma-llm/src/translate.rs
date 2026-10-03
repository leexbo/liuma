//! 内部消息方言(日志派生 Value)→ rig 核心 `Message`。
//!
//! 内部方言形状(与日志/不变式比对面一致,不随 provider 变化):
//! - user:content = string 或块数组(`{type:"text"}` / `{type:"image",
//!   attachment:{…}}`,图片字节经 [`AttachmentSource`] 组 data URL)
//! - assistant:`{content, tool_calls:[{id,name,arguments(JSON 串)}]}`
//! - tool:`{role:"tool", output, call, id, images?}`——**不携带工具名**,
//!   从同会话先前 assistant 的 tool_calls 按 id 反查(rig ToolResult 必填)
//!
//! 不支持图片的方言(如 deepseek 纯文本面)在此降级:图片块 →
//! [`OFFLOADED_IMAGE_TEXT`] 占位文本,不进 `input_image`。

use liuma_agent_loop::RequestHeader;
use rig_core::completion::message::{
    AdditionalParams, AssistantContent, CallId, DocumentSourceKind, Image, ImageMediaType, Message,
    Text, ToolFunction, ToolName, ToolResult, ToolResultContent, UserContent,
};
use serde_json::Value;

use crate::adapters::ProviderAdapter;

/// mediaType 串 → rig 枚举(无 FromStr;未知名回落 None)
fn media_type_of(m: &str) -> Option<ImageMediaType> {
    match m {
        "image/jpeg" | "image/jpg" => Some(ImageMediaType::JPEG),
        "image/png" => Some(ImageMediaType::PNG),
        "image/gif" => Some(ImageMediaType::GIF),
        "image/webp" => Some(ImageMediaType::WEBP),
        "image/heic" => Some(ImageMediaType::HEIC),
        "image/heif" => Some(ImageMediaType::HEIF),
        "image/svg+xml" => Some(ImageMediaType::SVG),
        _ => None,
    }
}

/// 工具名构造(空名回落 "unknown";内部方言 tool 名非空为既有不变式)
fn tool_name(name: &str) -> ToolName {
    ToolName::new(name)
        .unwrap_or_else(|_| ToolName::new("unknown").expect("守卫:unknown 非空,构造必为 Ok"))
}
use crate::attachments::{
    AttachmentSource, MAX_REQUEST_IMAGE_BYTES, OFFLOADED_IMAGE_TEXT, image_data_url,
    offload_request_images, project_files_to_text,
};

/// 图片块(image block)→ rig `Image`(data URL 源);字节缺席 → None(降级)
fn rig_image(block: &Value, source: &dyn AttachmentSource) -> Option<Image> {
    let url = image_data_url(block, source)?;
    let media_type = block["attachment"]["mediaType"]
        .as_str()
        .and_then(media_type_of);
    Some(Image {
        data: DocumentSourceKind::Url(url),
        media_type,
        detail: None,
        additional_params: None,
    })
}

/// user content(string 或块数组)→ rig 内容部件
fn user_parts(
    content: &Value,
    source: &dyn AttachmentSource,
    supports_images: bool,
) -> Vec<UserContent> {
    match content {
        // 纯文本消息保持紧凑 Text
        Value::String(s) => vec![UserContent::Text(Text::new(s))],
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| match b["type"].as_str() {
                Some("text") => b["text"].as_str().map(|t| UserContent::Text(Text::new(t))),
                Some("image") => {
                    if !supports_images {
                        // 方言无图片面:降级占位文本(请求可发,图片不丢线)
                        return Some(UserContent::Text(Text::new(OFFLOADED_IMAGE_TEXT)));
                    }
                    rig_image(b, source).map(UserContent::Image)
                }
                _ => None,
            })
            .collect(),
        _ => vec![UserContent::Text(Text::default())],
    }
}

/// 内部消息数组 → rig `Vec<Message>`(system 进列首;请求级文件投影与
/// 图片 offload 预处理与旧引擎同序)
pub(crate) fn to_rig_messages(
    header: &RequestHeader,
    messages: &Value,
    images: &dyn AttachmentSource,
    adapter: &dyn ProviderAdapter,
) -> Vec<Message> {
    // 请求级预处理(与旧引擎 wire_request 同链):文件投影 → 图片 offload
    let mut msgs = messages.clone();
    project_files_to_text(&mut msgs, images);
    offload_request_images(&mut msgs, MAX_REQUEST_IMAGE_BYTES);

    let supports_images = adapter.supports_images(&header.model);
    let call_names = crate::adapters::index_call_names(&msgs);

    let mut out = Vec::new();
    if !header.system.is_empty() {
        out.push(Message::System {
            content: header.system.clone(),
        });
    }
    for m in msgs.as_array().into_iter().flatten() {
        match m["role"].as_str() {
            Some("user") => out.push(Message::User {
                content: user_parts(&m["content"], images, supports_images),
            }),
            Some("assistant") => {
                let mut content = Vec::new();
                if let Some(text) = m["content"].as_str().filter(|s| !s.is_empty()) {
                    content.push(AssistantContent::Text(Text::new(text)));
                }
                // hosted 工具结果块回放(engine 收集进 server_tools 的
                // anthropic 原始块):经 rig Text 元数据通道重建,rig
                // 校验器恰允许 server_tool_use/web_search_tool_result/
                // code_execution_tool_result 三类。键为 rig pub(crate)
                // 稳定 wire 键,liuma 侧硬编码(值不匹配时 rig 回放侧
                // 自行报错,不静默)。responses 面无回放通道(rig
                // InputContent 缺 hosted 变体),仅 anthropic 面回放。
                if adapter.family() == crate::adapters::RigFamily::Anthropic {
                    for block in m["server_tools"].as_array().into_iter().flatten() {
                        if let Some(params) =
                            AdditionalParams::from_entries([("anthropic_content", block.clone())])
                        {
                            content.push(AssistantContent::Text(Text {
                                text: String::new(),
                                additional_params: Some(params),
                            }));
                        }
                    }
                }
                for call in m["tool_calls"].as_array().into_iter().flatten() {
                    let arguments = call["arguments"]
                        .as_str()
                        .and_then(|a| serde_json::from_str::<Value>(a).ok())
                        .unwrap_or_else(|| Value::String(String::new()));
                    let name = call["name"].as_str().unwrap_or_default();
                    content.push(AssistantContent::ToolCall(
                        rig_core::completion::message::ToolCall {
                            id: CallId::from_wire(call["id"].as_str().unwrap_or_default()),
                            function: ToolFunction {
                                name: tool_name(name),
                                arguments,
                            },
                            signature: None,
                            additional_params: None,
                        },
                    ));
                }
                out.push(Message::Assistant { id: None, content });
            }
            Some("tool") => {
                // 工具名反查(rig ToolResult 必填;内部 tool 消息不带名字)
                let call_id = m["id"].as_str().unwrap_or_default();
                let name = call_names
                    .get(call_id)
                    .cloned()
                    .unwrap_or_else(|| "unknown".into());
                // 空结果补占位:Anthropic API 拒收空 content 的 tool_result
                let output_text = {
                    let o = m["output"].as_str().unwrap_or_default();
                    if o.is_empty() { "(no output)" } else { o }
                };
                let mut content = vec![ToolResultContent::Text(Text::new(output_text))];
                for block in m["images"].as_array().into_iter().flatten() {
                    if supports_images && let Some(img) = rig_image(block, images) {
                        content.push(ToolResultContent::Image(img));
                    }
                }
                out.push(Message::User {
                    content: vec![UserContent::ToolResult(ToolResult {
                        call: CallId::from_wire(call_id),
                        name: tool_name(&name),
                        content,
                    })],
                });
            }
            _ => {}
        }
    }
    out
}
