//! 内存事件日志:seq 连续的运行时强制 + 查询 + 快照(重放底座)。
//!
//! 对应 WIT `liuma:session/event-log`;wasm 组件导出经本模块实现。
//! 追加式:事件一经 append 不可变,重放确定性由此而来(时钟经注入,信封即纯数据)。

use thiserror::Error;

use crate::chunk_rows::{self, ChunkRow, DeltaKind};
use crate::envelope::{EnvelopeError, EventEnvelope, SESSION_FORMAT_VERSION, decode_envelope};

/// 日志操作错误
#[derive(Debug, Error, PartialEq)]
pub enum LogError {
    /// seq 不连续
    #[error("session event seq {actual} is not contiguous; expected {expected}")]
    NotContiguous {
        /// 实际收到的 seq
        actual: u64,
        /// 期望的 seq
        expected: u64,
    },
    /// 持久化失败(sink 写盘错误;事件不入内存)
    #[error("session event persistence failed: {0}")]
    Durability(String),
    /// 信封层错误
    #[error(transparent)]
    Envelope(#[from] EnvelopeError),
}

/// 持久化汇:append 时在**调用方日志锁内**被调,把带 seq 的信封写入
/// 介质。返回 Err = 落盘失败(事件整体被拒,内存/介质不分叉)。
/// 逐事件回调;delta 连续段的落盘打包由实现自行负责(见
/// `liuma_host::JsonlBackend` 的写侧打包器)
pub type DurabilitySink = Box<dyn Fn(&EventEnvelope) -> Result<(), String> + Send>;

/// 内存追加式事件日志:seq 连续运行时强制 + 查询 + 快照(重放底座)。
///
/// `next_seq` 从 1 起;`append` 拒绝不连续事件(fail-fast 而非静默跳号)。
///
/// 内存表示:**连续同型 delta 事件收进一行打包行**(实测同一会话日志,
/// 信封表示 297.8 MB → 打包 36.4 MB,8.2×),其余一条。对外接口语义与
/// 「逐条事件」完全一致——[`Self::iter`] 展开行、[`Self::get`] 经
/// seq→记录下标映射 O(1) 定位、[`Self::query`]/[`Self::snapshot`] 走
/// 同一展开面。调用方无感。
#[derive(Default)]
pub struct EventLog {
    /// 权威存储:普通事件一条、delta 连续段一行(行内成员 seq 连续)
    records: Vec<LogRecord>,
    /// seq(从 0 起 = 下标偏移)→ `records` 下标;每成员一项,O(1) 定位
    index: Vec<u32>,
    sink: Option<DurabilitySink>,
}

/// 日志内的一条权威记录
#[derive(Debug, Clone)]
enum LogRecord {
    /// 普通事件
    Event(Box<EventEnvelope>),
    /// 打包行;成员 k = seq `row.seq0 + k`、片段 `row.texts[k]`
    Packed(Box<ChunkRow>, DeltaKind),
}

/// 行内第 k 个成员的时间戳(k < texts.len())
fn member_time(row: &ChunkRow, k: usize) -> i64 {
    let mut time = row.time0;
    for gap in &row.dt[..k] {
        time += gap;
    }
    time
}

impl LogRecord {
    /// 本记录承载的成员数
    fn members(&self) -> u64 {
        match self {
            LogRecord::Event(_) => 1,
            LogRecord::Packed(row, _) => row.texts.len() as u64,
        }
    }

    /// 行内第 k 个成员展开为事件(k < members)
    fn expand_member(&self, k: u64) -> Option<EventEnvelope> {
        match self {
            LogRecord::Event(ev) => (k == 0).then(|| (**ev).clone()),
            LogRecord::Packed(row, kind) => {
                let text = row.texts.get(k as usize)?;
                let mut time = row.time0;
                for gap in &row.dt[..k as usize] {
                    time += gap;
                }
                let field = match kind {
                    DeltaKind::Reasoning => "text",
                    DeltaKind::Chunk => "delta",
                };
                let ty = match kind {
                    DeltaKind::Reasoning => "assistant/reasoning",
                    DeltaKind::Chunk => "assistant/chunk",
                };
                let mut ev =
                    EventEnvelope::new_ignorable(ty, time, serde_json::json!({ field: text }));
                ev.seq = row.seq0 + k;
                Some(ev)
            }
        }
    }
}

/// 展开迭代器:`iter()` 的返回面。跨记录线性推进,逐成员产出逻辑事件;
/// 双端(`.rev()` 供「最近一条」类查询)。
#[derive(Default)]
pub struct EventIter<'a> {
    records: &'a [LogRecord],
    /// 前向:当前记录下标
    record_ix: usize,
    /// 前向:记录内成员下标
    member_ix: u64,
    /// 反向:游标所在记录下标(初始 = 末条;`usize::MAX` = 全部耗尽)
    back_record_ix: usize,
    /// 反向:本行**已产出到的成员下标**(排他,已产出区间 = `[back_member_ix,
    /// members)`);`usize::MAX` = 哨兵,尚未开始产出本行(首个待产出 = 末位)
    back_member_ix: usize,
}

