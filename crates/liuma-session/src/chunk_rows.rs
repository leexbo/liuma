//! 存储层无损打包:连续同型 delta 事件 → 一行存储行。
//!
//! Provider 流按 token 送增量,`assistant/reasoning` 与 `assistant/chunk`
//! 每条只有几个字节,却各带一整份 JSON 信封——真实会话实测 290,876 条中
//! 91.3% 是这两类,堆表示 7.7× 于磁盘字节。本模块把「≥3 条连续、同型、
//! 同 ignorable、seq 连续」的 delta 事件打包成一行 [`ChunkRow`],解码
//! 逐字节还原原事件(seq/time/type/data/ignorable 全等)。
//!
//! 打包行是**存储词汇,不是事件**:不进 [`crate::log::EventLog`] 的逻辑
//! 事件流,不参与 seq 分配,不进入任何 `session/event` 投影。读写两侧
//! 布局盲:同一份日志,packed / unpacked / 混排加载出完全相同的逻辑
//! 事件流,故无需格式版本变更、无需迁移。
//!
//! 编码白名单取**精确键**匹配;任何不完全匹配的形态逐字原样落盘——
//! 丢压缩,不丢数据。解码对行标签值先验证后展开,坏行报错(fail-loud):
//! 把坏行当普通事件会让一整段流静默消失。

use serde::{Deserialize, Serialize};

use crate::envelope::EventEnvelope;

/// 少于该成员数的 run 不打包:信封开销抵消收益。
/// 格式常量而非可调参——两种布局解码等价,改它不会作废已存日志。
pub const MIN_RUN: usize = 3;

/// 可打包的 delta 种类(事件类型级判别)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaKind {
    /// `assistant/reasoning`,data 精确键 `{text}`,text 为增量片段
    Reasoning,
    /// `assistant/chunk`,data 精确键 `{delta}`,delta 为增量片段
    Chunk,
}

impl DeltaKind {
    /// 行标签(落盘 kind 字段值)
    pub fn row_tag(self) -> &'static str {
        match self {
            DeltaKind::Reasoning => "reasoning",
            DeltaKind::Chunk => "chunk",
        }
    }

    fn event_type(self) -> &'static str {
        match self {
            DeltaKind::Reasoning => "assistant/reasoning",
            DeltaKind::Chunk => "assistant/chunk",
        }
    }

    /// 行标签 → 种类(读侧判别用)
    pub fn from_row_tag(tag: &str) -> Option<Self> {
        match tag {
            "reasoning" => Some(DeltaKind::Reasoning),
            "chunk" => Some(DeltaKind::Chunk),
            _ => None,
        }
    }

    fn field(self) -> &'static str {
        match self {
            DeltaKind::Reasoning => "text",
            DeltaKind::Chunk => "delta",
        }
    }
}

/// 一行打包存储行:成员 k 还原为 seq `seq0 + k`、time `time0 + Σdt[..k)`、
/// 片段 `texts[k]`。`texts` 保留逐片段、绝不 join——token 边界是数据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkRow {
    /// 首成员 seq
    pub seq0: u64,
    /// 首成员 time(毫秒)
    pub time0: i64,
    /// 相邻成员 time 差(长度 = 成员数 − 1);可为负(墙钟回拨)
    pub dt: Vec<i64>,
    /// 逐片段文本(长度 = 成员数;含首成员)
    pub texts: Vec<String>,
}

/// 一条持久化记录:原事件,或一行打包行。
#[derive(Debug, Clone, PartialEq)]
pub enum StorageRecord {
    /// 普通事件(逐字透传)
    Event(EventEnvelope),
    /// 打包行(解码时展开为 `texts.len()` 条事件)
    Packed(ChunkRow, DeltaKind),
}

/// 顶层行判别键(`{"row":"chunks",...}` 的键名)。事件类型皆含 `/`,
/// 裸词不相交(先例:JSONL 头行的 `session` 标签)。
pub const ROW_KEY: &str = "row";
/// 打包行的判别值(`"row":"chunks"` 的值;目前唯一一种物理行)。
pub const ROW_TAG: &str = "chunks";

/// 是否为可打包事件:类型 + data 精确键 + ignorable。
/// 输入可能来自实时 append 或解析出的存量文件,故按结构校验,不信类型系统。
pub fn classify(ev: &EventEnvelope) -> Option<DeltaKind> {
    if !ev.ignorable || !ev.surface_op.is_none() || !ev.source_event_seqs.is_none() {
        return None;
    }
    let kind = match ev.r#type.as_str() {
        "assistant/reasoning" => DeltaKind::Reasoning,
        "assistant/chunk" => DeltaKind::Chunk,
        _ => return None,
    };
    // 精确键:多一键/缺一键都不打包(白名单 fall-through = 逐字透传)
    let ok = match kind {
        DeltaKind::Reasoning => ev
            .data
            .as_object()
            .is_some_and(|m| m.len() == 1 && m.contains_key("text") && m["text"].is_string()),
        DeltaKind::Chunk => ev
            .data
            .as_object()
            .is_some_and(|m| m.len() == 1 && m.contains_key("delta") && m["delta"].is_string()),
    };
    ok.then_some(kind)
}

