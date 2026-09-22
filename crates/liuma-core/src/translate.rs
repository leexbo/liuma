//! 事件翻译:我方 [`EventEnvelope`](liuma_session::EventEnvelope) → 客方
//! [`SessionEvent`](crate::proto::SessionEvent)。纯函数,无 IO——注册表
//! (直播)与 history(重放)共用同一表,直播/历史视图天然一致,客方
//! seq 缝合不受扰。
//!
//! 有状态部分仅 turn/step 计数:我方信封不携带编号,由翻译器扫描
//! `turn/start` / `step/start` 推导(客方 assistant 节点依赖
//! `step/start.data = {turn, step}` 起步,缺失则流式节点不出现)。

use liuma_session::EventEnvelope;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::proto::{SessionEvent, SurfaceOp};

/// 模型源信息(assistant message 的 source 块)
#[derive(Debug, Clone)]
pub struct ProviderInfo {
    /// provider 标识(dialect)
    pub provider: String,
    /// 模型标识
    pub model: String,
}

/// 有状态翻译器(turn/step 计数)
#[derive(Debug, Clone)]
pub struct Translator {
    provider: ProviderInfo,
    turn: u64,
    step: u64,
    /// 最近一次 llm request-done 的用量(时长/ttft/输出 tok;
    /// 附着到下一个 assistant/message → 消息流 turn footer 指标)
    last_usage: Option<Value>,
}

fn message_id() -> String {
    // v7:时间有序(与引擎/队列预分配 id 同方案,便于日志比对)
    Uuid::now_v7().to_string()
}

/// turn/error 原始错误串可能内嵌 provider JSON(如
/// `provider 400 Bad Request: {"error":{"message":…}}`)。投影只取
/// 人类可读 message:原始诊断留会话日志,不整串进 UI
fn provider_error_message(raw: &str) -> String {
    if let Some(pos) = raw.find('{')
        && let Ok(v) = serde_json::from_str::<Value>(&raw[pos..])
        && let Some(msg) = v["error"]["message"].as_str()
        && !msg.is_empty()
    {
        return msg.to_string();
    }
    raw.to_string()
}

/// 我方 arguments 字段:可能已是字符串(provider wire 透传)或 JSON 值;
/// 客方要求原始 JSON 字符串
fn arguments_as_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

impl Translator {
    /// 新翻译器(从会话起点开始)
    pub fn new(provider: ProviderInfo) -> Self {
        Self {
            provider,
            turn: 0,
            step: 0,
            last_usage: None,
        }
    }