impl<'a> EventIter<'a> {
    /// 前向与反向游标是否相遇(全部产出完毕)。
    ///
    /// - 前向待产出 = `record_ix` 行的 `member_ix`;
    /// - 反向待产出 = `back_record_ix` 行的 `back_member_ix - 1`
    ///   (哨兵 `usize::MAX` = 尚未开始本行 → 待产出 = 末位成员)。
    ///
    /// 相遇 ⇔ 前向待产出位置在反向待产出位置之后(或反向已越过下界)。
    fn met(&self) -> bool {
        if self.back_record_ix >= self.records.len() {
            return true; // 反向全部产出完毕
        }
        match self.record_ix.cmp(&self.back_record_ix) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => {
                // 同一行:哨兵态(未开始)必不相遇(反向还有末位待产出);
                // 否则前向待产出 ≥ 反向已产出下界即相遇
                self.back_member_ix != usize::MAX && self.member_ix as usize >= self.back_member_ix
            }
        }
    }
}

impl<'a> Iterator for EventIter<'a> {
    type Item = EventEnvelope;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.met() {
                return None;
            }
            let rec = self.records.get(self.record_ix)?;
            match rec.expand_member(self.member_ix) {
                Some(ev) => {
                    self.member_ix += 1;
                    if self.member_ix >= rec.members() {
                        self.record_ix += 1;
                        self.member_ix = 0;
                    }
                    return Some(ev);
                }
                None => {
                    self.record_ix += 1;
                    self.member_ix = 0;
                }
            }
        }
    }
}

impl DoubleEndedIterator for EventIter<'_> {
    fn next_back(&mut self) -> Option<EventEnvelope> {
        loop {
            if self.met() {
                return None;
            }
            let rec = self.records.get(self.back_record_ix)?;
            let members = rec.members() as usize;
            if members == 0 {
                // 空记录:跳过(前移一条;越下界即耗尽)
                if self.back_record_ix == 0 {
                    return None;
                }
                self.back_record_ix -= 1;
                continue;
            }
            // 哨兵(尚未开始本行)→ 从末位进入
            if self.back_member_ix == usize::MAX {
                self.back_member_ix = members;
            }
            if self.back_member_ix == 0 {
                // 本行反向耗尽:退到前一条(哨兵态)
                if self.back_record_ix == 0 {
                    return None;
                }
                self.back_record_ix -= 1;
                self.back_member_ix = usize::MAX;
                continue;
            }
            self.back_member_ix -= 1;
            let ev = rec.expand_member(self.back_member_ix as u64)?;
            return Some(ev);
        }
    }
}

impl std::fmt::Debug for EventLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventLog")
            .field("high_water", &self.high_water())
            .field("durability", &self.sink.is_some())
            .finish_non_exhaustive()
    }
}

impl EventLog {
    /// 空日志
    pub fn new() -> Self {
        Self::default()
    }

    /// 装配持久化汇(替换语义;attach 装配点调用一次)
    pub fn set_durability_sink(&mut self, sink: DurabilitySink) {
        self.sink = Some(sink);
    }

    /// 是否已装配持久化汇
    pub fn has_durability(&self) -> bool {
        self.sink.is_some()
    }

    /// 下一个期望 seq(已 append 数 + 1;seq 从 1 起)
    pub fn next_seq(&self) -> u64 {
        self.index.len() as u64 + 1
    }

    /// 高水位(最新已 append seq)
    pub fn high_water(&self) -> u64 {
        self.index.len() as u64
    }

