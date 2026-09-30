//! 上下文构成启发式折叠(UI ContextMeter 分段条/图例的数据源)。
//!
//! 价格语言:
//! 固定密度 4 字符/令牌,内容块每块 +4 结构开销,每条消息 +4 角色开销。
//! 纯函数:输入 = 日志事件切片,输出 = 三段构成;直播与冷会话重放同源,
//! 面板数字不依赖会话存活。

use serde_json::Value;

use liuma_session::EventEnvelope;

/// 固定文本密度(4 字符/令牌)
const CHARS_PER_TOKEN: u64 = 4;
/// 每块结构开销(`BLOCK_OVERHEAD`)
const BLOCK_OVERHEAD: u64 = 4;
/// 每条消息角色开销(`ROLE_OVERHEAD`)
const ROLE_OVERHEAD: u64 = 4;

/// 上下文构成三段(`contextBreakdown` 投影同形)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBreakdown {
    /// 最新请求信封的 system prompt 启发式价格
    pub system_tokens: u64,
    /// 最新请求信封的工具 schema 启发式价格
    pub tools_tokens: u64,
    /// 模型可见消息(对话/思考/工具结果/折叠摘要)累积启发式价格
    pub message_tokens: u64,
}

fn ceil_div(n: u64, d: u64) -> u64 {
    n.div_ceil(d)
}

/// 单条模型可见消息的价格:内容块(文本/工具调用) + 角色开销
fn message_tokens(data: &Value) -> u64 {
    let mut tokens = ROLE_OVERHEAD;
    if let Some(text) = data["content"].as_str() {
        tokens += ceil_div(text.chars().count() as u64, CHARS_PER_TOKEN) + BLOCK_OVERHEAD;
    }
    if let Some(calls) = data["tool_calls"].as_array() {
        for call in calls {
            let name = call["name"].as_str().unwrap_or_default();
            let args = serde_json::to_string(&call["arguments"]).unwrap_or_default();
            tokens += ceil_div(name.chars().count() as u64, CHARS_PER_TOKEN)
                + ceil_div(args.chars().count() as u64, CHARS_PER_TOKEN)
                + BLOCK_OVERHEAD;
        }
    }
    tokens
}

/// 从日志计算上下文构成。
///
/// - system/tools 取**最新** audit llm "request" 记录(引擎出网前落档的
///   字符长度;重放可得,与直播一致——「最新请求信封」语义;压缩不
///   影响两段,system/工具目录不随折叠变化)
/// - message 按事件类型折叠 user/message、assistant/message、思考、
///   工具结果与折叠摘要,只计**模型可见面**:最新 `compaction/summary`
///   的 `throughSeq` 之前的历史已被折叠为该摘要(与
///   `derive_visible_messages` 同一语义),不再计入——否则分项随全量
///   历史单调累积,与压缩后的占用环同屏背离
pub fn context_breakdown<'a>(
    events: impl IntoIterator<Item = &'a EventEnvelope>,
) -> ContextBreakdown {
    let events: Vec<&EventEnvelope> = events.into_iter().collect();
    let through_seq = events
        .iter()
        .rev()
        .find(|e| e.r#type == "compaction/summary")
        .and_then(|e| e.data["throughSeq"].as_u64())
        .unwrap_or(0);
    let mut system_tokens = 0u64;
    let mut tools_tokens = 0u64;
    let mut message = 0u64;
    for ev in events {
        match ev.r#type.as_str() {
            "audit/call"
                if ev.data["boundary"].as_str() == Some("llm")
                    && ev.data["operation"].as_str() == Some("request") =>
            {
                if let Some(chars) = ev.data["detail"]["systemChars"].as_u64() {
                    system_tokens = ceil_div(chars, CHARS_PER_TOKEN) + ROLE_OVERHEAD;
                }
                if let Some(chars) = ev.data["detail"]["toolsChars"].as_u64() {
                    tools_tokens = ceil_div(chars, CHARS_PER_TOKEN) + BLOCK_OVERHEAD;
                }
            }
            _ => {
                // 被折叠遮蔽的历史(已定序且 seq <= 最新摘要 throughSeq)
                // 已由摘要代表,不再占上下文;seq 0 = 未定序,不参与判定
                if ev.seq > 0 && ev.seq <= through_seq {
                    continue;
                }
                message += message_contribution(ev);
            }
        }
    }
    ContextBreakdown {
        system_tokens,
        tools_tokens,
        message_tokens: message,
    }
}