fn fragment(kind: DeltaKind, ev: &EventEnvelope) -> &str {
    ev.data
        .get(kind.field())
        .and_then(|v| v.as_str())
        .unwrap_or_default()
}

/// `next` 是否能接在 `prev` 所在 run 之后(同型已由调用方判定)。
pub fn continues(prev: &EventEnvelope, next: &EventEnvelope) -> bool {
    if next.seq != prev.seq + 1 {
        return false;
    }
    // 相邻时间差必须可精确表示:i64 减法不溢出即可
    // (解码侧逐段 checked_add,两侧判定一致)
    next.time.checked_sub(prev.time).is_some()
}

/// 打包一段事件:每段 ≥[`MIN_RUN`] 的连续同型 delta 收进一行
/// [`ChunkRow`],其余逐字透传,顺序不变。纯函数、无状态——对任意
/// 切批都正确(flush 边界只影响行边界,不影响解码结果)。
pub fn pack(events: &[EventEnvelope]) -> Vec<StorageRecord> {
    let mut out = Vec::new();
    let mut kind: Option<DeltaKind> = None;
    let mut run: Vec<&EventEnvelope> = Vec::new();
    let flush = |out: &mut Vec<StorageRecord>,
                 kind: &mut Option<DeltaKind>,
                 run: &mut Vec<&EventEnvelope>| {
        if let Some(k) = *kind {
            if run.len() >= MIN_RUN {
                let dt = run.windows(2).map(|w| w[1].time - w[0].time).collect();
                out.push(StorageRecord::Packed(
                    ChunkRow {
                        seq0: run[0].seq,
                        time0: run[0].time,
                        dt,
                        texts: run.iter().map(|ev| fragment(k, ev).to_string()).collect(),
                    },
                    k,
                ));
            } else {
                out.extend(run.iter().map(|ev| StorageRecord::Event((*ev).clone())));
            }
        }
        *kind = None;
        run.clear();
    };
    for ev in events {
        match (classify(ev), kind) {
            (Some(k), Some(cur)) if k == cur && run.last().is_some_and(|p| continues(p, ev)) => {
                run.push(ev);
            }
            (k, _) => {
                flush(&mut out, &mut kind, &mut run);
                if let Some(k) = k {
                    kind = Some(k);
                    run.push(ev);
                } else {
                    out.push(StorageRecord::Event(ev.clone()));
                }
            }
        }
    }
    flush(&mut out, &mut kind, &mut run);
    out
}

/// 行级解码错误(fail-loud:坏行是存储损坏,静默展开会丢一整段流)
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("malformed {tag} storage row: {why}")]
pub struct RowError {
    /// 行标签
    pub tag: String,
    /// 具体原因
    pub why: String,
}

/// 展开一行打包行为原事件序列(顺序不变)。先验证后展开。
pub fn expand(row: &ChunkRow, kind: DeltaKind) -> Result<Vec<EventEnvelope>, RowError> {
    let tag = kind.row_tag();
    if row.texts.is_empty() {
        return Err(RowError {
            tag: tag.into(),
            why: "texts must be non-empty".into(),
        });
    }
    if row.dt.len() != row.texts.len() - 1 {
        return Err(RowError {
            tag: tag.into(),
            why: format!(
                "dt length {} does not match {} members",
                row.dt.len(),
                row.texts.len()
            ),
        });
    }
    // 重建边界:编码端只收安全整数时间;越界即不属于任何编码端输出
    let mut time = row.time0;
    for gap in &row.dt {
        time = time.checked_add(*gap).ok_or_else(|| RowError {
            tag: tag.into(),
            why: "member times must stay in range".into(),
        })?;
    }
    let mut events = Vec::with_capacity(row.texts.len());
    let mut time = row.time0;
    for (k, text) in row.texts.iter().enumerate() {
        if k > 0 {
            time += row.dt[k - 1];
        }
        let mut ev = EventEnvelope::new_ignorable(
            kind.event_type(),
            time,
            serde_json::json!({ kind.field(): text }),
        );
        ev.seq = row.seq0 + k as u64;
        events.push(ev);
    }
    Ok(events)
}

