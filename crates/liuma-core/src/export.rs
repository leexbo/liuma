//! 会话导出的 markdown 形态(人类可读):消息流 + 工具卡摘要行。
//!
//! 结构词用英文(数据产物,非 UI 文案),正文原样;后代小节由 registry
//! 侧按血缘序拼接(单文件独立阅读)。工具行摘要与桌面工具卡同源
//! ([`summarize_call`] 自 projection 上提):`tool/call` 与 `tool/result`
//! 以 call 事件的 seq 配对,无 result 的调用按「无结果」呈现。

use std::collections::HashMap;

use serde_json::Value;

/// 工具调用参数摘要(与桌面工具卡同源):按工具名取最可读的参数键。
/// shell 工具的模型面名字随平台走(`bash` / `pwsh`),按实际取
pub fn summarize_call(name: &str, arguments: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(arguments) else {
        return one_line(arguments, 80);
    };
    let shell_tool = liuma_sandbox::shell::tool_name();
    let keys: &[&str] = match name {
        // shell 工具优先 description(必填参数,给用户看的
        // 一句意图说明),回退 command
        n if n == shell_tool => &["description", "command"],
        "file_read" => &["path"],
        "file_edit" => &["path"],
        "present" => &["path"],
        "file_search" => &["content", "glob", "path"],
        "subagent" | "ralph" => &["task"],
        "workflow" => &["steps"],
        "goal" | "jobs" => &["action"],
        "exit_plan_mode" => &["plan"],
        _ => {
            return first_string(&v)
                .map(|s| one_line(&s, 80))
                .unwrap_or_else(|| one_line(arguments, 80));
        }
    };
    for key in keys {
        match &v[*key] {
            // workflow.steps:数组首元素(字符串)
            Value::Array(a) if *key == "steps" => {
                if let Some(first) = a.iter().find_map(|s| s.as_str()) {
                    return one_line(first, 80);
                }
            }
            Value::String(s) if !s.is_empty() => return one_line(s, 80),
            _ => {}
        }
    }
    first_string(&v)
        .map(|s| one_line(&s, 80))
        .unwrap_or_default()
}

/// 单行化 + 字符级截断(中文安全)
fn one_line(s: &str, max: usize) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&collapsed, max)
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}

fn first_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => a.iter().find_map(first_string),
        Value::Object(o) => o.values().find_map(first_string),
        _ => None,
    }
}

/// content 两态(字符串或 text 块数组)→ 纯文本
fn blocks_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b["type"].as_str() == Some("text"))
            .map(|b| b["text"].as_str().unwrap_or_default())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// 待配对的工具调用(call 事件先行,result 事件回填成败)
struct PendingCall {
    name: String,
    summary: String,
}