/// 单条事件对 message 段的贡献(0 = 不计:非可见类型或注入上下文)。
/// 批量([`context_breakdown`])与增量([`BreakdownAcc`])共用,口径不二
fn message_contribution(ev: &EventEnvelope) -> u64 {
    match ev.r#type.as_str() {
        "user/message"
        | "assistant/message"
        | "assistant/reasoning"
        | "tool/result"
        | "compaction/summary" => {
            // 注入上下文(user/message + source.kind != "user")不计入
            // message_tokens:注入是旁车上下文,不占历史消息数;
            // 真实用户/助手消息照算。
            if ev.r#type == "user/message"
                && ev.data["source"]["kind"].as_str().unwrap_or("user") != "user"
            {
                return 0;
            }
            match ev.r#type.as_str() {
                "assistant/reasoning" => ev.data["text"].as_str().map_or(0, price_block),
                "tool/result" => ev.data["output"].as_str().map_or(0, price_block),
                "compaction/summary" => ev.data["summary"].as_str().map_or(0, price_block),
                _ => message_tokens(&ev.data),
            }
        }
        _ => 0,
    }
}

/// [`context_breakdown`] 的增量维护形态:事件按 seq 顺序喂入,快照 O(1)。
///
/// 直播热路径(`push_stats_frame` 每 stats 事件一查)用:打包常驻后
/// 全量扫需展开整表,长会话每帧上百毫秒不可接受。message 段走前缀和:
/// 折叠掩蔽(throughSeq)总在被打蔽事件**之后**落地,快照时减去被遮蔽
/// 前缀即得,与全量扫逐字段相等(等价锁测试)。
#[derive(Debug, Clone)]
pub struct BreakdownAcc {
    /// 下一条期待喂入的 seq(已喂入 1..next_seq)
    next_seq: u64,
    /// 最新请求信封的 system/tools 价格(audit 覆盖式,与批量同)
    system_tokens: u64,
    tools_tokens: u64,
    /// 前缀和:prefix[s] = seq 1..=s 的 message 贡献和(prefix[0] = 0)
    prefix: Vec<u64>,
    /// 最新 compaction/summary 的 throughSeq(0 = 未折叠)
    through_seq: u64,
}

impl Default for BreakdownAcc {
    fn default() -> Self {
        Self {
            next_seq: 1,
            system_tokens: 0,
            tools_tokens: 0,
            prefix: vec![0],
            through_seq: 0,
        }
    }
}

impl BreakdownAcc {
    /// 顺序喂入一条已定序事件(seq 必须等于 [`Self::next_seq`]:
    /// 乱序/重复忽略——缺口由调用方补齐后再喂)
    pub fn push(&mut self, ev: &EventEnvelope) {
        if ev.seq != self.next_seq {
            return;
        }
        self.next_seq += 1;
        match ev.r#type.as_str() {
            "audit/call"
                if ev.data["boundary"].as_str() == Some("llm")
                    && ev.data["operation"].as_str() == Some("request") =>
            {
                if let Some(chars) = ev.data["detail"]["systemChars"].as_u64() {
                    self.system_tokens = ceil_div(chars, CHARS_PER_TOKEN) + ROLE_OVERHEAD;
                }
                if let Some(chars) = ev.data["detail"]["toolsChars"].as_u64() {
                    self.tools_tokens = ceil_div(chars, CHARS_PER_TOKEN) + BLOCK_OVERHEAD;
                }
            }
            "compaction/summary" => {
                self.through_seq = ev.data["throughSeq"].as_u64().unwrap_or(0);
            }
            _ => {}
        }
        let c = message_contribution(ev);
        self.prefix
            .push(self.prefix.last().copied().unwrap_or(0) + c);
    }

    /// 下一条期待喂入的 seq(补洞起点)
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// 当前构成(O(1),与 [`context_breakdown`] 全量扫同口径)
    pub fn snapshot(&self) -> ContextBreakdown {
        let total = self.prefix.last().copied().unwrap_or(0);
        // throughSeq 越界(> 已喂入末 seq)→ 全量遮蔽,与批量掩蔽一致
        let masked = self
            .prefix
            .get(self.through_seq as usize)
            .copied()
            .unwrap_or(total);
        ContextBreakdown {
            system_tokens: self.system_tokens,
            tools_tokens: self.tools_tokens,
            message_tokens: total - masked,
        }
    }
}