    /// 追加事件:seq 为 0 时自动分配,非 0 时必须与期望连续。
    ///
    /// 内存:与尾部打包行同型且 seq 连续 → 合并(单事件行凑满
    /// [`chunk_rows::MIN_RUN`] 原位并成一行);其余追加一条。
    /// 持久化汇:逐事件回调(落盘打包由 sink 实现负责,见
    /// [`DurabilitySink`]);两处布局规则一致,`load` 读回等价
    pub fn append(&mut self, mut ev: EventEnvelope) -> Result<u64, LogError> {
        let expected = self.next_seq();
        if ev.seq == 0 {
            ev.seq = expected;
        } else if ev.seq != expected {
            return Err(LogError::NotContiguous {
                actual: ev.seq,
                expected,
            });
        }
        // 尾部合并判定:同型 delta、行成员 seq 恰接在 expected 前
        let merged = match (chunk_rows::classify(&ev), self.records.last_mut()) {
            (Some(k), Some(LogRecord::Packed(row, row_kind)))
                if *row_kind == k && row.seq0 + row.texts.len() as u64 == expected =>
            {
                let prev_k = row.texts.len() - 1;
                let prev_time = member_time(row, prev_k);
                (prev_time.checked_sub(0), ev.time.checked_sub(prev_time))
                    .1
                    .map(|gap| {
                        let f = match k {
                            DeltaKind::Reasoning => "text",
                            DeltaKind::Chunk => "delta",
                        };
                        row.dt.push(gap);
                        row.texts.push(
                            ev.data
                                .get(f)
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                        );
                        self.index.push(self.index.last().copied().unwrap_or(0));
                        true
                    })
                    .unwrap_or(false)
            }
            (Some(k), Some(LogRecord::Event(pending)))
                if chunk_rows::classify(pending) == Some(k) && pending.seq + 1 == expected =>
            {
                // 单事件行 + 本条 = 2 条:仍不足 MIN_RUN,保持两条独立单事件行?
                // 否——先合并成行(行可从 2 条起步),凑满 MIN_RUN 前照常展开
                let prev_time = pending.time;
                let gap = match ev.time.checked_sub(prev_time) {
                    Some(g) => g,
                    None => {
                        return Err(LogError::NotContiguous {
                            actual: ev.seq,
                            expected, // 占位:time 回退不可合并,走单事件路径
                        });
                    }
                };
                let f = match k {
                    DeltaKind::Reasoning => "text",
                    DeltaKind::Chunk => "delta",
                };
                let row = ChunkRow {
                    seq0: pending.seq,
                    time0: pending.time,
                    dt: vec![gap],
                    texts: vec![
                        pending
                            .data
                            .get(f)
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        ev.data
                            .get(f)
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                    ],
                };
                let last_ix = self.records.len() - 1;
                self.records[last_ix] = LogRecord::Packed(Box::new(row), k);
                self.index.push(last_ix as u32);
                true
            }
            _ => false,
        };
        let _ = merged;
        if let Some(sink) = &self.sink {
            sink(&ev).map_err(LogError::Durability)?;
        }
        if !merged {
            self.index.push(self.records.len() as u32);
            self.records.push(LogRecord::Event(Box::new(ev)));
        }
        Ok(expected)
    }

    /// 按 seq 取事件(seq 从 1 起;O(1) 经 seq→记录下标映射定位)
    pub fn get(&self, seq: u64) -> Option<EventEnvelope> {
        let ix = *self.index.get(seq.checked_sub(1)? as usize)?;
        self.records
            .get(ix as usize)?
            .expand_member(seq - 1 - self.record_base(ix))
    }

    /// 记录 `ix` 的首成员 seq(= 其前全部记录的成员数 + 1)
    fn record_base(&self, ix: u32) -> u64 {
        self.records[..ix as usize]
            .iter()
            .map(LogRecord::members)
            .sum::<u64>()
    }