    /// 窗口预热:仅推进 turn/step/last_usage 三个状态字段,不构造任何
    /// 输出 Value。[`translate_window`] 对 cut 之前的信封跑本方法——
    /// 状态机判据与 [`Self::translate`] 同款,保证窗口翻译输出与
    /// 「全量翻译」逐字节一致(last_usage 跨窗附着语义含在内:turn N
    /// 尾的 request-done 在全量翻译里同样会附着到下一轮首条
    /// assistant/message)。
    fn prime(&mut self, ev: &EventEnvelope) {
        match ev.r#type.as_str() {
            "turn/start" => {
                self.turn += 1;
                self.step = 0;
            }
            "step/start" => self.step += 1,
            "audit/call"
                if ev.data["boundary"].as_str() == Some("llm")
                    && ev.data["operation"].as_str() == Some("request-done") =>
            {
                let detail = &ev.data["detail"];
                let usage = &detail["usage"];
                self.last_usage = Some(json!({
                    "durationMs": detail["durationMs"].as_i64().unwrap_or(0),
                    "ttftMs": usage["ttftMs"].as_i64(),
                    "outputTokens": usage["output_tokens"].as_u64(),
                }));
            }
            _ => {}
        }
    }

    /// 单事件翻译;None = 按表丢弃(goal/audit/plan 落档等内部词汇)
    pub fn translate(&mut self, ev: &EventEnvelope) -> Option<SessionEvent> {
        let out = match ev.r#type.as_str() {
            "turn/start" => {
                self.turn += 1;
                self.step = 0;
                session_event(ev, json!({ "turn": self.turn }), None, None, None)
            }
            "turn/end" => {
                // 软取消的 turn/end 带 {cancelled:"token"} → 客方 aborted
                let cancelled = ev.data.get("cancelled").is_some();
                let reason = if cancelled {
                    json!({ "kind": "aborted", "reason": { "kind": "legacy" } })
                } else {
                    json!({ "kind": "completed" })
                };
                session_event(
                    ev,
                    json!({ "turn": self.turn, "reason": reason }),
                    None,
                    None,
                    None,
                )
            }
            // turn 异常终止(传输失败/悬挂超时)→ 客方 turn/end 的
            // error 终止形状(web turn-error 节点/重试链据此渲染)
            "turn/error" => {
                let raw = ev.data["error"].as_str().unwrap_or_default();
                // 稳定错误码(引擎 2026-09 起随 turn/error 落档;旧日志缺省 TRANSPORT)
                let code = ev.data["code"].as_str().unwrap_or("TRANSPORT");
                let mut out = session_event(
                    ev,
                    json!({
                        "turn": self.turn,
                        "reason": {
                            "kind": "error",
                            "error": {
                                "code": code,
                                "message": provider_error_message(raw),
                            },
                        },
                    }),
                    None,
                    None,
                    None,
                );
                out.ty = "turn/end".into();
                out
            }
            "step/start" => {
                self.step += 1;
                session_event(
                    ev,
                    json!({ "turn": self.turn, "step": self.step }),
                    None,
                    None,
                    None,
                )
            }
            "step/end" => session_event(
                ev,
                json!({ "turn": self.turn, "step": self.step }),
                None,
                None,
                None,
            ),
            "user/message" => {
                // 纯文本日志 = 顶层字符串 → 包装 text 块;图消息日志已是
                // 块数组(图前文后)→ 原样透传(客户端按块类型渲染)
                let content = match &ev.data["content"] {
                    Value::Array(blocks) => Value::Array(blocks.clone()),
                    other => {
                        json!([ { "type": "text", "text": other.as_str().unwrap_or_default() } ])
                    }
                };
                // 持久 id(引擎/宿主预分配 v7;旧日志无 id 时回退合成)
                let id = ev.data["id"]
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(message_id);
                // source 分流:kind=user → 真实用户消息;否则保留
                // 注入染色的 source(旁车元数据,含 kind/form——客户端据它
                // 渲染「注入行」)。缺失 source 默认按真实用户处理。
                let source = match ev.data.get("source") {
                    Some(s) if !s.is_null() => s.clone(),
                    _ => json!({ "kind": "user" }),
                };
                session_event(
                    ev,
                    json!({
                        "id": id,
                        "role": "user",
                        "content": content,
                        "source": source,
                    }),
                    Some(vec![ev.seq]),
                    Some(SurfaceOp::Append),
                    None,
                )
            }
            // 队列 splice(steer 认领/队列出队):透传,inserted 条目重塑为
            // 客方 Message 形状(UI 只读 id;保留内容供未来队列重建)
            "agent/inbox/spliced" => {
                let target = ev.data["target"].as_str().unwrap_or_default();
                let inserted = ev.data["inserted"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .map(|m| {
                                let id = m["id"].as_str().unwrap_or_default();
                                let text = m["content"].as_str().unwrap_or_default();
                                json!({
                                    "id": id,
                                    "role": "user",
                                    "content": [ { "type": "text", "text": text } ],
                                    "source": { "kind": "user" },
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                session_event(
                    ev,
                    json!({
                        "target": target,
                        "start": ev.data["start"].as_u64().unwrap_or(0),
                        "removedCount": ev.data["removedCount"].as_u64().unwrap_or(0),
                        "inserted": inserted,
                    }),
                    None,
                    None,
                    None,
                )
            }
            "assistant/chunk" => {
                let delta = ev.data["delta"].as_str().unwrap_or_default();
                session_event(
                    ev,
                    json!({
                        "turn": self.turn, "step": self.step,
                        "chunk": { "type": "text-delta", "index": 0, "text": delta },
                    }),
                    None,
                    None,
                    Some(true),
                )
            }
            // 思考内容(Think 行数据源;UI 消费,客方未知类型忽略)
            "assistant/reasoning" => {
                let text = ev.data["text"].as_str().unwrap_or_default();
                session_event(
                    ev,
                    json!({ "turn": self.turn, "step": self.step, "text": text }),
                    None,
                    None,
                    Some(true),
                )
            }
            // 流式残段丢弃标记(重试前):投影据此清空该步的流式缓冲,
            // 与引擎的累积重置同序(重放与实况一致)
            "assistant/stream-reset" => session_event(
                ev,
                json!({ "turn": self.turn, "step": self.step }),
                None,
                None,
                Some(true),
            ),
            // LLM 请求重试(llm-retry):消息流折叠行数据源。
            // 载荷原样透传并附 turn/step 上下文(桌面轮分组定位)
            "llm/retry" | "llm/retry-started" => {
                let mut data = json!({ "turn": self.turn, "step": self.step });
                if let Some(obj) = ev.data.as_object() {
                    for (k, v) in obj {
                        data[k.as_str()] = v.clone();
                    }
                }
                session_event(ev, data, None, None, None)
            }
            "assistant/message" => {
                let text = ev.data["content"].as_str().unwrap_or_default();
                let mut content = Vec::new();
                if !text.is_empty() {
                    content.push(json!({ "type": "text", "text": text }));
                }
                if let Some(calls) = ev.data["tool_calls"].as_array() {
                    for call in calls {
                        content.push(json!({
                            "type": "tool-call",
                            "id": call["id"].as_str().unwrap_or_default(),
                            "name": call["name"].as_str().unwrap_or_default(),
                            "arguments": arguments_as_string(&call["arguments"]),
                        }));
                    }
                }
                // 附着最近请求的用量(时长/ttft/输出 tok;消息流 turn footer 指标)
                // 客方形状:message 包裹 + turn/step 平铺(assistant 节点读 data.message.content)
                let usage = self.last_usage.take();
                let mut data = json!({
                    "turn": self.turn, "step": self.step,
                    "message": {
                        "id": ev.data["id"].as_str().unwrap_or_default(),
                        "role": "assistant",
                        "content": content,
                        "source": {
                            "kind": "model",
                            "provider": self.provider.provider,
                            "model": self.provider.model,
                        },
                    },
                });
                if let Some(u) = usage {
                    data["usage"] = u;
                }
                session_event(ev, data, Some(vec![ev.seq]), Some(SurfaceOp::Append), None)
            }
            "tool/call" => {
                let mut data = json!({
                    "turn": self.turn, "step": self.step,
                    "callId": ev.seq.to_string(),
                    "name": ev.data["name"].as_str().unwrap_or_default(),
                    "arguments": arguments_as_string(&ev.data["arguments"]),
                });
                // call 侧渲染意图透传(运行中意图,如 file_edit 的 diff)
                if let Some(v) = ev.data.get("view").filter(|v| !v.is_null()) {
                    data["view"] = v.clone();
                }
                session_event(ev, data, None, None, None)
            }
            "tool/result" => {
                let call_id = ev.data["call"].as_u64().unwrap_or_default().to_string();
                let output = ev.data["output"].as_str().unwrap_or_default();
                let success = ev.data["success"].as_bool().unwrap_or(true);
                let mut data = json!({
                    "turn": self.turn, "step": self.step,
                    "message": {
                        "id": message_id(),
                        "role": "user",
                        "content": [ {
                            "type": "tool-result",
                            "toolCallId": call_id,
                            "content": [ { "type": "text", "text": output } ],
                            "isError": !success,
                        } ],
                        "source": { "kind": "tool", "callId": call_id },
                    },
                });
                if !success {
                    data["error"] = json!({ "name": "ToolError", "code": "tool-failed" });
                }
                // 渲染意图对象整体透传(终端退出详情/读窗口/搜索分组/diff;
                // 在场才有——消费侧窄化失败回落通用卡)
                if let Some(v) = ev.data.get("view").filter(|v| !v.is_null()) {
                    data["view"] = v.clone();
                }
                // 结果图片(MCP 图片桥;持久引用数组,在场才有)
                if let Some(imgs) = ev
                    .data
                    .get("images")
                    .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
                {
                    data["images"] = imgs.clone();
                }
                session_event(ev, data, Some(vec![ev.seq]), Some(SurfaceOp::Append), None)
            }
            // todo/write:整表快照已与客方同形({todos:[{content,status}]})——透传
            "todo/write" => session_event(ev, ev.data.clone(), None, None, None),
            // 计划归档四件(提交/批准/取消/拒绝)——原样透传,桌面投影成
            // 聊天流计划卡(状态随 approved/cancelled/declined 更新)
            "plan/submitted" | "plan/approved" | "plan/cancelled" | "plan/declined" => {
                session_event(ev, ev.data.clone(), None, None, None)
            }
            // 压缩结果对(summary 落档成功 / error 手动压缩失败)——原样
            // 透传,桌面投影成「已压缩」标记行(可展开摘要)或失败通告
            "compaction/summary" | "compaction/error" => {
                session_event(ev, ev.data.clone(), None, None, None)
            }
            "session/mode" => {
                let active = ev.data["mode"].as_str() == Some("plan");
                let mut out = session_event(ev, json!({ "active": active }), None, None, None);
                out.ty = "plan/mode".into();
                out
            }
            // llm 请求完成:不翻译成事件,但记住用量(时长/ttft/输出 tok)
            // 供下一个 assistant/message 附着(消息流 turn footer 指标)
            "audit/call"
                if ev.data["boundary"].as_str() == Some("llm")
                    && ev.data["operation"].as_str() == Some("request-done") =>
            {
                let detail = &ev.data["detail"];
                let usage = &detail["usage"];
                self.last_usage = Some(json!({
                    "durationMs": detail["durationMs"].as_i64().unwrap_or(0),
                    "ttftMs": usage["ttftMs"].as_i64(),
                    "outputTokens": usage["output_tokens"].as_u64(),
                }));
                return None;
            }
            // 内部词汇(goal/audit/compaction/plan 落档/会话元数据):丢弃。
            // 直播与 history 走同一表 → 丢弃一致,seq 缝合不受扰。
            _ => return None,
        };
        Some(out)
    }
}

/// 信封组装(seq/time 保留;类型改名场景由调用方覆写 ty)
fn session_event(
    ev: &EventEnvelope,
    data: Value,
    source_event_seqs: Option<Vec<u64>>,
    surface_op: Option<SurfaceOp>,
    ignorable: Option<bool>,
) -> SessionEvent {
    SessionEvent {
        ty: ev.r#type.clone(),
        seq: ev.seq,
        time: ev.time,
        data,
        source_event_seqs,
        surface_op,
        ignorable,
    }
}

/// 整段日志翻译(history / 冷会话重放)
pub fn translate_events<'a>(
    provider: &ProviderInfo,
    events: impl IntoIterator<Item = &'a EventEnvelope>,
) -> Vec<SessionEvent> {
    let mut tr = Translator::new(provider.clone());
    events
        .into_iter()
        .filter_map(|ev| tr.translate(ev))
        .collect()
}

/// 尾窗边界定位(信封层,零克隆零 Value 解析)。判据与 [`paginate`]
/// 的客方 surface 语义对齐:user/message(排除注入行 source.kind !=
/// "user")与 assistant/message 计入;translated 的 source_event_seqs
/// 恒为合成值 `vec![ev.seq]`,故边界即消息信封自身的 seq(勿用信封
/// 持久字段)。从 before_seq 窗口上界向尾数到第 max_messages 条消息。
pub fn page_cut(events: &[EventEnvelope], before_seq: Option<u64>, max_messages: usize) -> u64 {
    let mut count = 0usize;
    for ev in events.iter().rev() {
        if before_seq.is_some_and(|b| ev.seq >= b) {
            continue;
        }
        let surface = match ev.r#type.as_str() {
            "assistant/message" => true,
            "user/message" => {
                !(ev.data["source"].is_object() && ev.data["source"]["kind"] != "user")
            }
            _ => false,
        };
        if surface {
            count += 1;
            if count >= max_messages.max(1) {
                return ev.seq;
            }
        }
    }
    0
}

/// 尾窗翻译:cut 之前仅 [`Translator::prime`] 预热(零输出构造),
/// (cut, before_seq) 窗口内的事件与全量翻译同表逐条翻译。输出与
/// 「[`translate_events`] 全量 + [`paginate`] 截窗」逐字节一致(差分
/// 回归锁),翻译分配从 O(全量) 收敛到 O(窗口)。事件序保证 seq 升序,
/// 越过 before_seq 上界即整段跳出。
pub fn translate_window<'a>(
    provider: &ProviderInfo,
    events: impl IntoIterator<Item = &'a EventEnvelope>,
    cut: u64,
    before_seq: Option<u64>,
) -> Vec<SessionEvent> {
    let mut tr = Translator::new(provider.clone());
    let mut out = Vec::new();
    for ev in events {
        if before_seq.is_some_and(|b| ev.seq >= b) {
            break;
        }
        if ev.seq > cut {
            if let Some(e) = tr.translate(ev) {
                out.push(e);
            }
        } else {
            tr.prime(ev);
        }
    }
    out
}