/// 单文本块价格:字符价 + 块结构开销 + 角色开销
fn price_block(text: &str) -> u64 {
    ceil_div(text.chars().count() as u64, CHARS_PER_TOKEN) + BLOCK_OVERHEAD + ROLE_OVERHEAD
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(r#type: &str, data: Value) -> EventEnvelope {
        EventEnvelope::new(r#type, 0, data)
    }

    fn audit_llm_request(system_chars: u64, tools_chars: u64) -> EventEnvelope {
        ev(
            "audit/call",
            json!({
                "boundary": "llm",
                "operation": "request",
                "detail": { "systemChars": system_chars, "toolsChars": tools_chars },
            }),
        )
    }

    #[test]
    fn empty_log_yields_zero_breakdown() {
        let b = context_breakdown(&[]);
        assert_eq!(
            b,
            ContextBreakdown {
                system_tokens: 0,
                tools_tokens: 0,
                message_tokens: 0
            }
        );
    }

    #[test]
    fn header_chars_priced_at_4_chars_per_token() {
        let log = vec![
            ev("turn/start", json!({})),
            audit_llm_request(100, 200),
            ev("user/message", json!({ "content": "hello" })),
        ];
        // system = ceil(100/4) + 4(角色)= 29;tools = ceil(200/4) + 4(块)= 54
        let b = context_breakdown(&log);
        assert_eq!(b.system_tokens, 29);
        assert_eq!(b.tools_tokens, 54);
        // "hello" = ceil(5/4) + 4(块) + 4(角色)= 10
        assert_eq!(b.message_tokens, 10);
    }

    #[test]
    fn latest_request_envelope_wins() {
        let log = vec![audit_llm_request(100, 0), audit_llm_request(400, 100)];
        let b = context_breakdown(&log);
        assert_eq!(b.system_tokens, ceil_div(400, 4) + ROLE_OVERHEAD);
        assert_eq!(b.tools_tokens, ceil_div(100, 4) + BLOCK_OVERHEAD);
    }

    #[test]
    fn messages_fold_with_tool_calls_and_other_surface_types() {
        let log = vec![
            ev("user/message", json!({ "content": "abcd" })),
            ev(
                "assistant/message",
                json!({
                    "content": "efgh",
                    "tool_calls": [{
                        "name": "bash",
                        "arguments": { "command": "ls" },
                    }],
                }),
            ),
            ev("assistant/reasoning", json!({ "text": "ijkl" })),
            ev("tool/result", json!({ "output": "mnop" })),
            ev("compaction/summary", json!({ "summary": "qrst" })),
        ];
        let b = context_breakdown(&log);
        // user "abcd" = 1 + 4 + 4 = 9
        // assistant 内容 "efgh" = 9;工具调用 name "bash" = 1、
        //   args `{"command":"ls"}` 15 字符 = ceil(15/4)=4 + 块4 → 1+4+4 = 9
        //   → assistant 共 18
        // reasoning / tool/result / summary 各 9
        assert_eq!(b.message_tokens, 9 + 18 + 9 + 9 + 9);
    }

    #[test]
    fn non_request_audit_records_ignored() {
        let log = vec![ev(
            "audit/call",
            json!({ "boundary": "tool", "operation": "bash", "detail": {} }),
        )];
        let b = context_breakdown(&log);
        assert_eq!(b.system_tokens, 0);
        assert_eq!(b.tools_tokens, 0);
    }

    #[test]
    fn folded_history_is_excluded_from_message_tokens() {
        // 压缩可见面:最新 compaction/summary 的 throughSeq 之前的历史
        // (含更早的摘要)已折叠为该摘要,分项只计最新摘要与其后消息——
        // 与占用环同口径,否则分项随全量历史累积、压缩后不同步回落
        let mut early = ev("user/message", json!({ "content": "abcd" }));
        early.seq = 1;
        let mut folded_result = ev("tool/result", json!({ "output": "wxyz" }));
        folded_result.seq = 2;
        let mut old_summary = ev(
            "compaction/summary",
            json!({ "summary": "old summary", "throughSeq": 2 }),
        );
        old_summary.seq = 3;
        let mut middle = ev("assistant/message", json!({ "content": "mid answer" }));
        middle.seq = 4;
        let mut summary = ev(
            "compaction/summary",
            json!({ "summary": "condensed", "throughSeq": 4 }),
        );
        summary.seq = 5;
        let mut later = ev("user/message", json!({ "content": "later" }));
        later.seq = 6;

        let b = context_breakdown([
            &early,
            &folded_result,
            &old_summary,
            &middle,
            &summary,
            &later,
        ]);
        // 摘要 "condensed" = ceil(9/4)+4+4 = 11;"later" = ceil(5/4)+4+4 = 10;
        // seq <= 4 的四条(early/tool 结果/旧摘要/middle)不计
        assert_eq!(b.message_tokens, 11 + 10);
        // system/tools 语义不受压缩影响:折叠前的 request 记录仍生效
        let log_with_request = vec![audit_llm_request(100, 200), summary];
        let b = context_breakdown(&log_with_request);
        assert_eq!(b.system_tokens, ceil_div(100, 4) + ROLE_OVERHEAD);
        assert_eq!(b.tools_tokens, ceil_div(200, 4) + BLOCK_OVERHEAD);
    }

    /// 等价锁:BreakdownAcc 逐事件喂入 == context_breakdown 全量扫,
    /// 且在**每个前缀**上三字段逐一相等(增量维护不得漂移)。流内含
    /// audit 覆盖、注入消息、可打包 reasoning 对、中段与二段摘要
    #[test]
    fn breakdown_acc_matches_full_scan_at_every_prefix() {
        let stream: Vec<EventEnvelope> = vec![
            audit_llm_request(100, 200),
            ev("user/message", json!({ "content": "hi there" })),
            ev(
                "user/message",
                json!({ "content": "[injected]", "source": { "kind": "plugin" } }),
            ),
            EventEnvelope::new_ignorable(
                "assistant/reasoning",
                1,
                json!({ "text": "thinking hard about it" }),
            ),
            EventEnvelope::new_ignorable("assistant/reasoning", 2, json!({ "text": "more" })),
            ev(
                "assistant/message",
                json!({ "content": "answer", "tool_calls": [
                    { "name": "bash", "arguments": { "command": "ls" } }
                ] }),
            ),
            ev("tool/result", json!({ "output": "file_a file_b" })),
            audit_llm_request(8, 40),
            ev(
                "compaction/summary",
                json!({ "summary": "condensed history", "throughSeq": 6 }),
            ),
            ev("user/message", json!({ "content": "after fold" })),
            ev(
                "compaction/summary",
                json!({ "summary": "folded again", "throughSeq": 9 }),
            ),
            ev("assistant/message", json!({ "content": "final" })),
        ];
        let mut log = liuma_session::EventLog::new();
        let mut acc = BreakdownAcc::default();
        for e in stream {
            let seq = log.append(e).expect("append");
            let sequenced = log.get(seq).expect("get");
            acc.push(&sequenced);
            let snap: Vec<EventEnvelope> = log.iter().collect();
            let full = context_breakdown(snap.iter());
            assert_eq!(acc.snapshot(), full, "前缀 seq={seq} 处增量/全量漂移");
        }
    }

    /// 等价锁(真实样本,env 门控):整段日志喂入后与全量扫相等
    #[test]
    fn breakdown_acc_matches_full_scan_on_real_log() {
        let path = std::env::var("LIUMA_REAL_LOG").unwrap_or_else(|_| {
            "/Users/leexbo/.liuma/--Volumes-DATA-projects-liuma--/s-4ffe75f20bc846ffbaf4243667d53e8e/session.jsonl".into()
        });
        if !std::path::Path::new(&path).exists() {
            eprintln!("[ctx] 跳过(样本不存在:{path})");
            return;
        }
        let log = liuma_app::load_log(&path).expect("load");
        let mut acc = BreakdownAcc::default();
        for e in log.iter() {
            acc.push(&e);
        }
        let snap: Vec<EventEnvelope> = log.iter().collect();
        let full = context_breakdown(snap.iter());
        assert_eq!(acc.snapshot(), full);
    }
}