    /// 按类型过滤(空过滤 = 全部;返回展开后的克隆——打包行成员按需物化)
    pub fn query(&self, type_filter: Option<&str>) -> Vec<EventEnvelope> {
        self.iter()
            .filter(|ev| type_filter.is_none_or(|t| ev.r#type == t))
            .collect()
    }

    /// 最近一条「类型匹配且谓词成立」的逻辑事件(反向扫;`.rev().find()`
    /// 的高频形态——谓词失败继续向前,不做尾部一次定型)。返回展开后的克隆
    pub fn last_matching(
        &self,
        ty: &str,
        pred: impl Fn(&EventEnvelope) -> bool,
    ) -> Option<EventEnvelope> {
        for rec in self.records.iter().rev() {
            match rec {
                // 打包行成员皆同型(delta 类),首成员类型即全体
                LogRecord::Packed(row, kind) => {
                    let ty_of_row = match kind {
                        DeltaKind::Reasoning => "assistant/reasoning",
                        DeltaKind::Chunk => "assistant/chunk",
                    };
                    if ty_of_row != ty {
                        continue;
                    }
                    for k in (0..row.texts.len() as u64).rev() {
                        if let Some(ev) = rec.expand_member(k)
                            && pred(&ev)
                        {
                            return Some(ev);
                        }
                    }
                }
                LogRecord::Event(ev) => {
                    if ev.r#type == ty && pred(ev) {
                        return Some((**ev).clone());
                    }
                }
            }
        }
        None
    }

    /// 最近一条匹配类型的逻辑事件(反向扫;返回展开后的克隆)
    pub fn last_of(&self, ty: &str) -> Option<EventEnvelope> {
        self.last_matching(ty, |_| true)
    }

    /// 按类型**定向收集**(只展开命中的事件)。恢复/快照类调用方
    /// (instructions/skill/权限快照/投影恢复)此前整表 owned 展开
    /// ——46 万事件会话 ≈ 379MB 瞬时分配,而它们各自只消费几种事件
    /// 类型(user/message 子集只有千余条)。与
    /// `iter().filter(type ∈ types)` 逐字段一致(差分锁在测试)
    pub fn collect_of_types(&self, types: &[&str]) -> Vec<EventEnvelope> {
        let mut out = Vec::new();
        for rec in &self.records {
            match rec {
                LogRecord::Event(ev) => {
                    if types.contains(&ev.r#type.as_str()) {
                        out.push((**ev).clone());
                    }
                }
                LogRecord::Packed(row, kind) => {
                    // delta 类型在收集集内才展开(常规恢复面不含 delta,
                    // 整行跳过 = 零展开成本)
                    let ty = match kind {
                        DeltaKind::Reasoning => "assistant/reasoning",
                        DeltaKind::Chunk => "assistant/chunk",
                    };
                    if types.contains(&ty) {
                        for k in 0..row.texts.len() as u64 {
                            if let Some(ev) = rec.expand_member(k) {
                                out.push(ev);
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// 尾窗边界定位(history 分页):从尾向头数第 `max_messages` 条
    /// surface 消息(user/message 排除注入 / assistant/message)的 seq,
    /// 0 = 到头。O(记录数)零展开——打包行成员皆 delta,不是 surface
    /// 消息,整行跳过。语义与 `liuma_core::translate::page_cut` 逐字节
    /// 一致(切片版差分锁在 liuma-core)
    pub fn page_cut(&self, before_seq: Option<u64>, max_messages: usize) -> u64 {
        let mut count = 0usize;
        for rec in self.records.iter().rev() {
            let ev = match rec {
                LogRecord::Event(ev) => ev.as_ref(),
                LogRecord::Packed(..) => continue,
            };
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

    /// 全部逻辑事件(跨打包行顺序展开;调用方无感)
    pub fn iter(&self) -> EventIter<'_> {
        EventIter {
            records: &self.records,
            record_ix: 0,
            member_ix: 0,
            back_record_ix: self.records.len().wrapping_sub(1),
            back_member_ix: usize::MAX,
        }
    }

    /// 从指定 seq 起的展开迭代器(seq 从 1 起;越界 = 空迭代)。
    /// O(1) 定位:经 seq→记录下标映射直入,供「续扫」类消费方(轨迹
    /// 补喂分批)避免从头展开或整表物化
    pub fn iter_from(&self, seq: u64) -> EventIter<'_> {
        let (record_ix, member_ix) = match seq
            .checked_sub(1)
            .and_then(|i| self.index.get(i as usize).copied())
        {
            Some(ix) => (ix as usize, seq - 1 - self.record_base(ix)),
            None => (self.records.len(), 0),
        };
        EventIter {
            records: &self.records,
            record_ix,
            member_ix,
            back_record_ix: self.records.len().wrapping_sub(1),
            back_member_ix: usize::MAX,
        }
    }

    /// 借用式逐事件折叠(近零分配):普通行直接借用,打包成员展开进
    /// 复用缓冲(每行一次小分配,成员文本就地覆写)。热路径 fold
    /// (直播预热/统计/锚点/冷折叠)用——owned [`Self::iter`] 的逐条
    /// 克隆在长会话下是数百 MB 级瞬时分配。
    ///
    /// 注意:`f` 收到的引用只在当次回调内有效,**不得留存**
    pub fn for_each(&self, f: impl FnMut(&EventEnvelope)) {
        self.for_each_from(1, f);
    }

    /// [`Self::for_each`] 的定位形态:从指定 seq 起折叠(seq 从 1 起;
    /// 越界 = 空折叠)。续扫类消费方(轨迹补喂)用——owned
    /// [`Self::iter_from`] 免不了逐条克隆,借用直喂才是零展开
    pub fn for_each_from(&self, seq: u64, f: impl FnMut(&EventEnvelope)) {
        self.for_each_range(seq, None, f);
    }

    /// [`Self::for_each_from`] 的区间形态:`[from, until)` 半开区间
    /// (until = None 即到尾;until 排他,与 history 窗口语义一致)
    pub fn for_each_range(&self, from: u64, until: Option<u64>, mut f: impl FnMut(&EventEnvelope)) {
        // O(1) 定位起始记录与成员(与 iter_from 同款解析)
        let (start_rec, skip_members) = match from
            .checked_sub(1)
            .and_then(|i| self.index.get(i as usize).copied())
        {
            Some(ix) => (ix as usize, (from - 1 - self.record_base(ix)) as usize),
            None if from <= 1 => (0, 0),
            None => return, // 越过末尾:空折叠
        };
        // 空区间早退:until ≤ from 即 [from, until) 为空
        if until.is_some_and(|u| u <= from) {
            return;
        }
        // 复用缓冲:成员字段(类型/seq/time)就地覆写,文本 clear+push
        // 复用容量。surface_op/source_event_seqs 打包守卫保证恒 None
        let mut scratch = EventEnvelope {
            r#type: String::new(),
            seq: 0,
            time: 0,
            data: serde_json::Value::Null,
            surface_op: None,
            source_event_seqs: None,
            ignorable: true,
        };
        for (ri, rec) in self.records.iter().enumerate().skip(start_rec) {
            match rec {
                // 普通事件(定位行命中时偏移恒 0:单成员记录)。事件序
                // 升序,首个越界即整段结束
                LogRecord::Event(ev) => {
                    if until.is_some_and(|u| ev.seq >= u) {
                        return;
                    }
                    f(ev)
                }
                LogRecord::Packed(row, kind) => {
                    let (ty, field) = match kind {
                        DeltaKind::Reasoning => ("assistant/reasoning", "text"),
                        DeltaKind::Chunk => ("assistant/chunk", "delta"),
                    };
                    scratch.r#type = ty.to_string();
                    scratch.data = serde_json::json!({ field: String::new() });
                    let from = if ri == start_rec { skip_members } else { 0 };
                    // time 先推进到起始成员(time0 + Σdt[..from])
                    let mut time = row.time0;
                    for g in 0..from {
                        time += row.dt[g];
                    }
                    for k in from..row.texts.len() {
                        if k > from {
                            time += row.dt[k - 1];
                        }
                        scratch.seq = row.seq0 + k as u64;
                        if until.is_some_and(|u| scratch.seq >= u) {
                            return;
                        }
                        scratch.time = time;
                        if let Some(serde_json::Value::String(s)) = scratch.data.get_mut(field) {
                            s.clear();
                            s.push_str(&row.texts[k]);
                        }
                        f(&scratch);
                    }
                }
            }
        }
    }

    /// 快照:头部 + 逻辑事件数组(外部契约 = 逐条事件;格式版本在头部)。
    pub fn snapshot(&self) -> serde_json::Value {
        let events: Vec<EventEnvelope> = self.iter().collect();
        serde_json::json!({
            "version": SESSION_FORMAT_VERSION,
            "events": events,
        })
    }

    /// 从快照重建(读取方守卫:每个事件都过 [`decode_envelope`])。
    pub fn from_snapshot(raw: &serde_json::Value) -> Result<Self, LogError> {
        let mut log = Self::new();
        for raw_ev in raw["events"].as_array().unwrap_or(&Vec::new()) {
            let ev = decode_envelope(raw_ev)?;
            log.append(ev)?;
        }
        Ok(log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    fn user_msg(content: &str) -> EventEnvelope {
        EventEnvelope::new("user/message", 0, json!({ "content": content }))
    }

    #[test]
    fn append_assigns_contiguous_seq() {
        let mut log = EventLog::new();
        assert_eq!(log.append(user_msg("a")).unwrap(), 1);
        assert_eq!(log.append(user_msg("b")).unwrap(), 2);
        assert_eq!(log.high_water(), 2);
    }

    #[test]
    fn append_rejects_gap() {
        let mut log = EventLog::new();
        let mut ev = user_msg("a");
        ev.seq = 7;
        assert_eq!(
            log.append(ev),
            Err(LogError::NotContiguous {
                actual: 7,
                expected: 1
            })
        );
    }

    #[test]
    fn snapshot_roundtrip_is_deterministic() {
        // 重放确定性:同日志两次「快照→重建」状态相等
        let mut log = EventLog::new();
        log.append(user_msg("a")).unwrap();
        log.append(EventEnvelope::new(
            "assistant/message",
            1,
            json!({"content": "b"}),
        ))
        .unwrap();
        let r1 = EventLog::from_snapshot(&log.snapshot()).unwrap();
        let r2 = EventLog::from_snapshot(&log.snapshot()).unwrap();
        assert_eq!(r1.snapshot(), r2.snapshot());
        assert_eq!(r1.snapshot(), log.snapshot());
    }

    #[test]
    fn rebuild_refuses_unknown_not_ignorable() {
        // 读取方守卫经 from_snapshot 生效:伪造含未知未标事件的快照被拒
        let bad = json!({
            "version": SESSION_FORMAT_VERSION,
            "events": [
                { "type": "user/message", "seq": 1, "time": 0, "data": {}, "ignorable": false },
                { "type": "evil/unknown", "seq": 2, "time": 0, "data": {}, "ignorable": false },
            ],
        });
        assert!(EventLog::from_snapshot(&bad).is_err());
    }

    #[test]
    fn query_filters_by_type() {
        let mut log = EventLog::new();
        log.append(user_msg("a")).unwrap();
        log.append(EventEnvelope::new(
            "assistant/message",
            1,
            json!({"content": "b"}),
        ))
        .unwrap();
        assert_eq!(log.query(Some("user/message")).len(), 1);
        assert_eq!(log.query(None).len(), 2);
    }

    /// 持久化汇:锁内被调,收到已定 seq 的信封(单写权威核心)
    #[test]
    fn durability_sink_receives_sequenced_envelope() {
        let seen: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let mut log = EventLog::new();
        log.set_durability_sink(Box::new(move |ev| {
            seen2.lock().unwrap().push(ev.seq);
            Ok(())
        }));
        log.append(user_msg("a")).unwrap();
        log.append(user_msg("b")).unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![1, 2], "sink 收到定序信封");
        assert!(log.has_durability());
    }

    /// 落盘失败 = 事件被拒(内存不落账);恢复后按期望 seq 续写
    #[test]
    fn durability_failure_rejects_event_atomically() {
        let ok = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let ok2 = ok.clone();
        let mut log = EventLog::new();
        log.set_durability_sink(Box::new(move |_ev| {
            if ok2.load(std::sync::atomic::Ordering::Relaxed) {
                Ok(())
            } else {
                Err("disk gone".into())
            }
        }));
        log.append(user_msg("a")).unwrap();
        ok.store(false, std::sync::atomic::Ordering::Relaxed);
        let err = log.append(user_msg("b")).unwrap_err();
        assert!(matches!(err, LogError::Durability(_)));
        assert_eq!(log.high_water(), 1, "失败事件不入内存");
        ok.store(true, std::sync::atomic::Ordering::Relaxed);
        log.append(user_msg("b")).unwrap();
        assert_eq!(log.high_water(), 2, "恢复后按期望 seq 续写");
    }

    /// 双线程并发 append(汇持文件写):文件行序 == seq 序。
    /// 单写权威行为锁——汇在调用方日志锁内执行,文件写被同一锁串行
    #[test]
    fn concurrent_append_file_order_matches_seq_order() {
        let dir = std::env::temp_dir().join(format!(
            "liuma-log-order-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.jsonl");
        let file = std::fs::File::create(&path).unwrap();
        let file = Arc::new(Mutex::new(file));
        let log = Arc::new(Mutex::new(EventLog::new()));
        {
            let f = file.clone();
            log.lock().unwrap().set_durability_sink(Box::new(move |ev| {
                use std::io::Write as _;
                let mut w = f.lock().unwrap();
                serde_json::to_writer(&mut *w, ev).unwrap();
                w.write_all(b"\n").unwrap();
                w.flush().unwrap();
                Ok(())
            }));
        }
        let handles: Vec<_> = (0..4)
            .map(|t| {
                let log = log.clone();
                std::thread::spawn(move || {
                    for i in 0..25 {
                        log.lock()
                            .unwrap()
                            .append(EventEnvelope::new(
                                "user/message",
                                0,
                                serde_json::json!({ "content": format!("t{t}-{i}") }),
                            ))
                            .unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let seqs: Vec<u64> = text
            .lines()
            .map(|l| {
                let v: serde_json::Value = serde_json::from_str(l).unwrap();
                v["seq"].as_u64().unwrap()
            })
            .collect();
        assert_eq!(seqs.len(), 100);
        for (ix, s) in seqs.iter().enumerate() {
            assert_eq!(*s, (ix + 1) as u64, "文件行序应等于 seq 序");
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod iter_tests {
    use super::*;
    use serde_json::json;

    fn reasoning(seq: u64, time: i64, text: &str) -> EventEnvelope {
        let mut ev =
            EventEnvelope::new_ignorable("assistant/reasoning", time, json!({ "text": text }));
        ev.seq = seq;
        ev
    }

    fn user(seq: u64, content: &str) -> EventEnvelope {
        let mut ev = EventEnvelope::new("user/message", 0, json!({ "content": content }));
        ev.seq = seq;
        ev
    }

    fn seeded() -> EventLog {
        let mut log = EventLog::new();
        for (i, t) in ["a", "b", "c", "d", "e"].iter().enumerate() {
            log.append(reasoning(i as u64 + 1, i as i64, t)).unwrap();
        }
        log.append(user(6, "x")).unwrap();
        log
    }

    /// 双端展开:正反两向都产出全部逻辑事件,顺序正确
    #[test]
    fn iter_is_double_ended() {
        let log = seeded();
        let fwd: Vec<u64> = log.iter().map(|e| e.seq).collect();
        assert_eq!(fwd, vec![1, 2, 3, 4, 5, 6]);
        let bwd: Vec<u64> = log.iter().rev().map(|e| e.seq).collect();
        assert_eq!(bwd, vec![6, 5, 4, 3, 2, 1]);
        // 混合消费(Iterator::scan 类组合子依赖双向)
        let mixed: Vec<u64> = {
            let mut it = log.iter();
            let first = it.next().map(|e| e.seq);
            let last = it.next_back().map(|e| e.seq);
            [first, last].into_iter().flatten().collect()
        };
        assert_eq!(mixed, vec![1, 6]);
    }

    /// get(seq) O(1) 定位:打包行成员与普通事件混合取回
    #[test]
    fn get_hits_packed_members() {
        let log = seeded();
        for seq in 1u64..=6 {
            let ev = log.get(seq).unwrap_or_else(|| panic!("get {seq}"));
            assert_eq!(ev.seq, seq);
        }
        assert_eq!(log.get(0), None);
        assert_eq!(log.get(7), None);
        // 内容逐字节
        assert_eq!(log.get(3).unwrap().data["text"], "c");
        assert_eq!(log.get(6).unwrap().data["content"], "x");
    }

    /// last_of:跨打包行反向找最近一条
    #[test]
    fn last_of_finds_latest_match() {
        let mut log = seeded();
        log.append(user(7, "y")).unwrap();
        let last = log.last_of("user/message").expect("命中");
        assert_eq!(last.seq, 7);
        assert_eq!(last.data["content"], "y");
        assert_eq!(log.last_of("nonexistent/type"), None);
    }

    /// last_matching:谓词失败继续向前(guard/fold 的「最后一条真实用户
    /// 消息」语义——尾部注入消息不算,不能尾部一次定型)
    #[test]
    fn last_matching_skips_failing_tail() {
        let mut log = seeded();
        log.append(EventEnvelope::new(
            "user/message",
            0,
            json!({ "content": "[injected]", "source": { "kind": "plugin" } }),
        ))
        .unwrap();
        let is_real =
            |ev: &EventEnvelope| ev.data["source"]["kind"].as_str().unwrap_or("user") == "user";
        let hit = log
            .last_matching("user/message", is_real)
            .expect("跳过注入消息,命中更早的真实消息");
        assert_eq!(hit.seq, 6);
        assert_eq!(hit.data["content"], "x");
        // 全部不满足 → None(不打折成尾部)
        assert_eq!(log.last_matching("user/message", |_| false), None);
        // last_of = 无谓词形态,仍取尾部(seq 7 注入消息)
        assert_eq!(log.last_of("user/message").unwrap().seq, 7);
    }

    /// 差分锁(读侧布局盲):对抗流逐条 append(孤条 / 2 条行 / 长行 /
    /// 精确键守卫断连 / surface_op 断连混排),**每步**全读面
    /// (get / iter 正反 / last_of / snapshot)与未打包镜像逐字段相等
    #[test]
    fn reads_match_unpacked_mirror_on_adversarial_stream() {
        let d_reasoning = |time: i64, text: &str| {
            EventEnvelope::new_ignorable("assistant/reasoning", time, json!({ "text": text }))
        };
        let d_chunk = |time: i64, text: &str| {
            EventEnvelope::new_ignorable("assistant/chunk", time, json!({ "delta": text }))
        };
        let mut cases: Vec<EventEnvelope> = vec![
            // 孤条 delta(尾随非 delta → 永远单条)
            d_reasoning(0, "lone"),
            // 2 连(合并成 2 条行)
            d_reasoning(1, "a"),
            d_reasoning(2, "b"),
            // 非 delta 断连
            user(0, "break"),
            // 5 连 chunk(长行)
            d_chunk(10, "c1"),
            d_chunk(11, "c2"),
            d_chunk(12, "c3"),
            d_chunk(13, "c4"),
            d_chunk(14, "c5"),
            // 带额外键的 reasoning:精确键守卫 → 不打包且断连
            EventEnvelope::new_ignorable(
                "assistant/reasoning",
                20,
                json!({ "text": "x", "extra": 1 }),
            ),
            // 2 连(新行)
            d_reasoning(21, "p"),
            d_reasoning(22, "q"),
            // 带 surface_op 的 delta:不可打包,断连
            {
                let mut ev = d_reasoning(30, "surf");
                ev.surface_op = Some("op-1".into());
                ev
            },
            // 3 连 chunk
            d_chunk(31, "d1"),
            d_chunk(32, "d2"),
            d_chunk(33, "d3"),
            // 注入 user 消息
            EventEnvelope::new(
                "user/message",
                40,
                json!({ "content": "[injected]", "source": { "kind": "plugin" } }),
            ),
            // 尾部孤条 delta
            d_reasoning(99, "tail"),
        ];
        let mut log = EventLog::new();
        let mut mirror: Vec<EventEnvelope> = Vec::new();
        for (i, ev) in cases.drain(..).enumerate() {
            let mut ev = ev;
            ev.seq = i as u64 + 1;
            log.append(ev.clone()).unwrap();
            mirror.push(ev);
            // get:全量逐条
            for (s, want) in mirror.iter().enumerate() {
                assert_eq!(
                    log.get(s as u64 + 1),
                    Some(want.clone()),
                    "get {} 漂移",
                    s + 1
                );
            }
            // iter:正反两向
            assert_eq!(log.iter().collect::<Vec<_>>(), mirror, "iter 前向漂移");
            assert_eq!(
                log.iter().rev().collect::<Vec<_>>(),
                mirror.iter().rev().cloned().collect::<Vec<_>>(),
                "iter 反向漂移"
            );
            // last_of:三型各验
            for ty in ["assistant/reasoning", "assistant/chunk", "user/message"] {
                let want = mirror.iter().rev().find(|e| e.r#type == ty).cloned();
                assert_eq!(log.last_of(ty), want, "last_of {ty} 漂移");
            }
            // snapshot 事件数组 = 镜像序列化(逐字节)
            assert_eq!(
                log.snapshot()["events"],
                serde_json::to_value(&mirror).unwrap(),
                "snapshot 漂移"
            );
        }
        // 布局卫:差分确经混排(有打包行、行长 >2、且有单条行)
        assert!(
            log.records
                .iter()
                .any(|r| matches!(r, LogRecord::Packed(row, _) if row.texts.len() > 2)),
            "应有长打包行"
        );
        assert!(
            log.records.iter().any(|r| matches!(r, LogRecord::Event(_))),
            "应有单条行"
        );
        // iter_from:任意起点 = iter() 同后缀;越界 = 空
        let full: Vec<EventEnvelope> = log.iter().collect();
        for start in [1u64, 2, 4, 6, 9, 14, 18, 19] {
            let suffix: Vec<EventEnvelope> = log.iter_from(start).collect();
            let want: Vec<EventEnvelope> = full.iter().skip(start as usize - 1).cloned().collect();
            assert_eq!(suffix, want, "iter_from({start}) 应与 iter() 同后缀");
        }
        // for_each:逐字段与 iter() 一致(借用式零分配折叠面)
        let mut seen: Vec<EventEnvelope> = Vec::new();
        log.for_each(|ev| seen.push(ev.clone()));
        assert_eq!(seen, full, "for_each 应与 iter() 逐字段一致");
        // for_each_from:任意起点 = iter() 同后缀;越界 = 空折叠
        for start in [1u64, 2, 4, 6, 9, 14, 18, 19] {
            let mut tail: Vec<EventEnvelope> = Vec::new();
            log.for_each_from(start, |ev| tail.push(ev.clone()));
            let want: Vec<EventEnvelope> = full.iter().skip(start as usize - 1).cloned().collect();
            assert_eq!(tail, want, "for_each_from({start}) 应与 iter() 同后缀");
        }
        // collect_of_types:与 iter().filter 同集;命中 delta 类型时含打包成员
        for types in [
            vec!["user/message"],
            vec!["user/message", "compaction/summary"],
            vec!["assistant/reasoning"],
            vec!["assistant/chunk", "user/message"],
        ] {
            let got = log.collect_of_types(&types);
            let want: Vec<EventEnvelope> = full
                .iter()
                .filter(|e| types.contains(&e.r#type.as_str()))
                .cloned()
                .collect();
            assert_eq!(got, want, "collect_of_types({types:?}) 与过滤集漂移");
        }
    }

    /// 快照 = 逻辑事件流(外部契约不变)
    #[test]
    fn snapshot_is_logical_events() {
        let log = seeded();
        let snap = log.snapshot();
        let events = snap["events"].as_array().expect("数组");
        assert_eq!(events.len(), 6, "打包行按成员展开");
        assert_eq!(events[2]["seq"], 3);
        assert_eq!(events[2]["data"]["text"], "c");
    }

    /// 重放一致性:快照 → from_snapshot 重建 → 逐事件等于原流
    #[test]
    fn snapshot_round_trip() {
        let log = seeded();
        let snap = log.snapshot();
        let rebuilt = EventLog::from_snapshot(&snap).expect("重建");
        let a: Vec<EventEnvelope> = log.iter().collect();
        let b: Vec<EventEnvelope> = rebuilt.iter().collect();
        assert_eq!(a, b);
    }
}