// ── history 分页(api-proxy paginate 语义) ───────────────────────────

/// 分页结果
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    /// 本页事件(seq 升序)
    pub events: Vec<SessionEvent>,
    /// 是否还有更早页
    pub has_more: bool,
    /// 截断 seq(0 = 到头)
    pub cut: u64,
}

/// 客方 history 分页:从尾向前按 surface 消息(user/message、
/// assistant/message 且 surfaceOp=append)计数,数满 max_messages 的
/// 那条消息处取 `min(seq, min(sourceEventSeqs))` 为 cut;返回
/// seq > cut 的事件,hasMore = cut > 0。beforeSeq 为上一页边界
/// (loadOlder):窗口上界。
pub fn paginate(events: &[SessionEvent], before_seq: Option<u64>, max_messages: usize) -> Page {
    let window: Vec<&SessionEvent> = events
        .iter()
        .filter(|e| before_seq.is_none_or(|b| e.seq < b))
        .collect();

    let is_surface = |e: &&SessionEvent| {
        matches!(e.ty.as_str(), "user/message" | "assistant/message")
            && matches!(e.surface_op, Some(SurfaceOp::Append))
            // 注入行(user/message + source 在场且 kind != "user")不计入 surface
            // 用户消息:注入是旁车上下文,不占 history 条数。source
            // 缺失(真实用户消息缺省)按用户处理。
            && !(e.ty == "user/message"
                && e.data["source"].is_object()
                && e.data["source"]["kind"] != "user")
    };

    let mut count = 0usize;
    let mut cut = 0u64;
    for e in window.iter().rev() {
        if is_surface(e) {
            count += 1;
            if count >= max_messages {
                let mut boundary = e.seq;
                if let Some(seqs) = &e.source_event_seqs {
                    boundary = boundary.min(seqs.iter().copied().min().unwrap_or(boundary));
                }
                cut = boundary;
                break;
            }
        }
    }

    let page: Vec<SessionEvent> = window
        .into_iter()
        .filter(|e| e.seq > cut)
        .cloned()
        .collect();
    Page {
        events: page,
        has_more: cut > 0,
        cut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(ty: &str, seq: u64, data: Value) -> EventEnvelope {
        EventEnvelope {
            r#type: ty.into(),
            seq,
            time: 0,
            data,
            surface_op: None,
            source_event_seqs: None,
            ignorable: false,
        }
    }

    fn info() -> ProviderInfo {
        ProviderInfo {
            provider: "openai-completions".into(),
            model: "deepseek-chat".into(),
        }
    }

    /// 完整 turn:计数、surface 形状、callId 关联逐项锁定
    #[test]
    fn full_turn_translation() {
        let mut tr = Translator::new(info());
        let events = vec![
            ev("turn/start", 1, json!({})),
            ev("user/message", 2, json!({ "content": "hi" })),
            ev("step/start", 3, json!({})),
            ev("assistant/chunk", 4, json!({ "delta": "he" })),
            ev(
                "assistant/message",
                5,
                json!({
                    "content": "",
                    "tool_calls": [ { "id": "call_00_x", "name": "bash",
                        "arguments": { "command": "ls" } } ],
                }),
            ),
            ev(
                "tool/call",
                6,
                json!({ "name": "bash", "arguments": { "command": "ls" } }),
            ),
            ev(
                "tool/result",
                7,
                json!({ "call": 6, "output": "a.txt", "success": true }),
            ),
            ev("assistant/chunk", 8, json!({ "delta": "done" })),
            ev("assistant/message", 9, json!({ "content": "done" })),
            ev("step/end", 10, json!({})),
            ev("turn/end", 11, json!({})),
        ];

        let out: Vec<SessionEvent> = events.iter().filter_map(|e| tr.translate(e)).collect();
        assert_eq!(out.len(), events.len());

        // turn/step 计数
        assert_eq!(out[0].data, json!({ "turn": 1 }));
        assert_eq!(out[2].data, json!({ "turn": 1, "step": 1 }));
        assert_eq!(
            out[3].data["chunk"],
            json!({ "type": "text-delta", "index": 0, "text": "he" })
        );
        assert_eq!(out[3].ignorable, Some(true));

        // user surface:完整 Message 块 + append + 引用链
        assert_eq!(out[1].surface_op, Some(SurfaceOp::Append));
        assert_eq!(out[1].source_event_seqs, Some(vec![2]));
        assert_eq!(out[1].data["role"], "user");
        assert_eq!(out[1].data["content"][0]["type"], "text");
        assert_eq!(out[1].surface_op, Some(SurfaceOp::Append));
        assert_eq!(out[1].data["source"]["kind"], "user");

        // assistant message:text + tool-call 块(arguments 转字符串)
        let amsg = &out[4];
        assert_eq!(amsg.surface_op, Some(SurfaceOp::Append));
        let tc = &amsg.data["message"]["content"][0];
        assert_eq!(tc["type"], "tool-call");
        assert_eq!(tc["id"], "call_00_x");
        assert_eq!(tc["arguments"], json!("{\"command\":\"ls\"}"));
        assert_eq!(amsg.data["message"]["source"]["kind"], "model");
        assert_eq!(amsg.data["message"]["source"]["model"], "deepseek-chat");

        // tool/call:callId = seq 字符串
        assert_eq!(out[5].data["callId"], "6");
        assert_eq!(out[5].data["name"], "bash");
        // tool/result:toolCallId 关联 + surface
        assert_eq!(out[6].data["message"]["content"][0]["toolCallId"], "6");
        assert_eq!(out[6].data["message"]["source"]["callId"], "6");
        assert_eq!(out[6].surface_op, Some(SurfaceOp::Append));
        assert!(out[6].data.get("error").is_none());

        // turn/end completed
        assert_eq!(out[10].data["reason"], json!({ "kind": "completed" }));
    }

    /// 注入上下文(4a 完整溯源模型):user/message + source.kind≠user,
    /// role:user + source 原样透传(kind/form),区别于真实用户消息的 `kind = "user"`。
    #[test]
    fn user_message_injection_passes_source_through() {
        let mut tr = Translator::new(info());
        let out: Vec<SessionEvent> = [ev(
            "user/message",
            1,
            json!({
                "content": "AGENTS.md 全文",
                "id": "ctx-1",
                "source": { "kind": "agent-instructions", "form": "instructions" },
            }),
        )]
        .iter()
        .filter_map(|e| tr.translate(e))
        .collect();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].data["role"], "user");
        assert_eq!(out[0].data["content"][0]["type"], "text");
        assert_eq!(out[0].data["content"][0]["text"], "AGENTS.md 全文");
        assert_eq!(out[0].data["source"]["kind"], "agent-instructions");
        assert_eq!(out[0].data["source"]["form"], "instructions");
        assert_eq!(out[0].surface_op, Some(SurfaceOp::Append));
        assert_eq!(out[0].source_event_seqs, Some(vec![1]));
    }

    #[test]
    fn cancelled_turn_maps_to_aborted() {
        let mut tr = Translator::new(info());
        tr.translate(&ev("turn/start", 1, json!({})));
        let end = tr
            .translate(&ev("turn/end", 2, json!({ "cancelled": "token" })))
            .unwrap();
        assert_eq!(end.data["reason"]["kind"], "aborted");
    }

    /// durable turn/error → 客方 turn/end 的 error 终止形状
    /// (web turn-error 节点/重试链据此渲染)
    #[test]
    fn turn_error_maps_to_client_turn_end_error() {
        let mut tr = Translator::new(info());
        tr.translate(&ev("turn/start", 1, json!({})));
        let out = tr
            .translate(&ev(
                "turn/error",
                2,
                json!({ "error": "transport: 读超时" }),
            ))
            .unwrap();
        assert_eq!(out.ty, "turn/end");
        assert_eq!(out.data["turn"], 1);
        assert_eq!(out.data["reason"]["kind"], "error");
        assert_eq!(out.data["reason"]["error"]["code"], "TRANSPORT");
        assert_eq!(out.data["reason"]["error"]["message"], "transport: 读超时");
    }

    /// provider 400 的内嵌 JSON 错误:投影提取人类可读 message
    #[test]
    fn provider_error_message_extracts_embedded_json() {
        let raw = "provider 400 Bad Request: {\"error\":{\"message\":\
                   \"An assistant message with 'tool_calls' must be followed by tool messages\",\
                   \"type\":\"invalid_request_error\",\"code\":\"invalid_request_error\"}}";
        assert_eq!(
            provider_error_message(raw),
            "An assistant message with 'tool_calls' must be followed by tool messages"
        );
        // 无内嵌 JSON:原样
        assert_eq!(provider_error_message("读超时"), "读超时");
    }

    /// LLM 重试三件:llm/retry / llm/retry-started 原样透传(载 turn/step
    /// 上下文),assistant/stream-reset 以 ignorable 下发(投影清流式残段)
    #[test]
    fn retry_events_translate() {
        let mut tr = Translator::new(info());
        tr.translate(&ev("turn/start", 1, json!({})));
        tr.translate(&ev("step/start", 2, json!({})));
        let retry = tr
            .translate(&ev(
                "llm/retry",
                3,
                json!({
                    "retry": 1, "maxRetries": 5, "delayMs": 500,
                    "code": "TRANSPORT", "message": "连接失败",
                }),
            ))
            .expect("llm/retry");
        assert_eq!(retry.ty, "llm/retry");
        assert_eq!(retry.data["retry"], 1);
        assert_eq!(retry.data["maxRetries"], 5);
        assert_eq!(retry.data["delayMs"], 500);
        assert_eq!(retry.data["code"], "TRANSPORT");
        assert_eq!(retry.data["turn"], 1);
        assert_eq!(retry.data["step"], 1);

        let started = tr
            .translate(&ev("llm/retry-started", 4, json!({ "retry": 1 })))
            .expect("llm/retry-started");
        assert_eq!(started.data["retry"], 1);
        assert_eq!(started.data["turn"], 1);

        let reset = tr
            .translate(&ev("assistant/stream-reset", 5, json!({})))
            .expect("stream-reset");
        assert_eq!(reset.ignorable, Some(true));
        assert_eq!(reset.data["turn"], 1);
        assert_eq!(reset.data["step"], 1);
    }

    /// turn/error 携带稳定错误码(引擎落档;缺席 = 旧日志回落 TRANSPORT)
    #[test]
    fn turn_error_carries_code() {
        let mut tr = Translator::new(info());
        tr.translate(&ev("turn/start", 1, json!({})));
        let with_code = tr
            .translate(&ev(
                "turn/error",
                2,
                json!({ "error": "provider 401: nope", "code": "AUTH" }),
            ))
            .unwrap();
        assert_eq!(with_code.data["reason"]["error"]["code"], "AUTH");

        let legacy = Translator::new(info());
        let mut legacy = legacy;
        legacy.translate(&ev("turn/start", 1, json!({})));
        let out = legacy
            .translate(&ev("turn/error", 2, json!({ "error": "读超时" })))
            .unwrap();
        assert_eq!(out.data["reason"]["error"]["code"], "TRANSPORT");
    }

    #[test]
    fn failure_tool_result_carries_error() {
        let mut tr = Translator::new(info());
        tr.translate(&ev("turn/start", 1, json!({})));
        tr.translate(&ev("step/start", 2, json!({})));
        let r = tr
            .translate(&ev(
                "tool/result",
                3,
                json!({ "call": 2, "output": "boom", "success": false }),
            ))
            .unwrap();
        assert_eq!(r.data["message"]["content"][0]["isError"], true);
        assert_eq!(r.data["error"]["code"], "tool-failed");
    }

    /// 渲染意图透传:tool/call 与 tool/result 的 view 对象在场原样、
    /// 缺席零噪音(bash 终端详情经 view.terminal 携带)
    #[test]
    fn tool_view_passes_through_call_and_result() {
        let mut tr = Translator::new(info());
        tr.translate(&ev("turn/start", 1, json!({})));
        tr.translate(&ev("step/start", 2, json!({})));
        let call = tr
            .translate(&ev(
                "tool/call",
                3,
                json!({
                    "name": "file_edit",
                    "arguments": { "path": "a.txt" },
                    "view": { "card": "diff", "diffs": [ { "path": "a.txt",
                        "oldText": "o", "newText": "n" } ] },
                }),
            ))
            .unwrap();
        assert_eq!(call.data["view"]["card"], "diff");
        assert_eq!(call.data["view"]["diffs"][0]["newText"], "n");

        let r = tr
            .translate(&ev(
                "tool/result",
                4,
                json!({
                    "call": 3, "output": "out", "success": true,
                    "view": { "card": "terminal", "exitCode": 1,
                        "signal": null, "cwd": "/tmp/ws" },
                }),
            ))
            .unwrap();
        assert_eq!(r.data["view"]["card"], "terminal");
        assert_eq!(r.data["view"]["exitCode"], 1);
        assert_eq!(r.data["view"]["cwd"], "/tmp/ws");

        // 无视图的普通工具:两事件均无 view 键
        let plain = tr
            .translate(&ev(
                "tool/result",
                5,
                json!({ "call": 4, "output": "ok", "success": true }),
            ))
            .unwrap();
        assert!(plain.data.get("view").is_none());
    }

    #[test]
    fn todo_and_mode_rename() {
        let mut tr = Translator::new(info());
        // todo/write 与客方同形:透传(seq/时间保留,载荷原样)
        let todo = tr
            .translate(&ev(
                "todo/write",
                1,
                json!({ "todos": [
                { "content": "a", "status": "completed" },
                { "content": "b", "status": "in_progress" },
            ] }),
            ))
            .unwrap();
        assert_eq!(todo.ty, "todo/write");
        assert_eq!(todo.data["todos"][0]["content"], "a");
        assert_eq!(todo.data["todos"][1]["status"], "in_progress");

        let mode = tr
            .translate(&ev("session/mode", 2, json!({ "mode": "plan" })))
            .unwrap();
        assert_eq!(mode.ty, "plan/mode");
        assert_eq!(mode.data, json!({ "active": true }));
    }

    #[test]
    fn internal_vocabulary_dropped() {
        let mut tr = Translator::new(info());
        for (ty, data) in [
            ("goal/state", json!({ "goals": [] })),
            ("goal/state", json!({ "goals": [] })),
            ("audit/call", json!({ "boundary": "llm" })),
        ] {
            assert!(tr.translate(&ev(ty, 1, data)).is_none(), "{ty} 应丢弃");
        }
    }

    #[test]
    fn compaction_events_pass_through() {
        // 压缩结果对进桌面(summary 标记行 / error 通告)
        let mut tr = Translator::new(info());
        let s = tr
            .translate(&ev(
                "compaction/summary",
                1,
                json!({ "summary": "s", "throughSeq": 1 }),
            ))
            .expect("summary 透传");
        assert_eq!(s.ty, "compaction/summary");
        assert_eq!(s.data["summary"], "s");
        let e = tr
            .translate(&ev("compaction/error", 2, json!({ "message": "m" })))
            .expect("error 透传");
        assert_eq!(e.ty, "compaction/error");
    }

    /// 多 turn 计数器推进
    #[test]
    fn counters_advance_across_turns() {
        let mut tr = Translator::new(info());
        let seq_events = [
            ("turn/start", json!({})),
            ("step/start", json!({})),
            ("assistant/message", json!({ "content": "a" })),
            ("turn/end", json!({})),
            ("turn/start", json!({})),
            ("step/start", json!({})),
            ("step/end", json!({})),
            ("step/start", json!({})),
            ("assistant/message", json!({ "content": "b" })),
            ("turn/end", json!({})),
        ];
        let out: Vec<SessionEvent> = seq_events
            .iter()
            .enumerate()
            .filter_map(|(i, (ty, data))| tr.translate(&ev(ty, (i + 1) as u64, data.clone())))
            .collect();
        assert_eq!(out[5].data, json!({ "turn": 2, "step": 1 }));
        assert_eq!(out[7].data, json!({ "turn": 2, "step": 2 }));
        assert_eq!(out[8].data["message"]["source"]["model"], "deepseek-chat");
    }

    // ── paginate ─────────────────────────────────────────────────────

    fn surface_msg(seq: u64, ty: &str) -> SessionEvent {
        SessionEvent {
            ty: ty.into(),
            seq,
            time: 0,
            data: json!({}),
            source_event_seqs: Some(vec![seq]),
            surface_op: Some(SurfaceOp::Append),
            ignorable: None,
        }
    }

    fn filler(seq: u64) -> SessionEvent {
        SessionEvent {
            ty: "step/start".into(),
            seq,
            time: 0,
            data: json!({}),
            source_event_seqs: None,
            surface_op: None,
            ignorable: None,
        }
    }

    #[test]
    fn paginate_tail_and_older_pages() {
        // 构造 5 条 surface 消息(seq 1..=5,交替 user/assistant)+ 填充
        let mut events: Vec<SessionEvent> = Vec::new();
        let mut seq = 0u64;
        for i in 1..=5u64 {
            seq += 1;
            events.push(filler(seq)); // step/start
            seq += 1;
            events.push(surface_msg(
                seq,
                if i % 2 == 1 {
                    "user/message"
                } else {
                    "assistant/message"
                },
            ));
        }
        assert_eq!(events.len(), 10);

        // 尾页 max=2:从尾数 2 条 surface(seq 10、8)→ cut = 8
        let tail = paginate(&events, None, 2);
        assert_eq!(tail.cut, 8);
        assert!(tail.has_more);
        assert_eq!(tail.events.first().unwrap().seq, 9); // seq > 8
        assert_eq!(tail.events.len(), 2);

        // loadOlder:beforeSeq = 9 → 窗口 seq < 9(最高 surface 为 8),
        // 从尾数 2 条(8、6)→ cut = 6
        let older = paginate(&events, Some(9), 2);
        assert_eq!(older.cut, 6);
        assert!(older.has_more);
        assert_eq!(older.events.first().unwrap().seq, 7);

        // 到头页:cut = 0,hasMore = false
        let head = paginate(&events, Some(5), 10);
        assert_eq!(head.cut, 0);
        assert!(!head.has_more);
        assert_eq!(head.events.len(), 4); // seq 1..4
    }

    #[test]
    fn paginate_cut_uses_min_source_seq() {
        // sourceEventSeqs 引用更早 seq 时,cut 取 min(截掉派生链)
        let mut m = surface_msg(10, "assistant/message");
        m.source_event_seqs = Some(vec![7, 9]);
        let events = vec![filler(6), filler(7), filler(8), filler(9), m];
        let page = paginate(&events, None, 1);
        assert_eq!(page.cut, 7);
        assert_eq!(page.events.first().unwrap().seq, 8);
    }

    /// 翻译 + 分页联测:历史视图与直播同表
    #[test]
    fn translate_then_paginate() {
        let provider = info();
        let events: Vec<EventEnvelope> = vec![
            ev("turn/start", 1, json!({})),
            ev("user/message", 2, json!({ "content": "q1" })),
            ev("step/start", 3, json!({})),
            ev("assistant/message", 4, json!({ "content": "a1" })),
            ev("turn/end", 5, json!({})),
            ev("turn/start", 6, json!({})),
            ev("user/message", 7, json!({ "content": "q2" })),
            ev("step/start", 8, json!({})),
            ev("assistant/message", 9, json!({ "content": "a2" })),
            ev("turn/end", 10, json!({})),
        ];
        let translated = translate_events(&provider, &events);
        assert_eq!(translated.len(), events.len());

        let page = paginate(&translated, None, 3);
        // 3 条 surface(a2、q2、a1)数满于 a1(seq 4)→ cut = 4
        assert_eq!(page.cut, 4);
        assert!(page.has_more);
        assert_eq!(page.events[2].ty, "user/message");
        assert_eq!(page.events[2].data["content"][0]["text"], "q2");
    }

    // ── 窗口翻译(page_cut + translate_window 差分回归锁) ────────────

    /// 确定性伪随机生成多 turn 日志:含注入行、audit request-done、
    /// tool 链、软取消。老路径(全量翻译 + paginate)与新路径
    /// (page_cut + translate_window)输出必须逐字节一致,且 cut 同源。
    /// 锁死窗口判据三陷阱:合成 source_event_seqs(= 信封 seq,非持久
    /// 字段)、注入行不计数、last_usage 跨窗附着。
    fn synthetic_log() -> Vec<EventEnvelope> {
        // LCG 确定性伪随机(不依赖系统随机,Wasm 同规)
        let mut seed = 0x5EED_u64;
        let mut rand = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % 100
        };
        let mut events = Vec::new();
        let mut seq = 0u64;
        for turn in 1..=12u64 {
            seq += 1;
            events.push(ev("turn/start", seq, json!({ "turn": turn })));
            for step in 1..=3u64 {
                seq += 1;
                events.push(ev("step/start", seq, json!({ "turn": turn, "step": step })));
                // 注入上下文行(不计 surface;约 1/3 步带)
                if rand() < 33 {
                    seq += 1;
                    events.push(ev(
                        "user/message",
                        seq,
                        json!({
                            "content": [ { "type": "text", "text": "注入上下文" } ],
                            "source": { "kind": "plugin", "plugin": "liuma/system-prompt" },
                        }),
                    ));
                }
                // 真实用户消息(块数组形态;约每步一条)
                if step == 1 {
                    seq += 1;
                    events.push(ev(
                        "user/message",
                        seq,
                        json!({ "id": format!("u{turn}"), "role": "user",
                            "content": [ { "type": "text", "text": format!("问题{turn}") } ] }),
                    ));
                }
                seq += 1;
                events.push(ev(
                    "assistant/chunk",
                    seq,
                    json!({ "turn": turn, "step": step, "delta": "回答片段" }),
                ));
                // audit request-done(usage 附着源)
                seq += 1;
                events.push(ev(
                    "audit/call",
                    seq,
                    json!({ "boundary": "llm", "operation": "request-done",
                        "detail": { "durationMs": 1200, "usage": {
                            "output_tokens": 42, "ttftMs": 300 } } }),
                ));
                seq += 1;
                events.push(ev(
                    "assistant/message",
                    seq,
                    json!({ "turn": turn, "step": step, "content": format!("回答{turn}-{step}"),
                        "tool_calls": [ { "id": format!("c{step}"), "name": "bash",
                            "arguments": "{\"command\":\"ls\"}" } ] }),
                ));
                seq += 1;
                events.push(ev(
                    "tool/call",
                    seq,
                    json!({ "turn": turn, "step": step, "name": "bash",
                        "arguments": "{\"command\":\"ls\"}" }),
                ));
                seq += 1;
                events.push(ev(
                    "tool/result",
                    seq,
                    json!({ "turn": turn, "step": step, "call": 1, "output": "ok", "success": true }),
                ));
            }
            seq += 1;
            // 交替正常/软取消收尾
            if turn % 4 == 0 {
                events.push(ev("turn/end", seq, json!({ "cancelled": "token" })));
            } else {
                events.push(ev("turn/end", seq, json!({})));
            }
        }
        events
    }

    /// UUID 形状串归一化:translator 的 message_id() 合成 v7 UUID 每次运行
    /// 都不同(时长有序),差分比较前先替换为占位符;其余字段逐字节锁死。
    fn canonicalize(v: &mut Value) {
        match v {
            Value::String(s) => {
                if uuid::Uuid::parse_str(s).is_ok() {
                    *s = "<uuid>".into();
                }
            }
            Value::Array(items) => items.iter_mut().for_each(canonicalize),
            Value::Object(map) => map.values_mut().for_each(canonicalize),
            _ => {}
        }
    }

    fn canonical(events: &[SessionEvent]) -> String {
        let mut v = serde_json::to_value(events).unwrap();
        canonicalize(&mut v);
        serde_json::to_string(&v).unwrap()
    }

    /// 差分锁:三组窗口(None / 边界落在消息中段 / 边界落在填充事件间)
    /// × 全量翻译 + paginate 与 page_cut + translate_window 输出一致
    /// (UUID 归一化后逐字节)。另单列构造:信封持久 source_event_seqs
    /// 与合成值不一致时,cut 取信封自身 seq(与 paginate 的合成 min 语义对齐)。
    #[test]
    fn window_translate_matches_full_translate_paginate_byte_for_byte() {
        let provider = info();
        let events = synthetic_log();
        for (before, max) in [
            (None, 5),
            (None, 30),
            (Some(events[40].seq), 7),
            (Some(events[57].seq), 3),
        ] {
            let expected = paginate(&translate_events(&provider, &events), before, max);
            let cut = page_cut(&events, before, max);
            let actual_events = translate_window(&provider, &events, cut, before);
            assert_eq!(cut, expected.cut, "cut 同源(before={before:?}, max={max})");
            assert_eq!(
                canonical(&actual_events),
                canonical(&expected.events),
                "窗口翻译逐字节一致(before={before:?}, max={max})"
            );
            assert_eq!(cut > 0, expected.has_more, "has_more 同源(cut>0)");
        }
    }

    /// 信封自带持久 source_event_seqs(归因链,供 event_trace 消费)与
    /// 翻译层合成值 vec![ev.seq] 不是一回事:窗口 cut 必须取后者语义
    /// (消息信封自身 seq),否则窗口边界错切。
    #[test]
    fn page_cut_uses_envelope_seq_not_persistent_source_seqs() {
        let mut m = ev("assistant/message", 10, json!({ "content": "a" }));
        m.source_event_seqs = Some(vec![3, 7]); // 持久归因链指向更早事件
        let events = vec![m];
        assert_eq!(page_cut(&events, None, 1), 10, "cut = 信封 seq");
    }
    /// llm request-done 用量附着:audit 事件本身丢弃,但下一个
    /// assistant/message 的 data 携带 usage(durationMs/ttftMs/outputTokens)
    #[test]
    fn usage_attached_to_assistant_message() {
        let mut tr = Translator::new(info());
        // request-done 先于 assistant/message 落档(引擎时序)
        assert!(
            tr.translate(&ev(
                "audit/call",
                9,
                json!({
                    "boundary": "llm",
                    "operation": "request-done",
                    "detail": { "durationMs": 3400, "usage": {
                        "output_tokens": 120, "ttftMs": 500 } },
                })
            ))
            .is_none(),
            "audit 事件不翻译成帧"
        );
        let out = tr
            .translate(&ev("assistant/message", 10, json!({ "content": "ok" })))
            .expect("assistant/message");
        assert_eq!(
            out.data["usage"],
            json!({ "durationMs": 3400, "ttftMs": 500, "outputTokens": 120 })
        );
        // 用量只附着一次:下一个 message 无 usage
        let next = tr
            .translate(&ev("assistant/message", 11, json!({ "content": "again" })))
            .expect("第二个 message");
        assert!(next.data.get("usage").is_none());
    }

    /// 冷启动计时分解(性能基线,真实大日志缺席即跳过):
    /// read → Value 解析 → decode_envelope → append → load_log 总账
    /// → page_cut+translate_window 全量,附单遍直解与序列化对照组。
    /// load_log 单遍直解改造(B)的前后对照就以此为本底数字
    #[test]
    fn cold_open_timing_breakdown() {
        const REAL_LOG: &str = "/Users/leexbo/.liuma/--Volumes-DATA-projects-liuma--/s-367e20369b584ddebffbc0b9d04501da/session.jsonl";
        if !std::path::Path::new(REAL_LOG).exists() {
            return;
        }
        use std::time::Instant;

        let t = Instant::now();
        let text = std::fs::read_to_string(REAL_LOG).unwrap();
        let read = t.elapsed();

        let t = Instant::now();
        let values: Vec<Value> = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let parse = t.elapsed();

        let t = Instant::now();
        let envs: Vec<liuma_session::EventEnvelope> = values
            .iter()
            .map(|v| liuma_session::decode_envelope(v).unwrap())
            .collect();
        let decode = t.elapsed();

        let t = Instant::now();
        let mut log = liuma_session::EventLog::new();
        for ev in envs {
            log.append(ev).unwrap();
        }
        let append = t.elapsed();

        let t = Instant::now();
        let log2 = liuma_app::load_log(REAL_LOG).unwrap();
        let load_log_total = t.elapsed();

        let slice: Vec<liuma_session::EventEnvelope> = log2.iter().cloned().collect();
        let provider = super::ProviderInfo {
            provider: "anthropic".into(),
            model: "test".into(),
        };
        let t = Instant::now();
        let cut = super::page_cut(&slice, None, usize::MAX);
        let events = super::translate_window(&provider, &slice, cut, None);
        let translate = t.elapsed();

        let t = Instant::now();
        let payload = serde_json::to_string(&events).unwrap();
        let ser = t.elapsed();
        let t = Instant::now();
        let back: Vec<super::SessionEvent> = serde_json::from_str(&payload).unwrap();
        let de = t.elapsed();
        assert_eq!(back.len(), events.len());

        // 单遍反序列化对照:Envelope 直解(跳过中间 Value 树 + 二次遍历;
        // 生产化需把 decode_envelope 的 fail-closed 守卫并入)
        let t = Instant::now();
        let direct_envs: Vec<liuma_session::EventEnvelope> = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let direct = t.elapsed();
        assert_eq!(direct_envs.len(), slice.len());

        eprintln!(
            "\n── 冷启动计时分解(n={} 事件,{} 字节)──",
            slice.len(),
            text.len()
        );
        eprintln!("read_to_string      {:>10.1?}", read);
        eprintln!("serde Value 解析    {:>10.1?}", parse);
        eprintln!("decode_envelope     {:>10.1?}", decode);
        eprintln!("EventLog::append    {:>10.1?}", append);
        eprintln!("load_log 总账       {:>10.1?}", load_log_total);
        eprintln!("page_cut+translate  {:>10.1?}", translate);
        eprintln!("(对照)单遍 Envelope 解 {:>8.1?}", direct);
        eprintln!("(对照)serde 序列化  {:>10.1?}  {} 字节", ser, payload.len());
        eprintln!("(对照)serde 反序列化 {:>10.1?}", de);
    }
}