/// 一段会话日志(JSONL 文本)→ markdown 消息流(不含文档头与后代
/// 小节)。坏行跳过;user 注入上下文(source.kind != "user")以
/// Context 呈现(与转录面一致)
pub fn stream_markdown(log_text: &str) -> String {
    let mut out = String::new();
    let mut pending: HashMap<u64, PendingCall> = HashMap::new();
    let section = |out: &mut String, role: &str, body: &str| {
        let body = body.trim();
        if body.is_empty() {
            return;
        }
        if !out.is_empty() {
            out.push_str("\n---\n\n");
        }
        out.push_str(&format!("**{role}**\n\n{body}\n"));
    };
    for line in log_text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        for ev in liuma_session::chunk_rows::decode_line_events(line)
            .into_iter()
            .flatten()
        {
            match ev.r#type.as_str() {
                "user/message" => {
                    let kind = ev.data["source"]["kind"].as_str().unwrap_or("user");
                    let role = if kind == "user" { "User" } else { "Context" };
                    let text = blocks_text(&ev.data["content"]);
                    section(&mut out, role, &text);
                }
                "assistant/message" => {
                    let text = if ev.data["content"].is_string() {
                        blocks_text(&ev.data["content"])
                    } else {
                        blocks_text(&ev.data["message"]["content"])
                    };
                    section(&mut out, "Assistant", &text);
                }
                "tool/call" => {
                    let name = ev.data["name"].as_str().unwrap_or_default();
                    let arguments = match &ev.data["arguments"] {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    pending.insert(
                        ev.seq,
                        PendingCall {
                            name: name.to_string(),
                            summary: summarize_call(name, &arguments),
                        },
                    );
                }
                "tool/result" => {
                    let call_seq = ev.data["call"].as_u64();
                    let call = call_seq.and_then(|seq| pending.remove(&seq));
                    let Some(call) = call else { continue };
                    let exit = ev.data["view"]["Terminal"]["exitCode"].as_i64();
                    let mark = match (ev.data["success"].as_bool(), exit) {
                        (Some(false), Some(code)) => format!("✗ (exit {code})"),
                        (Some(false), _) => "✗".to_string(),
                        (_, Some(0)) | (Some(true), None) => "✓".to_string(),
                        (_, Some(code)) => format!("✓ (exit {code})"),
                        (None, None) => "(no result)".to_string(),
                    };
                    let summary = if call.summary.is_empty() {
                        call.name.clone()
                    } else {
                        format!("{} — {}", call.name, call.summary)
                    };
                    section(&mut out, "Tool", &format!("- `{summary}` — {mark}"));
                }
                "compaction/summary" => {
                    section(&mut out, "System", "*context compacted*");
                }
                "turn/error" => {
                    let err = ev.data["error"].as_str().unwrap_or("unknown");
                    section(&mut out, "Error", &one_line(err, 200));
                }
                _ => {}
            }
        }
    }
    // 收尾:无 result 的调用按「无结果」呈现(回合中断/导出时点在途)
    for (_, call) in pending {
        let summary = if call.summary.is_empty() {
            call.name.clone()
        } else {
            format!("{} — {}", call.name, call.summary)
        };
        section(&mut out, "Tool", &format!("- `{summary}` — (no result)"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use liuma_session::EventEnvelope;
    use serde_json::json;

    fn envelope(ty: &str, seq: u64, data: Value) -> EventEnvelope {
        // EventLog.append 才派发 seq;直构时手工置(配对键 = seq)
        let mut ev = EventEnvelope::new(ty, 0, data);
        ev.seq = seq;
        ev
    }

    fn lines(log: &[EventEnvelope]) -> String {
        log.iter()
            .map(|ev| format!("{}\n", serde_json::to_string(ev).unwrap()))
            .collect()
    }

    #[test]
    fn summarize_call_prefers_readable_keys() {
        let shell = liuma_sandbox::shell::tool_name();
        let s = summarize_call(shell, r#"{"description":"跑测试","command":"cargo test"}"#);
        assert_eq!(s, "跑测试");
        assert_eq!(
            summarize_call("file_read", r#"{"path":"/a/b.rs"}"#),
            "/a/b.rs"
        );
        // 坏 JSON 回落原文单行化
        assert_eq!(summarize_call("x", "not json"), "not json");
    }

    #[test]
    fn stream_renders_roles_tools_and_pairing() {
        let shell = liuma_sandbox::shell::tool_name();
        let log = vec![
            envelope(
                "user/message",
                1,
                json!({ "content": "帮我看看构建", "source": { "kind": "user" } }),
            ),
            envelope(
                "user/message",
                2,
                json!({ "content": "(注入上下文)", "source": { "kind": "injected" } }),
            ),
            envelope("assistant/message", 3, json!({ "content": "我来跑测试" })),
            envelope(
                "tool/call",
                4,
                json!({ "name": shell, "arguments": r#"{"command":"cargo test"}"# }),
            ),
            envelope(
                "tool/result",
                5,
                json!({ "call": 4, "output": "ok", "success": true,
                        "view": { "Terminal": { "exitCode": 0 } } }),
            ),
            envelope(
                "tool/call",
                6,
                json!({ "name": "file_edit", "arguments": r#"{"path":"/x/y.rs"}"# }),
            ),
            envelope(
                "compaction/summary",
                7,
                json!({ "summary": "s", "throughSeq": 6 }),
            ),
        ];
        let md = stream_markdown(&lines(&log));
        assert!(md.contains("**User**\n\n帮我看看构建"), "user 正文: {md}");
        assert!(
            md.contains("**Context**\n\n(注入上下文)"),
            "注入以 Context 呈现"
        );
        assert!(md.contains("**Assistant**\n\n我来跑测试"));
        assert!(
            md.contains(&format!("- `{shell} — cargo test` — ✓")),
            "工具摘要行: {md}"
        );
        // 无 result 的调用收尾按「无结果」呈现
        assert!(md.contains("`file_edit — /x/y.rs` — (no result)"), "{md}");
        assert!(md.contains("*context compacted*"), "压缩标记: {md}");
        // 分节横线夹在角色小节之间
        assert!(md.matches("\n---\n\n").count() >= 5, "分节数: {md}");
    }

    #[test]
    fn stream_marks_failed_tool_with_exit_code() {
        let shell = liuma_sandbox::shell::tool_name();
        let log = vec![
            envelope(
                "tool/call",
                1,
                json!({ "name": shell, "arguments": r#"{"command":"false"}"# }),
            ),
            envelope(
                "tool/result",
                2,
                json!({ "call": 1, "output": "", "success": false,
                        "view": { "Terminal": { "exitCode": 1 } } }),
            ),
        ];
        let md = stream_markdown(&lines(&log));
        assert!(
            md.contains(&format!("- `{shell} — false` — ✗ (exit 1)")),
            "失败工具行应带退出码: {md}"
        );
    }
}