/// 一行 JSONL → 逻辑事件(**读侧布局盲统一入口**):普通行先走信封
/// 直解快路(无中间 Value 树);直解失败且行为打包行(`row` 键判别)
/// 时展开成员。`load_log` / `EventStore` 等一切「按行读文件」的路径
/// 必须经此判别——绕过它直解信封会把打包行当损坏文件拒载
/// (缺 `type` 字段)。非打包行的报错保持信封守卫原语义不变。
pub fn decode_line_events(line: &str) -> Result<Vec<EventEnvelope>, crate::EnvelopeError> {
    match crate::decode_envelope_str(line) {
        Ok(ev) => Ok(vec![ev]),
        Err(envelope_err) => {
            let raw: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => return Err(envelope_err),
            };
            if raw.get(ROW_KEY).and_then(|v| v.as_str()) != Some(ROW_TAG) {
                return Err(envelope_err);
            }
            let kind = raw
                .get("kind")
                .and_then(|v| v.as_str())
                .and_then(DeltaKind::from_row_tag)
                .ok_or_else(|| {
                    crate::EnvelopeError::Decode("packed row missing/unknown `kind`".into())
                })?;
            let row: ChunkRow = serde_json::from_value(raw)
                .map_err(|e| crate::EnvelopeError::Decode(format!("packed row corrupt: {e}")))?;
            expand(&row, kind)
                .map_err(|e| crate::EnvelopeError::Decode(format!("packed row invalid: {e}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 行解码统一入口:普通行直解、打包行展开、坏行报原信封错
    #[test]
    fn decode_line_events_is_layout_blind() {
        // 普通行:单事件直解(含守卫语义)
        let normal =
            r#"{"type":"user/message","seq":1,"time":0,"data":{"content":"hi"},"ignorable":false}"#;
        let evs = decode_line_events(normal).expect("普通行直解");
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].r#type, "user/message");
        // 打包行:展开为成员(seq 锚定逐条还原)
        let packed = r#"{"row":"chunks","kind":"reasoning","seq0":1,"time0":1,"dt":[1,1],"texts":["a","b","c"]}"#;
        let evs = decode_line_events(packed).expect("打包行展开");
        assert_eq!(
            evs.iter()
                .map(|e| e.data["text"].clone())
                .collect::<Vec<_>>(),
            vec![json!("a"), json!("b"), json!("c")]
        );
        assert_eq!(evs.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2, 3]);
        // 坏行:非打包行报原信封错(守卫语义不变)
        assert!(decode_line_events("{not json").is_err());
        assert!(
            decode_line_events(
                r#"{"type":"evil/unknown","seq":1,"time":0,"data":{},"ignorable":false}"#
            )
            .is_err()
        );
        // 打包行损坏:fail-loud
        assert!(
            decode_line_events(
                r#"{"row":"chunks","kind":"reasoning","seq0":1,"time0":1,"dt":[1],"texts":[]}"#
            )
            .is_err()
        );
    }

    fn reasoning(seq: u64, time: i64, text: &str) -> EventEnvelope {
        let mut ev =
            EventEnvelope::new_ignorable("assistant/reasoning", time, json!({ "text": text }));
        ev.seq = seq;
        ev
    }

    fn chunk(seq: u64, time: i64, delta: &str) -> EventEnvelope {
        let mut ev =
            EventEnvelope::new_ignorable("assistant/chunk", time, json!({ "delta": delta }));
        ev.seq = seq;
        ev
    }

    fn expanded(events: &[StorageRecord]) -> Vec<EventEnvelope> {
        let mut out = Vec::new();
        for rec in events {
            match rec {
                StorageRecord::Event(ev) => out.push(ev.clone()),
                StorageRecord::Packed(row, kind) => out.extend(expand(row, *kind).expect("展开")),
            }
        }
        out
    }

    /// 契约 1:pack → expand 逐字节等于原事件(seq/time/type/data/ignorable)
    #[test]
    fn round_trip_is_byte_exact() {
        let src = vec![
            reasoning(1, 1000, "The"),
            reasoning(2, 1001, " user"),
            reasoning(3, 1003, " asks"),
            chunk(4, 1010, "你好"),
            reasoning(5, 1020, "x"),
            EventEnvelope::new("user/message", 1030, json!({ "content": "next" })),
        ];
        let packed = pack(&src);
        // 3 连续 reasoning → 1 行;chunk 单条;reasoning 单条;user 透传
        assert_eq!(packed.len(), 4, "3-run 成一行,其余透传: {packed:?}");
        let back = expanded(&packed);
        assert_eq!(back, src, "逐字节还原");
    }

    /// 契约 2a:run < MIN_RUN 不打包
    #[test]
    fn short_runs_pass_through() {
        let src = vec![reasoning(1, 1, "a"), reasoning(2, 2, "b")];
        let packed = pack(&src);
        assert!(
            packed.iter().all(|r| matches!(r, StorageRecord::Event(_))),
            "两条不打包: {packed:?}"
        );
        assert_eq!(expanded(&packed), src);
    }

    /// 契约 2b:seq 断裂 / 类型切换 → 拆行
    #[test]
    fn runs_split_on_gaps_and_kind() {
        let mut gap = reasoning(9, 3, "c");
        gap.seq = 9; // 断裂
        let src = vec![reasoning(1, 1, "a"), reasoning(2, 2, "b"), gap];
        let packed = pack(&src);
        assert_eq!(packed.len(), 3, "seq 断裂拆行: {packed:?}");
        assert_eq!(expanded(&packed), src);

        let mixed = vec![
            reasoning(1, 1, "a"),
            reasoning(2, 2, "b"),
            reasoning(3, 3, "c"),
            chunk(4, 4, "d"),
            chunk(5, 5, "e"),
            chunk(6, 6, "f"),
        ];
        let packed = pack(&mixed);
        assert_eq!(packed.len(), 2, "类型切换拆行");
        assert_eq!(expanded(&packed), mixed);
    }

    /// 契约 3:形态不完全匹配 → 逐字透传(丢压缩不丢数据)
    #[test]
    fn non_whitelisted_shapes_pass_through_verbatim() {
        let extra_key = EventEnvelope {
            data: json!({ "text": "a", "extra": 1 }),
            ..reasoning(1, 1, "unused")
        };
        let non_string = EventEnvelope {
            data: json!({ "text": 42 }),
            ..reasoning(2, 2, "unused")
        };
        let non_ignorable = EventEnvelope::new("assistant/reasoning", 3, json!({ "text": "c" }));
        let attributed = EventEnvelope {
            source_event_seqs: Some(vec![1]),
            ..reasoning(4, 4, "d")
        };
        let other_type = EventEnvelope::new("audit/call", 5, json!({ "text": "e" }));
        let src = vec![extra_key, non_string, non_ignorable, attributed, other_type];
        let packed = pack(&src);
        assert!(
            packed.iter().all(|r| matches!(r, StorageRecord::Event(_))),
            "全部透传: {packed:?}"
        );
        assert_eq!(expanded(&packed), src);
    }

    /// 契约 4:坏行 fail-loud
    #[test]
    fn malformed_rows_error_loudly() {
        let row = ChunkRow {
            seq0: u64::MAX,
            time0: 0,
            dt: vec![],
            texts: vec!["a".into(), "b".into()],
        };
        // seq0 + 成员数溢出
        assert!(expand(&row, DeltaKind::Reasoning).is_err(), "seq 溢出");
        // dt 长度不符
        let row = ChunkRow {
            seq0: 1,
            time0: 0,
            dt: vec![1, 2],
            texts: vec!["a".into(), "b".into()],
        };
        assert!(expand(&row, DeltaKind::Chunk).is_err(), "dt 长度不符");
        // texts 空
        let row = ChunkRow {
            seq0: 1,
            time0: 0,
            dt: vec![],
            texts: vec![],
        };
        assert!(expand(&row, DeltaKind::Reasoning).is_err(), "texts 空");
        // time 越界
        let row = ChunkRow {
            seq0: 1,
            time0: i64::MAX,
            dt: vec![1],
            texts: vec!["a".into(), "b".into()],
        };
        assert!(expand(&row, DeltaKind::Reasoning).is_err(), "time 越界");
    }

    /// JSON 往返:打包行序列化 → 反序列化 → expand 仍逐字节
    #[test]
    fn row_json_round_trip() {
        let src = vec![
            reasoning(14, 1_790_656_207_262, "The"),
            reasoning(15, 1_790_656_207_263, " user"),
            reasoning(16, 1_790_656_207_265, " asks"),
        ];
        let packed = pack(&src);
        let StorageRecord::Packed(row, kind) = &packed[0] else {
            panic!("应为打包行");
        };
        assert_eq!(*kind, DeltaKind::Reasoning);
        assert_eq!(row.seq0, 14);
        assert_eq!(row.texts, vec!["The", " user", " asks"]);
        assert_eq!(row.dt, vec![1, 2]);
        let line = serde_json::to_string(&row).expect("序列化");
        let back: ChunkRow = serde_json::from_str(&line).expect("反序列化");
        assert_eq!(&back, row);
        assert_eq!(expand(&back, *kind).expect("展开"), src);
    }

    /// 墙钟回拨:dt 为负照样精确还原
    #[test]
    fn negative_time_gaps_round_trip() {
        let src = vec![
            reasoning(1, 1000, "a"),
            reasoning(2, 999, "b"),
            reasoning(3, 997, "c"),
        ];
        let packed = pack(&src);
        let StorageRecord::Packed(row, _) = &packed[0] else {
            panic!("应为打包行");
        };
        assert_eq!(row.dt, vec![-1, -2]);
        assert_eq!(expanded(&packed), src);
    }
}
