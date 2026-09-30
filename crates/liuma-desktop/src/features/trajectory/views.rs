//! 轨迹视图(事件台账)。结构自上而下:工具栏(Duration/Turns/Calls/搜索)→ Overview
//! 时间线(3 轨道)→ 台账表 + 右侧检查器。行模型/交互语义 =
//! 台账表 / 时间线 / 工具栏三件套;
//! kind 色板派生自 dark token(常量处注释),骨架色用 theme.rs 现有项。

#![allow(non_snake_case)] // kind 色板取值 fn 保持原常量调用形态(随 theme 双盘)
use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::toolbar::Toolbar;
use gpui_kit::component::{Icon, IconName, Sizable as _, StyledExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Bounds, CursorStyle, Div, Entity, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Rgba, ScrollWheelEvent,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use liuma_core::trajectory::{TrajectoryRecord, TrajectoryRequest, TrajectoryUsage};

use crate::features::trajectory::{InspectTarget, TrajectoryView};
use crate::kits::i18n::t;
use crate::kits::icons::{LiumaIcon, fixed};
use crate::kits::theme;
use crate::shell::store::AppStore;

// ── kind 色板(dark 盘取定义色值;浅盘取白底可读变体)────────────
/// hex → Rgba(本地色板便捷构造)
fn rgb(hex: u32) -> Rgba {
    Rgba {
        r: ((hex >> 16) & 0xFF) as f32 / 255.0,
        g: ((hex >> 8) & 0xFF) as f32 / 255.0,
        b: (hex & 0xFF) as f32 / 255.0,
        a: 1.0,
    }
}
// ASSISTANT 紫:深盘取合成紫 0x9474BC,浅盘取 0x6E4FA3
fn ASSISTANT_VIOLET() -> Rgba {
    if theme::is_dark() {
        rgb(0x9474BC)
    } else {
        rgb(0x6E4FA3)
    }
}
// TTFT 弱紫 = 解码紫 54% 混卡片底(运行时混合,随盘反演)
fn TTFT_VIOLET() -> Rgba {
    mix(ASSISTANT_VIOLET(), theme::CARD(), 0.54)
}
// TOOL 琥珀:深盘 #DD8629,浅盘 #B45309
fn TOOL_AMBER() -> Rgba {
    if theme::is_dark() {
        rgb(0xDD8629)
    } else {
        rgb(0xB45309)
    }
}
// JSON 高亮(VSCode Dark+ / Light+):字符串值
fn JSON_STRING() -> Rgba {
    if theme::is_dark() {
        rgb(0xCE9178)
    } else {
        rgb(0xA31515)
    }
}
// JSON 高亮:数字
fn JSON_NUMBER() -> Rgba {
    if theme::is_dark() {
        rgb(0xB5CEA8)
    } else {
        rgb(0x098658)
    }
}
// JSON 树配色:键蓝 / 标点白 / 箭头灰
fn JSON_PROPERTY() -> Rgba {
    if theme::is_dark() {
        rgb(0x5DB0D7)
    } else {
        rgb(0x0451A5)
    }
}
fn JSON_PUNCT() -> Rgba {
    if theme::is_dark() {
        rgb(0xE8EAED)
    } else {
        rgb(0x3B3B3B)
    }
}
fn JSON_EXPANDER() -> Rgba {
    if theme::is_dark() {
        rgb(0x9AA0A6)
    } else {
        rgb(0x8E8E93)
    }
}

/// 颜色混合(t = a 的权重)
fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    Rgba {
        r: a.r * t + b.r * (1. - t),
        g: a.g * t + b.g * (1. - t),
        b: a.b * t + b.b * (1. - t),
        a: 1.0,
    }
}

/// kind → 台账标签文本
fn kind_label(kind: &str) -> std::borrow::Cow<'static, str> {
    match kind {
        "system" => t!("trajectory.kind_system"),
        "user" => t!("trajectory.kind_user"),
        "context" => t!("trajectory.kind_context"),
        "compacted" => t!("trajectory.kind_compacted"),
        "message" => t!("trajectory.kind_assistant"),
        "decision" => t!("trajectory.kind_decision"),
        _ => t!("trajectory.kind_tool"),
    }
}

/// kind → 标签配色(前景 + 15% 同色底)
fn kind_colors(kind: &str) -> (Rgba, Rgba) {
    match kind {
        // USER:business 蓝前景 + 蓝 15% 底
        "user" => (theme::BRAND(), mix(theme::BRAND(), theme::BASE(), 0.15)),
        // ASSISTANT:解码紫前景 + 紫 15% 底
        "message" => (
            ASSISTANT_VIOLET(),
            mix(ASSISTANT_VIOLET(), theme::BASE(), 0.15),
        ),
        // TOOL:琥珀前景 + 琥珀 15% 底
        "tool" => (TOOL_AMBER(), mix(TOOL_AMBER(), theme::BASE(), 0.15)),
        // CONTEXT:success 主色混灰前景 + 绿 15% 底
        "context" => (
            mix(theme::SUCCESS(), theme::CAPTION(), 0.32),
            mix(theme::SUCCESS(), theme::BASE(), 0.15),
        ),
        // DECISION:warn 前景 + warn 15% 底(建议面:读得到,不抢眼)
        "decision" => (theme::WARN(), mix(theme::WARN(), theme::BASE(), 0.15)),
        // SYSTEM / COMPACTED:中性
        _ => (theme::LABEL_2(), theme::DOCK()),
    }
}

/// kind → 时间线轨道(Input/Model/Tools)
fn lane_of(kind: &str) -> u8 {
    match kind {
        "system" | "user" | "context" => 0,
        "message" | "compacted" => 1,
        // 决策与工具同轨:守卫裁决就发生在调用执行那一刻
        _ => 2,
    }
}

/// 工具行文本拆分:text = "{name} {args_json}" → (name, args)
fn split_tool_text(text: &str) -> (&str, &str) {
    match text.split_once(' ') {
        Some((name, args)) => (name, args),
        None => (text, ""),
    }
}

// ── 格式化 ────────────────────────────────────────────────────

/// 千分位整数
fn thousands(n: i64) -> String {
    let neg = n < 0;
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if neg { format!("-{out}") } else { out }
}

/// 毫秒时长(千分位 ms)
fn fmt_ms(ms: i64) -> String {
    if ms <= 0 {
        "—".into()
    } else {
        format!("{} ms", thousands(ms))
    }
}

/// token 数千分位
fn fmt_tok(v: u64) -> String {
    thousands(v as i64)
}

/// epoch ms → 本地日期时间(Started 值格式:2026-08-18 22:59:59.493)
fn fmt_clock(ms: i64) -> String {
    if ms <= 0 {
        return "—".into();
    }
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
        .unwrap_or_else(|| "—".into())
}

/// Timing source 行值(有会话时间戳数据 = Session timestamps,
/// 否则 Not available)
fn timing_source(available: bool) -> String {
    if available {
        t!("trajectory.timing_session").into()
    } else {
        t!("trajectory.timing_na").into()
    }
}

// ── 台账行模型(折叠/摘要/边界重建)──────────────

/// 一行台账(渲染模型)
pub(crate) enum LedgerRow {
    /// 「加载更早」行(30px)
    LoadEarlier,
    /// Turn 折叠摘要行(20px)
    TurnSummary {
        turn: u64,
        steps: usize,
        tools: usize,
    },
    /// Calls 折叠摘要行(20px)
    CallSummary {
        message_index: u64,
        count: usize,
        names: Vec<String>,
    },
    /// 记录行(30px);turn_start 为过滤后重算的轮首。
    /// `rec_ix` = records 下标:行描述不持借用,虚拟列表按 index 解引用
    Record { rec_ix: usize, turn_start: bool },
}

impl LedgerRow {
    /// 行型(定高行:类型序列相同 = 总内容高不变,列表可零操作)
    fn kind(&self) -> u8 {
        match self {
            LedgerRow::LoadEarlier => 0,
            LedgerRow::TurnSummary { .. } => 1,
            LedgerRow::CallSummary { .. } => 2,
            LedgerRow::Record { .. } => 3,
        }
    }
}

/// 搜索命中(空格分词 AND、大小写不敏感)
fn search_hit(rec: &TrajectoryRecord, query: &str) -> bool {
    let tokens: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).collect();
    if tokens.is_empty() {
        return true;
    }
    let mut hay = format!(
        "{} {} {} {} ",
        rec.kind,
        rec.text,
        rec.result.as_deref().unwrap_or(""),
        rec.group,
    );
    if let Some(t) = rec.turn {
        hay.push_str(&format!("turn {t} "));
    }
    for f in [&rec.payload, &rec.output_detail, &rec.thinking_detail]
        .into_iter()
        .flatten()
    {
        hay.push(' ');
        hay.push_str(&f.to_lowercase());
    }
    let hay = hay.to_lowercase();
    tokens.iter().all(|t| hay.contains(t))
}

/// 折叠判定输入(渲染期从 store 快照)
struct CollapseState {
    all_turns: bool,
    turns: HashSet<u64>,
    all_calls: bool,
    calls: HashSet<u64>,
}

impl CollapseState {
    fn turn_collapsed(&self, turn: u64) -> bool {
        self.all_turns ^ self.turns.contains(&turn)
    }
    fn call_collapsed(&self, message_index: u64) -> bool {
        self.all_calls ^ self.calls.contains(&message_index)
    }
}

/// 可见记录 → 渲染行(搜索/时间线选区过滤 + turn/calls 折叠 + 轮首重算)
fn build_rows(
    records: &[TrajectoryRecord],
    visible: &[bool],
    collapse: &CollapseState,
) -> Vec<LedgerRow> {
    let mut rows: Vec<LedgerRow> = Vec::new();
    let mut seen_turns: HashSet<u64> = HashSet::new();
    let mut i = 0;
    while i < records.len() {
        if !visible[i] {
            i += 1;
            continue;
        }
        let rec = &records[i];
        let turn_start = rec.turn.is_some_and(|t| seen_turns.insert(t));
        // 轮折叠:保留首条,其余计步数
        if let Some(t) = rec.turn
            && turn_start
            && collapse.turn_collapsed(t)
        {
            let mut steps = 0usize;
            let mut tools = 0usize;
            let mut j = i + 1;
            while j < records.len() {
                let r = &records[j];
                if !visible[j] {
                    j += 1;
                    continue;
                }
                if r.turn != Some(t) {
                    break;
                }
                steps += 1;
                if r.kind == "tool" {
                    tools += 1;
                }
                j += 1;
            }
            rows.push(LedgerRow::Record {
                rec_ix: i,
                turn_start,
            });
            rows.push(LedgerRow::TurnSummary {
                turn: t,
                steps,
                tools,
            });
            i = j;
            continue;
        }
        // Calls 折叠:assistant 步消息后的连续工具行并入摘要
        if rec.kind == "message"
            && rec.group.starts_with("Step")
            && collapse.call_collapsed(rec.index)
        {
            let mut j = i + 1;
            let mut names: Vec<String> = Vec::new();
            let mut count = 0usize;
            while j < records.len() {
                let r = &records[j];
                if !visible[j] {
                    j += 1;
                    continue;
                }
                if r.kind != "tool" || r.group != rec.group {
                    break;
                }
                let (name, _) = split_tool_text(&r.text);
                let name = name.to_string();
                if !names.contains(&name) {
                    names.push(name);
                }
                count += 1;
                j += 1;
            }
            rows.push(LedgerRow::Record {
                rec_ix: i,
                turn_start,
            });
            if count > 0 {
                rows.push(LedgerRow::CallSummary {
                    message_index: rec.index,
                    count,
                    names,
                });
            }
            i = j;
            continue;
        }
        rows.push(LedgerRow::Record {
            rec_ix: i,
            turn_start,
        });
        i += 1;
    }
    rows
}

// ── 时间线投影(sequence/duration 双模式)──────

/// 一条投影条形(域归一化 0..1)
#[derive(Clone)]
struct TlSpan {
    x0: f64,
    x1: f64,
    record_index: u64,
    kind: String,
    is_error: bool,
    /// 助手条 TTFT 分界(条内 0..1;None = 单色)
    ttft_split: Option<f64>,
}

/// 投影:sequence = 等宽序列;duration = 按耗时、压缩 idle gap
fn build_spans(records: &[TrajectoryRecord], duration_mode: bool) -> Vec<TlSpan> {
    let entries: Vec<&TrajectoryRecord> =
        records.iter().filter(|r| r.started_at.is_some()).collect();
    if entries.is_empty() {
        return vec![];
    }
    let n = entries.len();
    if !duration_mode {
        return entries
            .iter()
            .enumerate()
            .map(|(i, r)| TlSpan {
                x0: i as f64 / n as f64,
                x1: (i + 1) as f64 / n as f64,
                record_index: r.index,
                kind: r.kind.clone(),
                is_error: r.is_error,
                ttft_split: assistant_ttft_split(r),
            })
            .collect();
    }
    let durs: Vec<f64> = entries
        .iter()
        .map(|r| r.time_seconds.unwrap_or(0.) * 1000.)
        .collect();
    let total: f64 = durs.iter().sum();
    if total <= 0. {
        return build_spans(records, false);
    }
    let mut acc = 0.;
    entries
        .iter()
        .zip(durs)
        .map(|(r, d)| {
            let x0 = acc / total;
            acc += d;
            TlSpan {
                x0,
                x1: acc / total,
                record_index: r.index,
                kind: r.kind.clone(),
                is_error: r.is_error,
                ttft_split: assistant_ttft_split(r),
            }
        })
        .collect()
}

/// 助手条 TTFT 分界 = ttft / 总时长(计时完整才有)
fn assistant_ttft_split(rec: &TrajectoryRecord) -> Option<f64> {
    if rec.kind != "message" {
        return None;
    }
    let ttft = rec.ttft_ms? as f64;
    let total = rec.time_seconds? * 1000.;
    if ttft <= 0. || total <= ttft {
        return None;
    }
    Some(ttft / total)
}

/// 时间线上的折叠带(域归一化 0..1)
#[derive(Clone)]
struct FoldBand {
    x0: f64,
    x1: f64,
    /// 遮蔽区间起点早于已载窗口(左端不从 0 假装完整)
    clipped: bool,
    /// 被点击时选中的台账行(那一次折叠的 compacted 记录)
    record_index: u64,
}

/// 折叠带投影:每次折叠遮蔽的 seq 区间 → 条形的 x 区间。
///
/// 映射复用 [`build_spans`] 的结果(sequence 等差 / duration 按耗时,
/// 两条路都不必各自再算一遍);区间端点落在窗口外时退到 0(左)/ 折叠行
/// 自身位置(右),并以 `clipped` 标注——「更早」是有信息量的,
/// 假装完整不是。
fn build_fold_bands(records: &[TrajectoryRecord], spans: &[TlSpan]) -> Vec<FoldBand> {
    let first_seq = records.first().map(|r| r.seq).unwrap_or(0);
    let x_of = |ix: u64| {
        spans
            .iter()
            .find(|sp| sp.record_index == ix)
            .map(|sp| (sp.x0, sp.x1))
    };
    let mut out = Vec::new();
    for rec in records.iter().filter(|r| r.kind == "compacted") {
        let Some(f) = rec.fold.as_ref() else {
            continue;
        };
        let covered: Vec<(f64, f64)> = records
            .iter()
            .filter(|r| r.seq >= f.shadowed_start && r.seq <= f.shadowed_end)
            .filter_map(|r| x_of(r.index))
            .collect();
        // 遮蔽区间整体不在窗口(翻页后):退到折叠行自身的位置画一条窄带
        // ——带要说的是「这里折过一次」,不是伪造一段区间
        let xs = if covered.is_empty() {
            match x_of(rec.index) {
                Some((x0, x1)) => vec![(x0, x0.max(x1 - 0.004))],
                None => continue,
            }
        } else {
            covered
        };
        let x0 = xs.iter().map(|(a, _)| *a).fold(f64::MAX, f64::min);
        let x1 = xs.iter().map(|(_, b)| *b).fold(f64::MIN, f64::max);
        out.push(FoldBand {
            x0,
            x1,
            clipped: f.shadowed_start < first_seq,
            record_index: rec.index,
        });
    }
    out
}

/// 条形主色(USER 蓝/TOOL 琥珀/error 红/ASSISTANT 紫)
fn span_color(span: &TlSpan) -> Rgba {
    if span.is_error {
        return theme::DANGER();
    }
    match span.kind.as_str() {
        "user" => theme::BRAND(),
        "tool" => TOOL_AMBER(),
        "message" => ASSISTANT_VIOLET(),
        _ => theme::LABEL_3(),
    }
}

// ── 渲染期快照 ─────────────────────────────────────────────────

/// store 一次性读出(避免借用交叉;同 chat_pane 的预取模式)
struct Snap<'a> {
    /// 台账(**借用**;此前为每帧 `clone()`——大会话下含 payload /
    /// thinking / system_prompt 的整表深拷贝是滚动卡顿与分配抖动之源)
    view: &'a TrajectoryView,
    /// 记录集派生索引(检查器查找用)
    index: Option<&'a RecordIndex>,
    /// 检查器选中记录所属轮(每帧算一次;此前 `record_row` **每行**重扫
    /// 全表找选中记录 —— 虚拟化后是「可视行数 × 记录数」的每帧成本)
    inspector_turn: Option<u64>,
    duration: bool,
    collapse: CollapseState,
    inspector: Option<InspectTarget>,
    inspector_tab: Option<&'static str>,
    last_tab: &'static str,
    raw_thinking: bool,
    json_expanded: HashSet<String>,
    expanded_tools: HashSet<String>,
    selection: Option<(f64, f64)>,
    viewport: Option<(f64, f64)>,
    draft: Option<(f64, f64)>,
    inspector_width: f32,
}

/// 记录集派生索引(记录变化时一次遍历建齐)。
///
/// 检查器此前三处查找都靠全表扫(`previous_system_snapshot` /
/// `parent_message` / `step_tool_calls`),台账行内还有一处「选中轮」全表扫
/// ——虚拟化后每帧每可视行各扫一遍全表(可视 50 行 × 2000 记录 = 十万级)。
/// 本索引把三者降为 O(1)/O(k)(k = 该步工具数)
pub(crate) struct RecordIndex {
    /// 记录 index → 显示序下标
    pos_of: std::collections::HashMap<u64, usize>,
    /// 每行:此前最近一条「带 System Prompt 快照的 SYSTEM 记录」下标
    prev_system: Vec<Option<usize>>,
    /// 每行:同 (turn, group) 内此前最近一条 message 下标
    parent_msg: Vec<Option<usize>>,
    /// (turn, group) → 该步全部工具记录下标(显示序)
    tool_sibs: std::collections::HashMap<(Option<u64>, String), Vec<usize>>,
}

impl RecordIndex {
    fn build(records: &[TrajectoryRecord]) -> Self {
        let mut pos_of = std::collections::HashMap::with_capacity(records.len());
        let mut prev_system = Vec::with_capacity(records.len());
        let mut parent_msg = Vec::with_capacity(records.len());
        let mut tool_sibs: std::collections::HashMap<(Option<u64>, String), Vec<usize>> =
            std::collections::HashMap::new();
        // 前一个「带快照 SYSTEM」下标 / 同步前一条 message 下标(前向扫描)
        let mut last_system: Option<usize> = None;
        let mut last_msg: std::collections::HashMap<(Option<u64>, String), usize> =
            std::collections::HashMap::new();
        for (i, r) in records.iter().enumerate() {
            pos_of.insert(r.index, i);
            prev_system.push(last_system);
            let key = (r.turn, r.group.clone());
            // 严格早于本行:先取后写(message 自身不作自己的父)
            parent_msg.push(
                last_msg
                    .get(&key)
                    .copied()
                    .filter(|j| records[*j].index < r.index),
            );
            if r.kind == "message" {
                last_msg.insert(key.clone(), i);
            }
            if r.kind == "system" && r.system_prompt.is_some() {
                last_system = Some(i);
            }
            if r.kind == "tool" {
                tool_sibs.entry(key).or_default().push(i);
            }
        }
        Self {
            pos_of,
            prev_system,
            parent_msg,
            tool_sibs,
        }
    }

    /// 前一个带 System Prompt 快照的 SYSTEM 记录(Diff 页左侧)
    fn previous_system_snapshot<'r>(
        &self,
        records: &'r [TrajectoryRecord],
        r: &TrajectoryRecord,
    ) -> Option<&'r TrajectoryRecord> {
        let i = *self.pos_of.get(&r.index)?;
        self.prev_system
            .get(i)
            .copied()
            .flatten()
            .map(|j| &records[j])
    }

    /// 同 (turn, group) 内此前最近一条 message
    fn parent_message<'r>(
        &self,
        records: &'r [TrajectoryRecord],
        r: &TrajectoryRecord,
    ) -> Option<&'r TrajectoryRecord> {
        let i = *self.pos_of.get(&r.index)?;
        self.parent_msg
            .get(i)
            .copied()
            .flatten()
            .map(|j| &records[j])
    }

    /// 该步全部工具记录(显示序)
    fn step_tool_calls<'r, 'i>(
        &'i self,
        records: &'r [TrajectoryRecord],
        r: &TrajectoryRecord,
    ) -> impl Iterator<Item = &'r TrajectoryRecord> + 'i
    where
        'r: 'i,
    {
        self.tool_sibs
            .get(&(r.turn, r.group.clone()))
            .into_iter()
            .flatten()
            .map(move |j| &records[*j])
    }

    /// 按记录 index 取记录(检查器选中的那一行)
    fn get<'r>(&self, records: &'r [TrajectoryRecord], index: u64) -> Option<&'r TrajectoryRecord> {
        self.pos_of.get(&index).map(|i| &records[*i])
    }
}

/// 行槽/投影缓存签名:任一项变化都会改变行集合或投影条,故变化即重建
#[derive(Clone, PartialEq)]
struct LedgerSig {
    /// 台账数据版本(拉取/翻页/增量各 +1)
    version: u64,
    /// 数据指纹(O(1) 派生量)。版本号由各写入点维护,指纹兜住「忘了 +1」
    /// 的路径(测试直接替换 records、外部直接改折叠集合等):缓存键必须由
    /// 依赖的数据派生,不能只信手工计数器
    records: usize,
    first_index: Option<u64>,
    last_index: Option<u64>,
    total: u64,
    has_older: bool,
    requests: usize,
    /// 折叠态版本(折叠切换单调 +1)
    collapse_ver: u64,
    /// 折叠形状(覆盖绕过版本号的直接改动:搜索页会直接改折叠集合)
    all_turns: bool,
    turns: usize,
    all_calls: bool,
    calls: usize,
    /// 搜索串(空 = 不过滤)
    search: String,
    /// 时间线 duration 模式
    duration: bool,
    /// 时间线选区(域归一化;None = 全览)
    selection: Option<(f64, f64)>,
    /// 时间线拖拽草稿选区
    draft: Option<(f64, f64)>,
}

/// 行槽与投影行缓存(签名守卫)。虚拟列表每帧按 index 直读本缓存,故
/// `build_spans` / `build_fold_bands` / 可见性过滤 / `build_rows` 只在签名
/// 变化时跑——稳态帧与滚动帧免除全部 O(n) 预计算
pub(crate) struct LedgerCache {
    sig: LedgerSig,
    /// 时间线投影条
    spans: Vec<TlSpan>,
    /// 投影条上的折叠带
    bands: Vec<FoldBand>,
    /// 台账行槽(「加载更早」前缀 + 数据行)
    pub(crate) rows: Vec<LedgerRow>,
    /// 记录集派生索引(检查器 O(1) 查找)
    index: RecordIndex,
}

/// 可见性 = 搜索 ∧ 时间线选区(选区按投影条重叠;无条目记录被移出)
fn visible_records(
    records: &[TrajectoryRecord],
    spans: &[TlSpan],
    search: &str,
    range: Option<(f64, f64)>,
) -> Vec<bool> {
    let searching = !search.trim().is_empty();
    records
        .iter()
        .map(|r| {
            let ok_search = !searching || search_hit(r, search);
            let ok_range = match range {
                None => true,
                Some((a, b)) => spans
                    .iter()
                    .any(|sp| sp.record_index == r.index && sp.x1 > a && sp.x0 < b),
            };
            ok_search && ok_range
        })
        .collect()
}

/// 重建行槽/投影缓存(签名命中即返回)。渲染前调用
pub(crate) fn refresh_view_cache(st: &mut AppStore, cx: &App) {
    let search = st
        .trajectory
        .trajectory_search
        .as_ref()
        .map(|e| e.read(cx).value().to_string())
        .unwrap_or_default();
    let view = &st.trajectory.trajectory;
    let sig = LedgerSig {
        version: st.trajectory.trajectory_version,
        records: view.records.len(),
        first_index: view.records.first().map(|r| r.index),
        last_index: view.records.last().map(|r| r.index),
        total: view.total,
        has_older: view.has_older,
        requests: view.requests.len(),
        collapse_ver: st.trajectory.collapse_ver,
        all_turns: st.trajectory.all_turns_collapsed,
        turns: st.trajectory.collapsed_turns.len(),
        all_calls: st.trajectory.all_calls_collapsed,
        calls: st.trajectory.collapsed_calls.len(),
        search,
        duration: st.trajectory.trajectory_duration,
        selection: st.trajectory.timeline_selection,
        draft: st.trajectory.timeline_draft,
    };
    if st
        .trajectory
        .view_cache
        .as_ref()
        .is_some_and(|c| c.sig == sig)
    {
        return;
    }
    // 形状变化前的可视锚:当前顶行所在记录(折叠/搜索后据此复位视口,
    // 免长台账跳回顶部)
    let anchor_rec_ix = st.trajectory.view_cache.as_ref().and_then(|c| {
        let top = st.trajectory.trajectory_list.logical_scroll_top();
        c.rows.get(top.item_ix..)?.iter().find_map(|r| match r {
            LedgerRow::Record { rec_ix, .. } => Some(*rec_ix),
            _ => None,
        })
    });

    let records = &st.trajectory.trajectory.records;
    let spans = build_spans(records, sig.duration);
    let bands = build_fold_bands(records, &spans);
    let collapse = CollapseState {
        all_turns: st.trajectory.all_turns_collapsed,
        turns: st.trajectory.collapsed_turns.clone(),
        calls: st.trajectory.collapsed_calls.clone(),
        all_calls: st.trajectory.all_calls_collapsed,
    };
    let visible = visible_records(records, &spans, &sig.search, sig.selection.or(sig.draft));
    let mut rows: Vec<LedgerRow> = Vec::new();
    if st.trajectory.trajectory.has_older {
        rows.push(LedgerRow::LoadEarlier);
    }
    rows.extend(build_rows(records, &visible, &collapse));

    st.trajectory.view_cache = Some(LedgerCache {
        sig,
        spans,
        bands,
        rows,
        index: RecordIndex::build(records),
    });
    sync_trajectory_rows(st, anchor_rec_ix);
}

/// 行槽对齐 ListState。行高按型定值(记录 30px;摘要行 20px;「加载更早」
/// 30px),故只按**行型序列**判变化,不做无谓重排:
/// - 序列同长同型 = 纯内容更新(流式 upsert / 状态翻转),列表零操作;
/// - 纯追加 → `splice(old..old, added)`(新项 Unmeasured,入视口才测高);
/// - 纯前插 → `splice(0..0, added)`:list 内部把 `logical_scroll_top` 的
///   `item_ix` 平移 +added 且保留已测高度,「加载更早」不跳视口;
/// - 形状变化(折叠/搜索/换会话)→ `reset` + 按锚记录复位视口
fn sync_trajectory_rows(st: &mut AppStore, anchor_rec_ix: Option<usize>) {
    let Some(cache) = st.trajectory.view_cache.as_ref() else {
        return;
    };
    let kinds: Vec<u8> = cache.rows.iter().map(LedgerRow::kind).collect();
    let old = std::mem::replace(&mut st.trajectory.row_kinds, kinds.clone());
    if old == kinds {
        return; // 行型未变 = 总高未变
    }
    let list = st.trajectory.trajectory_list.clone();
    let (old_len, new_len) = (old.len(), kinds.len());
    if old_len < new_len {
        let added = new_len - old_len;
        if kinds[..old_len] == old[..] {
            list.splice(old_len..old_len, added); // 纯追加
            return;
        }
        if kinds[added..] == old[..] {
            list.splice(0..0, added); // 纯前插(滚动位自动平移)
            return;
        }
    }
    // 形状变化:重建 + 复位到原可视记录。reset 会清掉 logical_scroll_top,
    // 位置自此不可信(见 store::scroll_pos_known),须等下一次真滚动
    st.trajectory.scroll_pos_known = false;
    list.reset(new_len);
    if let Some(target) = anchor_rec_ix
        && let Some(ix) = cache
            .rows
            .iter()
            .position(|r| matches!(r, LedgerRow::Record { rec_ix, .. } if *rec_ix == target))
    {
        list.scroll_to(gpui_kit::ListOffset {
            item_ix: ix,
            offset_in_item: px(0.),
        });
    }
}

fn snap(st: &AppStore) -> Snap<'_> {
    let index = st.trajectory.view_cache.as_ref().map(|c| &c.index);
    let inspector_turn = match st.trajectory.inspector {
        Some(InspectTarget::Record(ix)) => index
            .and_then(|i| i.get(&st.trajectory.trajectory.records, ix))
            .and_then(|r| r.turn),
        _ => None,
    };
    Snap {
        view: &st.trajectory.trajectory,
        index,
        inspector_turn,
        duration: st.trajectory.trajectory_duration,
        collapse: CollapseState {
            all_turns: st.trajectory.all_turns_collapsed,
            turns: st.trajectory.collapsed_turns.clone(),
            all_calls: st.trajectory.all_calls_collapsed,
            calls: st.trajectory.collapsed_calls.clone(),
        },
        inspector: st.trajectory.inspector,
        inspector_tab: st.trajectory.inspector_tab,
        last_tab: st.trajectory.inspector_last_tab,
        raw_thinking: st.trajectory.inspector_raw_thinking,
        json_expanded: st.trajectory.json_expanded.clone(),
        expanded_tools: st.trajectory.expanded_inspector_tools.clone(),
        selection: st.trajectory.timeline_selection,
        viewport: st.trajectory.timeline_viewport,
        draft: st.trajectory.timeline_draft,
        inspector_width: st.trajectory.inspector_width,
    }
}

/// track bounds 共享单元格(渲染期 canvas 写入,事件闭包读取;
/// GPUI 渲染单线程串行,thread-local 安全)
fn track_bounds_cell() -> Rc<Cell<Option<Bounds<Pixels>>>> {
    thread_local! {
        static CELL: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::default();
    }
    CELL.with(|c| c.clone())
}

/// 窗口 x → track 原始分数 0..1
fn local_frac(cell: &Rc<Cell<Option<Bounds<Pixels>>>>, window_x: Pixels) -> Option<f64> {
    let b = cell.get()?;
    let w = f32::from(b.size.width).max(1.) as f64;
    Some(f32::from(window_x - b.origin.x) as f64 / w)
}

// ── 主渲染 ────────────────────────────────────────────────────

/// 轨迹视图根:工具栏 + 时间线 + (台账表 | 检查器)
pub fn render(store: &Entity<AppStore>, window: &mut Window, cx: &mut App) -> impl IntoElement {
    // 行槽/投影行按签名缓存:命中即免本帧全部 O(n) 预计算;台账滚动由
    // ListState 托管(回调只装一次)
    store.update(cx, |st, cx| {
        refresh_view_cache(st, cx);
        st.install_trajectory_scroll_handler(cx);
    });
    // 小状态一次取净(借用即刻结束):长借 `&AppStore` 会与下方各段所需的
    // `&mut App` 冲突,故快照在各段内按需构建
    let (resizing, drag_active, spans, bands) = {
        let st = store.read(cx);
        let (spans, bands) = match st.trajectory.view_cache.as_ref() {
            Some(c) => (c.spans.clone(), c.bands.clone()),
            None => (Vec::new(), Vec::new()),
        };
        (
            st.trajectory.inspector_resize_anchor.is_some(),
            st.trajectory.timeline_drag.is_some(),
            spans,
            bands,
        )
    };
    // 拖拽监听需要 spans 的自持副本(下方 timeline 取走所有权)
    let listeners_spans = spans.clone();

    div()
        .id("trajectory-view")
        .v_flex()
        .size_full()
        .min_h(px(0.))
        .overflow_hidden()
        .debug_selector(|| "trajectory-view".to_string())
        .child(toolbar(store, cx))
        .child(timeline(store, cx, spans, bands))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                // 行区水平收缩许可:台账(flex_1)先缩、检查器(flex_shrink_0)
                // 由渲染期钳制让位,无 min_w(0) 时二者以内容最小宽参与协商
                .min_w(px(0.))
                .child(ledger(store, cx))
                .children(inspector(store, window, cx)),
        )
        // 拖宽/时间线拖拽进行中:窗口级 move/up 经 canvas.paint(Paint
        // 相位)注册——render 在 Prepaint 相位跑,直接 on_mouse_event
        // 会 panic(与 sessions::drag_overlay 同款惯例)
        .when(resizing || drag_active, |el| {
            el.child(window_listeners(store, listeners_spans))
        })
}

/// 拖宽/时间线拖拽进行中的窗口级 move/up 注册(Paint 相位):
/// canvas.paint 每帧重跑,指针移出面板仍收拖动事件(无指针捕获的
/// GPUI 惯例);拖拽态清空后内层早退,结束即自然消失
fn window_listeners(store: &Entity<AppStore>, spans: Vec<TlSpan>) -> impl IntoElement {
    let m = store.clone();
    let u = store.clone();
    gpui_kit::canvas(
        // prepaint:无自定义绘制
        |_, _, _| (),
        move |_, _, window, cx| {
            let resizing = m.read(cx).trajectory.inspector_resize_anchor.is_some();
            let dragging = m.read(cx).trajectory.timeline_drag.is_some();
            if resizing {
                let m2 = m.clone();
                window.on_mouse_event(move |ev: &MouseMoveEvent, _, _, cx| {
                    m2.update(cx, |st, cx| {
                        st.inspector_resize_move(f32::from(ev.position.x), cx)
                    });
                });
                let u2 = u.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, _, _, cx| {
                    u2.update(cx, |st, cx| st.inspector_resize_end(cx));
                });
            }
            if dragging {
                let bounds = track_bounds_cell();
                let m2 = m.clone();
                window.on_mouse_event(move |ev: &MouseMoveEvent, _, _, cx| {
                    if let Some(frac) = local_frac(&bounds, ev.position.x) {
                        m2.update(cx, |st, cx| st.move_timeline_drag(frac, cx));
                    }
                });
                let u2 = u.clone();
                let u_bounds = track_bounds_cell();
                let u_spans = spans.clone();
                window.on_mouse_event(move |ev: &MouseUpEvent, _, _, cx| {
                    let (anchor, draft) = {
                        let st = u2.read(cx);
                        (st.trajectory.timeline_drag, st.trajectory.timeline_draft)
                    };
                    let Some(anchor) = anchor else {
                        u2.update(cx, |st, cx| st.clear_timeline_drag(cx));
                        return;
                    };
                    let track_w = u_bounds
                        .get()
                        .map(|b| f32::from(b.size.width))
                        .unwrap_or(0.) as f64;
                    let cur = local_frac(&u_bounds, ev.position.x).unwrap_or(anchor);
                    // 位移 <3px 视为点击:命中条形 → 选记录;空白 → 最小窗口选区
                    let is_click = (cur - anchor).abs() * track_w < 3.;
                    u2.update(cx, |st, cx| {
                        st.clear_timeline_drag(cx);
                        if is_click {
                            let (v0, v1) = st.trajectory.timeline_viewport.unwrap_or((0., 1.));
                            let domain = cur * (v1 - v0) + v0;
                            if let Some(sp) =
                                u_spans.iter().find(|sp| domain >= sp.x0 && domain <= sp.x1)
                            {
                                let ix = sp.record_index;
                                st.select_trajectory_record(ix, cx);
                            } else {
                                // 最小窗口 = 4 个操作宽,以点击点为中心
                                let n = u_spans.len().max(1) as f64;
                                let half = 2. / n;
                                st.set_timeline_selection(Some((domain - half, domain + half)), cx);
                            }
                        } else if let Some(d) = draft {
                            st.set_timeline_selection(Some(d), cx);
                        }
                    });
                });
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

// ── 工具栏─────────────────────────────

fn toolbar(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let s = snap(st);
    let input = store
        .read(cx)
        .trajectory
        .trajectory_search
        .as_ref()
        .map(|e| {
            div()
                .flex()
                .w(px(164.))
                .h(px(24.))
                .items_center()
                .child(Input::new(e).small())
        });
    // 库 Toolbar 接管条体与 roving 键盘焦点;尺寸档取 XSmall 使按钮与
    // 其包装层同为 20px(`input_h(XSmall)` = `h_5()`),与手绘药丸同高;
    // 条高/内外边距再由本处样式覆盖回原值(库 XSmall 档默认 h_7/p_1/gap_1)
    // 库 `Toolbar` 未实现 `InteractiveElement`,挂不了 debug_selector;
    // 外层包一层只作测试寻址(与 kits::collapse_strip 同款做法)
    div()
        .debug_selector(|| "traj-toolbar".to_string())
        .flex_shrink_0()
        .child(
            Toolbar::new("traj-toolbar")
                .xsmall()
                .h(px(32.))
                .px(px(8.))
                .gap(px(2.))
                .border_b_1()
                .border_color(theme::BORDER())
                .content(toggle_button(
                    cx,
                    "traj-toolbar-jump-top",
                    t!("trajectory.jump_top"),
                    false,
                    fixed(IconName::ChevronUp, 12.),
                    {
                        let s = store.clone();
                        move |_, _, cx| {
                            s.update(cx, |st, cx| st.jump_trajectory_top(cx));
                        }
                    },
                ))
                .content(toggle_button(
                    cx,
                    "traj-toolbar-jump-bottom",
                    t!("trajectory.jump_bottom"),
                    false,
                    fixed(IconName::ChevronDown, 12.),
                    {
                        let s = store.clone();
                        move |_, _, cx| {
                            s.update(cx, |st, cx| st.jump_trajectory_bottom(cx));
                        }
                    },
                ))
                .content(toggle_button(
                    cx,
                    "traj-toolbar-duration",
                    t!("trajectory.toolbar_duration"),
                    s.duration,
                    fixed(LiumaIcon::Clock, 12.),
                    {
                        let s = store.clone();
                        move |_, _, cx| {
                            s.update(cx, |st, cx| st.toggle_trajectory_duration(cx));
                        }
                    },
                ))
                .content(action_button(
                    cx,
                    "traj-toolbar-turns",
                    t!("trajectory.toolbar_turns"),
                    s.collapse.all_turns,
                    {
                        let s = store.clone();
                        move |_, _, cx| {
                            s.update(cx, |st, cx| st.toggle_all_turns(cx));
                        }
                    },
                ))
                .content(action_button(
                    cx,
                    "traj-toolbar-calls",
                    t!("trajectory.toolbar_calls"),
                    s.collapse.all_calls,
                    {
                        let s = store.clone();
                        move |_, _, cx| {
                            s.update(cx, |st, cx| st.toggle_all_calls(cx));
                        }
                    },
                ))
                // 计数文本与搜索框走 content(非 Sizable,库不施加尺寸档——
                // 原样保留);库要求宿主输入框置于条尾以保其自身方向键行为
                .content(
                    div()
                        .min_w(px(0.))
                        .flex_1()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(theme::CAPTION())
                        .pl(px(8.))
                        .child(t!(
                            "trajectory.counts",
                            shown = s.view.records.len(),
                            total = s.view.total,
                            requests = s.view.requests.len()
                        )),
                )
                .contents(input.map(|el| el.into_any_element())),
        )
}

/// 工具栏切换钮(模式开关,pressed = 高亮;恒显自身图标)
///
/// 走 `custom` 变体而非默认 ghost:库 `Button` 自己必然设 hover 样式,
/// 再调 `.hover()` 会撞 GPUI 的「hover style already set」断言,故悬停配色
/// 只能经变体给定。也因此本钮以 `Toolbar::content` 挂入——`Toolbar::child`
/// 在渲染期强制 `prepare_for_toolbar()`(= `.ghost().compact()`),会把变体
/// 改回 ghost;尺寸档改由本处 `.xsmall()` 自持。
fn toggle_button(
    cx: &App,
    id: &'static str,
    label: impl Into<gpui_kit::SharedString>,
    pressed: bool,
    icon: Icon,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    let label = label.into();
    let sel = id.to_string();
    let label_sel = format!("{id}-label");
    Button::new(id)
        .compact()
        .xsmall()
        .debug_selector(move || sel.clone())
        // 可见文字走子元素时,无障碍名仍要显式给(库的 `.label()` 二者兼供)
        .accessibility_label(label.to_string())
        .rounded(px(6.))
        .px(px(8.))
        // 库 `Button` 底是 `cursor_default()`,自定义变体不会转手型
        .cursor_pointer()
        .child(
            // 图标 + 文字自成一排:库对**子元素**内容自设
            // `.button_text_size(self.size)`,子元素自带字号压过继承 ——
            // 在钮根上写 `.text_size()` 到不了文字(XSmall 档 = 12px)
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(px(11.))
                .child(icon)
                .child(div().debug_selector(move || label_sel.clone()).child(label)),
        )
        .when(pressed, |el| {
            el.border_1().border_color(theme::GLASS_BORDER())
        })
        .custom(
            ButtonCustomVariant::new(cx)
                .color(if pressed {
                    theme::GLASS_BG().into()
                } else {
                    theme::TRANSPARENT().into()
                })
                .foreground(if pressed {
                    theme::LABEL().into()
                } else {
                    theme::LABEL_3().into()
                })
                // 按下态原无悬停变化,与底色同值即无变化
                .hover(if pressed {
                    theme::GLASS_BG().into()
                } else {
                    theme::BORDER().into()
                }),
        )
        .on_click(move |ev, w, cx| on_click(ev, w, cx))
}

/// 工具栏动作钮(展开/折叠动作,无 pressed 态;图标随状态
/// 翻转——全折叠显 ⊞(点=展开),展开显 ⊟(点=折叠),等宽字体)
fn action_button(
    cx: &App,
    id: &'static str,
    label: impl Into<gpui_kit::SharedString>,
    all_collapsed: bool,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    let label = label.into();
    let sel = id.to_string();
    let label_sel = format!("{id}-label");
    // 图标位是 Menlo 字形而非图标字体,故走 `Button` 的任意子元素槽
    // (`Button` 实现了 `ParentElement`),不用 `.icon()`
    Button::new(id)
        .compact()
        .xsmall()
        .debug_selector(move || sel.clone())
        .accessibility_label(label.to_string())
        .rounded(px(6.))
        .px(px(5.))
        // 库 `Button` 底是 `cursor_default()`,自定义变体不会转手型
        .cursor_pointer()
        .custom(
            ButtonCustomVariant::new(cx)
                .color(theme::TRANSPARENT().into())
                .foreground(theme::LABEL_3().into())
                .hover(theme::BORDER().into()),
        )
        // 字形与文字自成一排(字号同 toggle_button:库对子元素内容自设
        // `.button_text_size(self.size)`,写在钮根上到不了文字)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(px(11.))
                .child(
                    div()
                        .font_family("Menlo")
                        .text_size(px(12.))
                        .line_height(gpui_kit::relative(1.))
                        .child(if all_collapsed { "⊞" } else { "⊟" }),
                )
                .child(div().debug_selector(move || label_sel.clone()).child(label)),
        )
        .on_click(move |ev, w, cx| on_click(ev, w, cx))
}

// ── Overview 时间线────────────────────

fn timeline(
    store: &Entity<AppStore>,
    cx: &App,
    spans: Vec<TlSpan>,
    bands: Vec<FoldBand>,
) -> impl IntoElement {
    let st = store.read(cx);
    let s = snap(st);
    let (spans, bands) = (spans.as_slice(), bands.as_slice());
    let labels = [
        t!("trajectory.legend_input"),
        t!("trajectory.legend_model"),
        t!("trajectory.legend_tools"),
    ];
    let labels_col = div().w(px(44.)).flex_shrink_0().relative().children(
        labels
            .iter()
            .enumerate()
            .map(|(lane, l)| {
                div()
                    .absolute()
                    .left_0()
                    .right(px(6.))
                    .top(px(6. + lane as f32 * 14.))
                    .text_right()
                    .text_size(px(10.))
                    .text_color(theme::CAPTION())
                    .child(l.to_string())
            })
            .collect::<Vec<_>>(),
    );

    // 视口映射:域分数 → track 分数
    let (v0, v1) = s.viewport.unwrap_or((0., 1.));
    let vspan = (v1 - v0).max(1e-6);
    let to_track = |x: f64| ((x - v0) / vspan).clamp(0., 1.7);

    let selected_record = match s.inspector {
        Some(InspectTarget::Record(ix)) => Some(ix),
        _ => None,
    };
    let mut bars: Vec<gpui_kit::AnyElement> = Vec::new();
    for (i, sp) in spans.iter().enumerate() {
        let x0 = to_track(sp.x0);
        let x1 = to_track(sp.x1);
        if x1 <= 0. || x0 >= 1. {
            continue;
        }
        let lane = lane_of(&sp.kind) as f32;
        let top = px(7. + lane * 14.);
        let left_pct = x0.clamp(0., 1.);
        let right_pct = x1.clamp(0., 1.);
        let width = right_pct - left_pct;
        let is_current = selected_record == Some(sp.record_index);
        let store2 = store.clone();
        let ix = sp.record_index;
        // 条构造闭包(TTFT 分色需建两次;Stateful 不可 Clone)
        let make = |left: f32, w: f32| {
            let s3 = store2.clone();
            div()
                .id(("tl-span", i))
                .debug_selector(move || format!("tl-span-{ix}"))
                .absolute()
                .top(top)
                .h(px(8.))
                .rounded(px(1.))
                .left(gpui_kit::relative(left))
                .w(gpui_kit::relative(w))
                .when(is_current, |el| el.border_1().border_color(theme::BRAND()))
                .hover(|st| st.border_1().border_color(theme::LABEL_3()))
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    s3.update(cx, |st, cx| st.select_trajectory_record(ix, cx));
                })
        };
        if let Some(split) = sp.ttft_split {
            // TTFT/解码分色:左段弱紫 + 右段解码紫(渐变分界的离散近似)
            let total = width.max(1e-4) as f32;
            let left_w = (total * split as f32).max(0.002);
            bars.push(
                make(left_pct as f32, left_w)
                    .bg(TTFT_VIOLET())
                    .into_any_element(),
            );
            bars.push(
                make(
                    (left_pct + left_w as f64) as f32,
                    (total - left_w).max(0.002),
                )
                .bg(ASSISTANT_VIOLET())
                .into_any_element(),
            );
        } else {
            bars.push(
                make(left_pct as f32, width.max(0.004) as f32)
                    .bg(span_color(sp))
                    .into_any_element(),
            );
        }
    }

    // turn 边界竖线(turn_start 记录的条形位置)。
    // span 定位用预建映射:此前每条 turn 线对 spans 线性 find,
    // O(轮数 × 记录数)——2000 记录 × 40 轮 = 每帧 8 万次比较,
    // 轨迹面板开着时随每次重绘跑(渲染风暴期 ×140 帧)
    let span_by_record: std::collections::HashMap<u64, &TlSpan> =
        spans.iter().map(|sp| (sp.record_index, sp)).collect();
    let mut turn_lines: Vec<gpui_kit::AnyElement> = Vec::new();
    for r in s.view.records.iter().filter(|r| r.turn_start) {
        if let Some(sp) = span_by_record.get(&r.index) {
            let sp = *sp;
            let x = to_track(sp.x0).clamp(0., 1.);
            if x <= 0. {
                continue;
            }
            turn_lines.push(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(gpui_kit::relative(x as f32))
                    .w(px(1.))
                    .bg(theme::BORDER())
                    .into_any_element(),
            );
        }
    }

    // 折叠带(画在条形**之下**:它是背景事实,不抢条形的可点性;
    // 点击 = 选中那一次折叠的台账行)
    let mut fold_bands: Vec<gpui_kit::AnyElement> = Vec::new();
    for (i, band) in bands.iter().enumerate() {
        let x0 = to_track(band.x0).clamp(0., 1.);
        let x1 = to_track(band.x1).clamp(0., 1.);
        if x1 <= 0. || x0 >= 1. {
            continue;
        }
        let w = (x1 - x0).max(0.006);
        let s2 = store.clone();
        let ix = band.record_index;
        let line = gpui_kit::Rgba {
            a: 0.28,
            ..theme::LABEL_3()
        };
        fold_bands.push(
            div()
                .id(("fold-band", i))
                .debug_selector(move || format!("fold-band-{i}"))
                .absolute()
                .top_0()
                .bottom_0()
                .left(gpui_kit::relative(x0 as f32))
                .w(gpui_kit::relative(w as f32))
                .bg(gpui_kit::Rgba {
                    a: 0.10,
                    ..theme::LABEL_3()
                })
                .border_l_1()
                .border_r_1()
                .border_color(line)
                .cursor_pointer()
                .hover(|st| {
                    st.bg(gpui_kit::Rgba {
                        a: 0.18,
                        ..theme::LABEL_3()
                    })
                })
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    s2.update(cx, |st, cx| st.select_trajectory_record(ix, cx));
                })
                // 起点早于已载窗口:左缘标「更早」,不假装区间完整
                .when(band.clipped, |el| {
                    el.child(
                        div()
                            .debug_selector(move || format!("fold-band-clip-{i}"))
                            .absolute()
                            .left(px(1.))
                            .top(px(2.))
                            .text_size(px(8.))
                            .text_color(theme::CAPTION())
                            .child(t!("trajectory.fold_band_clip").to_string()),
                    )
                })
                .into_any_element(),
        );
    }

    // 选区/草稿视觉:填充 + 两侧边线 + 选区外遮罩
    let sel = s.draft.or(s.selection);
    let mut overlays: Vec<gpui_kit::AnyElement> = Vec::new();
    if let Some((a, b)) = sel {
        let a = to_track(a).clamp(0., 1.);
        let b = to_track(b).clamp(0., 1.);
        if b > a {
            overlays.push(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(gpui_kit::relative(a as f32))
                    .w(gpui_kit::relative((b - a) as f32))
                    .bg(gpui_kit::Rgba {
                        a: 0.12,
                        ..theme::BRAND()
                    })
                    .into_any_element(),
            );
            for x in [a, b] {
                overlays.push(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(gpui_kit::relative(x as f32))
                        .w(px(2.))
                        .bg(theme::BRAND())
                        .into_any_element(),
                );
            }
            if a > 0. {
                overlays.push(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(gpui_kit::relative(a as f32))
                        .bg(gpui_kit::Rgba {
                            a: 0.58,
                            ..theme::INK()
                        })
                        .into_any_element(),
                );
            }
            if b < 1. {
                overlays.push(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(gpui_kit::relative(b as f32))
                        .right_0()
                        .bg(gpui_kit::Rgba {
                            a: 0.58,
                            ..theme::INK()
                        })
                        .into_any_element(),
                );
            }
        }
    }

    // 空计时数据文案
    let empty = spans.is_empty().then(|| {
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(10.))
            .text_color(theme::CAPTION())
            .child(t!("trajectory.no_timing_data"))
    });

    let bounds = track_bounds_cell();
    let down_store = store.clone();
    let left_store = down_store.clone();
    let down_bounds = bounds.clone();
    let wheel_store = store.clone();
    let wheel_bounds = bounds.clone();
    let wheel_n = spans.len().max(1);

    // 「…」加载更早(左缘,仅 has_older)
    let load_earlier = s.view.has_older.then(|| {
        let s2 = store.clone();
        div()
            .id("tl-load-earlier")
            .absolute()
            .left_0()
            .top_0()
            .bottom_0()
            .w(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(11.))
            .text_color(theme::LABEL_3())
            .bg(theme::DOCK())
            .rounded_r(px(6.))
            .opacity(0.85)
            .hover(|st| st.opacity(1.))
            .cursor_pointer()
            .child("…")
            .on_click(move |_, _, cx| {
                s2.update(cx, |st, cx| st.load_earlier_trajectory(cx));
            })
    });

    let track = div()
        .id("timeline-track")
        .relative()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .overflow_hidden()
        .cursor(CursorStyle::Crosshair)
        .debug_selector(|| "timeline-track".to_string())
        // 渲染期写入 bounds(canvas 包一层 div 以取 Styled)
        .child(div().absolute().size_full().child(gpui_kit::canvas(
            move |b, _, _| {
                bounds.set(Some(b));
            },
            |_, _, _, _| {},
        )))
        .children(fold_bands)
        .children(turn_lines)
        .children(bars)
        .children(overlays)
        .children(load_earlier)
        .children(empty)
        .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, _, cx| {
            // 双击清选区;按下开启拖拽(锚点 = track 原始分数)
            if ev.click_count == 2 {
                left_store.update(cx, |st, cx| st.set_timeline_selection(None, cx));
                return;
            }
            let Some(frac) = local_frac(&down_bounds, ev.position.x) else {
                return;
            };
            cx.stop_propagation();
            left_store.update(cx, |st, cx| st.begin_timeline_drag(frac, cx));
        })
        .on_mouse_down(MouseButton::Right, {
            let right_store = down_store.clone();
            move |_, _, cx| {
                cx.stop_propagation();
                right_store.update(cx, |st, cx| st.set_timeline_selection(None, cx));
            }
        })
        .on_scroll_wheel(move |ev: &ScrollWheelEvent, _, cx| {
            // 滚轮缩放:光标锚点,domain × exp(ΔY×0.0015),下限 4 操作
            let Some(b) = wheel_bounds.get() else { return };
            let w = f32::from(b.size.width).max(1.) as f64;
            let local = f32::from(ev.position.x - b.origin.x) as f64 / w;
            if !(0. ..=1.).contains(&local) {
                return;
            }
            let dy = match ev.delta {
                gpui_kit::ScrollDelta::Lines(l) => l.y as f64 * 40.,
                gpui_kit::ScrollDelta::Pixels(p) => f32::from(p.y) as f64,
            };
            let min_span = 4. / wheel_n as f64;
            wheel_store.update(cx, |st, cx| {
                let (a0, b0) = st.trajectory.timeline_viewport.unwrap_or((0., 1.));
                let span = b0 - a0;
                if span >= 1. && dy < 0. {
                    return; // 已全览继续放大:无操作
                }
                let ratio = (dy * 0.0015).exp();
                let new_span = (span * ratio).clamp(min_span, 1.);
                let anchor = a0 + local * span;
                let na = (anchor - local * new_span).clamp(0., 1. - new_span);
                let nb = na + new_span;
                st.set_timeline_viewport(Some((na, nb)), cx);
            });
        });

    div()
        .flex()
        .flex_shrink_0()
        .h(px(50.))
        .bg(theme::CARD())
        .border_b_1()
        .border_color(theme::BORDER())
        .child(labels_col)
        .child(track)
        .child(
            div()
                .flex()
                .items_center()
                .px(px(6.))
                .text_size(px(9.))
                .text_color(theme::CAPTION())
                .child(t!("trajectory.drag_hint")),
        )
}

// ── 台账表────────────────────────────────

/// 台账表:内建 `list()` 虚拟化——只有可视 + overdraw 行被构建。
///
/// 行槽由 [`LedgerCache`] 按签名预计算,item 闭包按 index 直读(与消息列
/// 同款)。此前每帧构建全部行、且 `snap()` 深拷贝整份台账(含 payload /
/// thinking / system_prompt):大会话滚动是 MB 级分配 + O(n²) 逐行全表扫
fn ledger(store: &Entity<AppStore>, cx: &App) -> impl IntoElement {
    let st = store.read(cx);
    let list_state = st.trajectory.trajectory_list.clone();
    let loading_initial =
        st.trajectory.trajectory.loading && st.trajectory.trajectory.records.is_empty();
    let empty_table = !loading_initial && st.trajectory.trajectory.records.is_empty();
    let row_count = st
        .trajectory
        .view_cache
        .as_ref()
        .map(|c| c.rows.len())
        .unwrap_or(0);
    // 空态/首拉态不入列表:整面占位(与定高数据行不同型,单独渲染)
    if empty_table || loading_initial || row_count == 0 {
        return div()
            .id("trajectory-scroll")
            .debug_selector(|| "trajectory-scroll".to_string())
            .v_flex()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .items_center()
            .justify_center()
            .gap(px(8.))
            .py(px(40.))
            .text_size(px(12.))
            .text_color(theme::CAPTION())
            .when(loading_initial, |el| {
                el.child(Spinner::new().xsmall())
                    .child(t!("trajectory.folding"))
            })
            .when(empty_table, |el| {
                el.child(fixed(IconName::Inbox, 16.))
                    .child(t!("trajectory.empty"))
            })
            .when(empty_table, |el| {
                el.debug_selector(|| "trajectory-empty".to_string())
            })
            .when(loading_initial, |el| {
                el.debug_selector(|| "trajectory-loading".to_string())
            })
            .into_any_element();
    }

    // 逐项闭包持 store 实体:虚拟化下只有可视(+overdraw)项被构建,
    // 每项单次 read 借用
    let item_store = store.clone();
    let list = gpui_kit::list(list_state, move |ix, _window, cx| {
        let st = item_store.read(cx);
        let s = snap(st);
        let Some(cache) = st.trajectory.view_cache.as_ref() else {
            return div().into_any_element();
        };
        let Some(row) = cache.rows.get(ix) else {
            return div().into_any_element();
        };
        match row {
            LedgerRow::LoadEarlier => load_earlier_row(st, &item_store).into_any_element(),
            LedgerRow::TurnSummary { turn, steps, tools } => {
                turn_summary_row(&item_store, *turn, *steps, *tools).into_any_element()
            }
            LedgerRow::CallSummary {
                message_index,
                count,
                names,
            } => call_summary_row(&item_store, *message_index, *count, names).into_any_element(),
            LedgerRow::Record { rec_ix, turn_start } => match s.view.records.get(*rec_ix) {
                Some(rec) => record_row(&item_store, &s, rec, *turn_start).into_any_element(),
                None => div().into_any_element(),
            },
        }
    });

    // 点表空白:关检查器 + 清时间线选区(行内点击已 stop_propagation,
    // 到达此处的必是背景点击,一并清空选中态)
    let s2 = store.clone();
    div()
        .id("trajectory-scroll")
        .debug_selector(|| "trajectory-scroll".to_string())
        .v_flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .on_click(move |_, _, cx| {
            s2.update(cx, |st, cx| {
                st.close_inspector(cx);
                st.set_timeline_selection(None, cx);
            });
        })
        // 列表须显式占满包裹层(taffy 下 auto 尺寸会塌成 0 高 → 零行)
        .child(list.h_full().w_full())
        .into_any_element()
}

/// 「加载更早」行(30px;has_older 时置行首)
fn load_earlier_row(st: &AppStore, store: &Entity<AppStore>) -> impl IntoElement {
    let loading = st.trajectory.trajectory.loading_older;
    let remaining = st
        .trajectory
        .trajectory
        .total
        .saturating_sub(st.trajectory.trajectory.records.len() as u64);
    let s2 = store.clone();
    div()
        .id("load-earlier")
        .flex()
        .h(px(30.))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(theme::LABEL_3())
        .hover(|st| st.bg(theme::LAYER()).text_color(theme::LABEL_2()))
        .debug_selector(|| "load-earlier".to_string())
        .when(loading, |el| {
            el.child(Spinner::new().xsmall())
                .child(t!("trajectory.loading_older").to_string())
        })
        .when(!loading, |el| {
            el.child(t!("trajectory.load_older", n = remaining))
        })
        .on_click(move |_, _, cx| {
            s2.update(cx, |st, cx| st.load_earlier_trajectory(cx));
        })
}

/// Turn 折叠摘要行(20px)
fn turn_summary_row(
    store: &Entity<AppStore>,
    turn: u64,
    steps: usize,
    tools: usize,
) -> impl IntoElement {
    let s = store.clone();
    div()
        .id(("turn-summary", turn as usize))
        .flex()
        .h(px(20.))
        .flex_shrink_0()
        .min_w(px(0.))
        .items_center()
        .pl(px(40.))
        .cursor_pointer()
        .text_size(px(11.))
        .text_color(theme::CAPTION())
        .hover(|st| st.text_color(theme::LABEL_3()))
        .debug_selector(move || format!("turn-summary-{turn}"))
        .child(
            // 长文本截断(record_row 正文列同款):窄面板下不让行内容
            // 溢出右缘
            div().min_w(px(0.)).flex_1().truncate().child(t!(
                "trajectory.folded_steps",
                steps = steps,
                tools = tools
            )),
        )
        .on_click(move |_, _, cx| {
            s.update(cx, |st, cx| st.toggle_turn(turn, cx));
        })
}

/// Calls 折叠摘要行(20px)
fn call_summary_row(
    store: &Entity<AppStore>,
    message_index: u64,
    count: usize,
    names: &[String],
) -> impl IntoElement {
    let s = store.clone();
    div()
        .id(("call-summary", message_index as usize))
        .flex()
        .h(px(20.))
        .flex_shrink_0()
        .min_w(px(0.))
        .items_center()
        .pl(px(40.))
        .cursor_pointer()
        .text_size(px(11.))
        .text_color(theme::CAPTION())
        .hover(|st| st.text_color(theme::LABEL_3()))
        .debug_selector(move || format!("call-summary-{message_index}"))
        .child(
            // 长文本截断(record_row 正文列同款):窄面板下不让行内容
            // 溢出右缘(工具名串可任意长)
            div().min_w(px(0.)).flex_1().truncate().child(t!(
                "trajectory.folded_tools",
                count = count,
                names = names.join(", ")
            )),
        )
        .on_click(move |_, _, cx| {
            s.update(cx, |st, cx| st.toggle_call(message_index, cx));
        })
}

/// 记录行(30px):event 列 122px(轮角标 + Request 圆点 + kindTag)+
/// content 列(摘要 / 工具双栏)
fn record_row(
    store: &Entity<AppStore>,
    s: &Snap<'_>,
    rec: &TrajectoryRecord,
    turn_start: bool,
) -> impl IntoElement {
    let selected = s.inspector == Some(InspectTarget::Record(rec.index));
    // 选中轮由 `snap()` 每帧算一次(原为每行全表扫)
    let selected_turn = s.inspector_turn.is_some_and(|t| Some(t) == rec.turn);

    let rail_color = if rec.is_error {
        mix(theme::DANGER(), theme::BASE(), 0.22)
    } else {
        mix(theme::BRAND(), theme::BASE(), 0.22)
    };

    // event 列
    let mut event = div()
        .relative()
        .w(px(122.))
        .flex_shrink_0()
        .h_full()
        .flex()
        .items_center()
        .pl(px(36.))
        .pr(px(4.));
    if selected_turn && !selected {
        event = event.child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(2.))
                .bg(rail_color),
        );
    }
    if turn_start && let Some(t) = rec.turn {
        event = event.child(
            div()
                .absolute()
                .left(px(2.))
                .top(px(1.))
                .rounded_b(px(2.))
                .bg(theme::DOCK())
                .px(px(5.))
                .py(px(1.))
                .font_family("Menlo")
                .text_size(px(8.))
                .text_color(theme::LABEL_3())
                .child(t!("trajectory.turn_n", t = t)),
        );
    }
    if let Some(n) = rec.request_number {
        let s2 = store.clone();
        let dot_color = if s
            .view
            .requests
            .iter()
            .any(|q| q.number == n && q.status == "error")
        {
            theme::DANGER()
        } else if s.inspector == Some(InspectTarget::Request(n)) {
            theme::BRAND()
        } else {
            theme::CAPTION()
        };
        let active_req = s.inspector == Some(InspectTarget::Request(n));
        event = event.child(
            div()
                .id(("traj-request", n as usize))
                .size(px(16.))
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .debug_selector(move || format!("traj-request-{n}"))
                .child(
                    div()
                        .size(px(5.))
                        .rounded_full()
                        .bg(dot_color)
                        .when(active_req, |el| el.border_1().border_color(theme::BRAND())),
                )
                .hover(|st| st.opacity(0.85))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    s2.update(cx, |st, cx| st.select_trajectory_request(n, cx));
                }),
        );
    }
    let (fg, bg) = kind_colors(&rec.kind);
    event = event.child(div().flex_1());
    event = event.child(
        div()
            .h(px(19.))
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(3.))
            .rounded(px(4.))
            .bg(bg)
            .px(px(5.))
            .text_size(px(10.))
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .text_color(fg)
            .child(kind_label(&rec.kind).to_string()),
    );

    // content 列
    let content: gpui_kit::AnyElement = if rec.kind == "tool" {
        let (name, args) = split_tool_text(&rec.text);
        let result_color = if rec.is_error {
            theme::DANGER()
        } else if rec.result.as_deref() == Some("No output") {
            theme::CAPTION()
        } else {
            theme::LABEL_3()
        };
        div()
            .flex()
            .min_w(px(0.))
            .flex_1()
            .items_center()
            .gap(px(7.))
            .px(px(8.))
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .font_family("Menlo")
                    .text_color(theme::LABEL_2())
                    .child(name.to_string()),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .truncate()
                    .text_size(px(12.))
                    .font_family("Menlo")
                    .text_color(theme::LABEL_3())
                    .child(args.to_string()),
            )
            .when_some(rec.result.clone(), |el, r| {
                el.child(
                    div()
                        .flex()
                        .min_w(px(0.))
                        .max_w(gpui_kit::relative(0.4))
                        .flex_shrink_0()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme::CAPTION())
                                .child("→"),
                        )
                        .child(
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(px(12.))
                                .text_color(result_color)
                                .child(r),
                        ),
                )
            })
            // 守卫裁决**(本次调用的一个阶段)**:不另立行,行尾一句话
            // 交代结论;细节进检查器「决策」页
            .when_some(rec.decision.clone(), |el, d| {
                el.child(decision_chip(rec.index, &d))
            })
            .into_any_element()
    } else if rec.kind == "decision" {
        // 独立成行的裁决(stop/context,以及 decide 工具——它的 receipt
        // 由引擎在 step 收尾统一落档,到得比它那条调用晚):场景名 +
        // 答案摘要;细节进决策页
        div()
            .min_w(px(0.))
            .flex()
            .flex_1()
            .items_center()
            .gap(px(7.))
            .px(px(8.))
            .child(div().text_size(px(12.)).text_color(theme::LABEL_2()).child(
                decision_scenario_label(rec.decision.as_ref().map(|d| d.scenario.as_str())),
            ))
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(theme::LABEL_3())
                    .child(
                        rec.decision
                            .as_ref()
                            .map(decision_summary)
                            .unwrap_or_default(),
                    ),
            )
            .into_any_element()
    } else if rec.kind == "compacted" {
        // 折叠行:左侧括线 + 富化单行(触发/条数/prefix token/已裁)
        // ——摘要在检查器,这里只留可扫的事实
        let mut row = div()
            .min_w(px(0.))
            .flex()
            .flex_1()
            .items_center()
            .gap(px(7.))
            .px(px(8.));
        if rec.fold.is_some() {
            row = row.child(
                div()
                    .flex_shrink_0()
                    .w(px(2.))
                    .h(px(16.))
                    .rounded(px(1.))
                    .bg(mix(theme::BRAND(), theme::BASE(), 0.45)),
            );
        }
        row = row.child(
            div()
                .min_w(px(0.))
                .flex_1()
                .truncate()
                .text_size(px(12.))
                .text_color(theme::LABEL_3())
                .child(match rec.text.as_str() {
                    "Context compacted" => t!("trajectory.compact_fallback").to_string(),
                    _ => rec.text.clone(),
                }),
        );
        if let Some(f) = &rec.fold {
            row = row.child(fold_chip(rec.index, f));
        }
        row.into_any_element()
    } else {
        let color = match rec.kind.as_str() {
            "user" => theme::LABEL(),
            "message" if rec.text == "(tool call only)" => theme::CAPTION(),
            "message" => theme::LABEL_2(),
            _ => theme::LABEL_3(),
        };
        div()
            .min_w(px(0.))
            .flex_1()
            .truncate()
            .px(px(8.))
            .text_size(px(12.))
            .text_color(color)
            // liuma-core 自产的展示占位在渲染层词典化(检索面仍用线上
            // 原文,见 filter haystack);其余逐字
            .child(match rec.text.as_str() {
                "(tool call only)" => t!("trajectory.tool_call_only").to_string(),
                "Context compacted" => t!("trajectory.compact_fallback").to_string(),
                _ => rec.text.clone(),
            })
            .into_any_element()
    };

    let s2 = store.clone();
    let ix = rec.index;
    div()
        .id(("traj-row", ix as usize))
        .relative()
        .flex()
        .h(px(30.))
        .flex_shrink_0()
        .items_center()
        .when(turn_start, |el| {
            el.border_t_2().border_color(theme::BORDER())
        })
        .when(selected, |el| el.bg(theme::LAYER()))
        .when(!selected, |el| el.hover(|st| st.bg(theme::LAYER())))
        .cursor_pointer()
        .debug_selector(move || format!("trajectory-row-{ix}"))
        .child(event)
        .child(content)
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            s2.update(cx, |st, cx| st.select_trajectory_record(ix, cx));
        })
}

// ── 检查器──────────────────────────────────

/// 检查器标签页集合(按数据在场裁剪;diff_available = 更新记录且
/// 前一 SYSTEM 快照在场)
fn inspector_tabs_for(rec: Option<&TrajectoryRecord>, diff_available: bool) -> Vec<&'static str> {
    let Some(r) = rec else {
        return vec!["summary", "usage", "timing"]; // 请求
    };
    match r.kind.as_str() {
        "tool" => {
            // tab 集:Summary / Payload?/ Result?/ Decision?/ Schema / Timing
            // —— Schema、Timing 恒在(数据缺席由页内缺省文案兜底);
            // Decision 仅在本次调用被裁决过时在场(守卫另一次调用前问过)
            let mut tabs = vec!["summary"];
            if r.payload.is_some() {
                tabs.push("payload");
            }
            if r.result.is_some() || r.output_detail.is_some() {
                tabs.push("result");
            }
            if r.decision.is_some() {
                tabs.push("decision");
            }
            tabs.push("schema");
            tabs.push("timing");
            tabs
        }
        // 无调用可挂的裁决(stop/context)独立成行,本体即决策页
        "decision" => vec!["decision"],
        "message" => {
            // message 恒三页:[Summary, Preview, Raw]
            // (内容缺席由页内缺省文案兜底)
            vec!["summary", "preview", "raw"]
        }
        "context" => {
            // context 基础三页 [Summary, Preview, Raw],
            // source 染色在场 → 尾加 Source
            let mut tabs = vec!["summary", "preview", "raw"];
            if r.source.is_some() {
                tabs.push("source");
            }
            tabs
        }
        "user" => vec!["summary", "payload"],
        "system" => {
            // 无 Summary 页:Diff 在场时置首,System Prompt + Tools 恒在
            // (老日志无快照由页内缺省文案兜底)
            let mut tabs = Vec::new();
            if diff_available {
                tabs.push("diff");
            }
            tabs.push("system");
            tabs.push("tools");
            tabs
        }
        "compacted" => {
            // 折叠事实页置首(那是这次压缩「为什么/怎么压」的答案);
            // 摘要在 Summary、裁定 receipt 在 Decision
            let mut tabs = Vec::new();
            if r.fold.is_some() {
                tabs.push("fold");
            }
            tabs.push("summary");
            if r.output_detail.is_some() {
                tabs.push("result");
            }
            if r.decision.is_some() {
                tabs.push("decision");
            }
            tabs
        }
        _ => vec!["summary"],
    }
}

/// 台账表最小渲染宽:检查器让位的下限(面板让位到 320 时检查器
/// 至多 120,右缘不再溢出面板被裁)
const LEDGER_MIN_W: f32 = 200.;

fn inspector(
    store: &Entity<AppStore>,
    window: &mut Window,
    cx: &mut App,
) -> Option<impl IntoElement> {
    let s = snap(store.read(cx));
    let target = s.inspector?;
    // 渲染期钳制检查器宽:存储宽(inspector_width,拖宽协商 320..720)
    // 不知道面板当前渲染宽(面板让位可压到 320),固定宽 + flex_shrink_0
    // 会把行区顶穿、检查器右缘被面板 overflow_hidden 裁掉。与面板拖宽
    // 同哲学(metrics::panel_width_for 注释):钳制只是视图让位,不改
    // 存储值——窗口拉宽自然回意愿宽
    let (panel_open, panel_px, sidebar_collapsed, sidebar_px) = {
        let st = store.read(cx);
        (
            st.panel_open,
            st.panel_px,
            st.sidebar_collapsed,
            st.sidebar_px,
        )
    };
    let panel_w = f32::from(crate::shell::metrics::panel_width_for(
        panel_open,
        panel_px,
        f32::from(window.viewport_size().width),
        sidebar_collapsed,
        sidebar_px,
    ));
    let inspector_w = s.inspector_width.min((panel_w - LEDGER_MIN_W).max(0.));
    let record = match target {
        InspectTarget::Record(ix) => s.index.and_then(|i| i.get(&s.view.records, ix)),
        InspectTarget::Request(_) => None,
    };
    let request = match target {
        InspectTarget::Request(n) => s.view.requests.iter().find(|q| q.number == n),
        InspectTarget::Record(_) => record
            .and_then(|r| r.request_number)
            .and_then(|n| s.view.requests.iter().find(|q| q.number == n)),
    };

    let diff_available = record.is_some_and(|r| {
        r.kind == "system"
            && r.text != "Initial System Prompt"
            && s.index
                .is_some_and(|i| i.previous_system_snapshot(&s.view.records, r).is_some())
    });
    let tabs = inspector_tabs_for(record, diff_available);
    let active = s
        .inspector_tab
        .filter(|t| tabs.contains(t))
        .unwrap_or_else(|| {
            if tabs.contains(&s.last_tab) {
                s.last_tab
            } else {
                // 记忆页不在 tab 集合 → 回退首页
                tabs.first().copied().unwrap_or("summary")
            }
        });

    // 头(请求 = 圆点 + Request #N + Turn;记录 = kindTag + Turn · Step)
    let header: gpui_kit::AnyElement = match (target, record, request) {
        (InspectTarget::Request(n), _, Some(q)) => div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .size(px(5.))
                    .rounded_full()
                    .bg(if q.status == "error" {
                        theme::DANGER()
                    } else {
                        theme::BRAND()
                    }),
            )
            .child(
                div()
                    .font_family("Menlo")
                    .text_size(px(12.))
                    .text_color(theme::LABEL_2())
                    .child(t!("trajectory.request_n", n = n)),
            )
            .child(
                div()
                    .font_family("Menlo")
                    .text_size(px(11.))
                    .text_color(theme::CAPTION())
                    .child(t!("trajectory.turn_n", t = q.turn)),
            )
            .into_any_element(),
        (_, Some(r), _) => {
            let (fg, bg) = kind_colors(&r.kind);
            let location = match (&r.turn, r.group.as_str()) {
                (Some(t), g) if g.starts_with("Step") => t!("trajectory.turn_at", t = t, at = g),
                (Some(t), _) => t!("trajectory.turn_message", t = t),
                _ => t!("trajectory.between_turns"),
            };
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .h(px(19.))
                        .flex()
                        .items_center()
                        .rounded(px(4.))
                        .bg(bg)
                        .px(px(5.))
                        .text_size(px(10.))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .child(kind_label(&r.kind).to_string()),
                )
                .child(
                    div()
                        .font_family("Menlo")
                        .text_size(px(11.))
                        .text_color(theme::CAPTION())
                        .child(location),
                )
                .into_any_element()
        }
        _ => div().into_any_element(),
    };

    let close_store = store.clone();
    let body: gpui_kit::AnyElement = match (record, request, active) {
        // ── 请求 ──
        (None, Some(q), "usage") => usage_body(q).into_any_element(),
        (None, Some(q), "timing") => request_timing_body(q).into_any_element(),
        (None, Some(q), _) => request_summary_body(store, &s, q).into_any_element(),
        // ── 记录 ──
        (Some(r), _, "payload") => payload_body(store, &s, r).into_any_element(),
        (Some(r), _, "result") => result_body(store, &s, r).into_any_element(),
        (Some(r), _, "raw") => raw_body(store, &s, r).into_any_element(),
        (Some(r), _, "preview") => match r.kind.as_str() {
            "message" => assistant_preview_body(store, &s, r).into_any_element(),
            _ => preview_tab_body(r).into_any_element(),
        },
        (Some(r), _, "source") => source_tab_body(r).into_any_element(),
        (Some(r), _, "system") => system_body(r).into_any_element(),
        (Some(r), _, "tools") => tools_body(store, &s, r).into_any_element(),
        (Some(r), _, "diff") => diff_body(&s, r).into_any_element(),
        (Some(r), _, "schema") => schema_body(store, &s, r).into_any_element(),
        (Some(r), _, "decision") => decision_tab_body(r).into_any_element(),
        (Some(r), _, "fold") => fold_tab_body(r).into_any_element(),
        (Some(r), _, "timing") => timing_body(r).into_any_element(),
        (Some(r), _, _) => summary_body(store, &s, r).into_any_element(),
        // 目标数据已不在窗口(翻页/直播后):占位
        (None, None, _) => div()
            .child(empty_text(t!("trajectory.na")))
            .into_any_element(),
    };

    let resize_store = store.clone();
    Some(
        div()
            .id("trajectory-inspector")
            .relative()
            .flex_shrink_0()
            .v_flex()
            .min_h(px(0.))
            .w(px(inspector_w))
            // 水平裁剪收在检查器自身:内部 json/mono 单行不折行,无此
            // 约束会长画到面板右缘才被 trajectory-view 裁掉
            .overflow_hidden()
            .border_l_1()
            .border_color(theme::BORDER())
            .bg(theme::LAYER())
            .debug_selector(|| "trajectory-inspector".to_string())
            // 左缘拖宽把手(320..720;move/up 在 render 期窗口级注册)
            .child(
                div()
                    .id("inspector-resize")
                    .absolute()
                    .left(px(-4.))
                    .top_0()
                    .bottom_0()
                    .w(px(8.))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        resize_store.update(cx, |st, cx| {
                            st.inspector_resize_begin(f32::from(ev.position.x), cx)
                        });
                    }),
            )
            .child(
                div()
                    .flex()
                    .h(px(42.))
                    .flex_shrink_0()
                    .items_center()
                    .px(px(12.))
                    .gap(px(4.))
                    .border_b_1()
                    .border_color(theme::BORDER())
                    .child(header)
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("inspector-close")
                            .flex()
                            .size(px(24.))
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_color(theme::LABEL_3())
                            .hover(|st| st.bg(theme::DOCK()))
                            .child(fixed(IconName::Close, 14.))
                            .on_click(move |_, _, cx| {
                                close_store.update(cx, |st, cx| st.close_inspector(cx));
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .h(px(34.))
                    .flex_shrink_0()
                    .items_center()
                    .gap(px(2.))
                    .px(px(8.))
                    .border_b_1()
                    .border_color(theme::BORDER())
                    .children(tabs.iter().enumerate().map(|(ti, t)| {
                        let s2 = store.clone();
                        let name = *t;
                        let is_active = name == active;
                        div()
                            .id(("inspector-tab", ti))
                            .flex()
                            .h(px(24.))
                            .items_center()
                            .px(px(8.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_size(px(12.))
                            .when(is_active, |el| {
                                el.bg(theme::GLASS_BG())
                                    .text_color(theme::LABEL())
                                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                            })
                            .when(!is_active, |el| {
                                el.text_color(theme::LABEL_3())
                                    .hover(|st| st.text_color(theme::LABEL_2()))
                            })
                            .debug_selector(move || format!("inspector-tab-{name}"))
                            .child(tab_label(name).to_string())
                            .on_click(move |_, _, cx| {
                                s2.update(cx, |st, cx| st.set_inspector_tab(name, cx));
                            })
                    })),
            )
            .child(
                div()
                    .id("inspector-body")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .p(px(12.))
                    .child(body),
            ),
    )
}

fn tab_label(name: &str) -> std::borrow::Cow<'static, str> {
    match name {
        "payload" => t!("trajectory.tab_payload"),
        "result" => t!("trajectory.tab_result"),
        "timing" => t!("trajectory.tab_timing"),
        "raw" => t!("trajectory.tab_raw"),
        "usage" => t!("trajectory.tab_usage"),
        "system" => t!("trajectory.tab_system_prompt"),
        "preview" => t!("trajectory.tab_preview"),
        "source" => t!("trajectory.tab_source"),
        "tools" => t!("trajectory.tab_tools"),
        "diff" => t!("trajectory.tab_diff"),
        "schema" => t!("trajectory.tab_schema"),
        "decision" => t!("trajectory.tab_decision"),
        "fold" => t!("trajectory.tab_fold"),
        _ => t!("trajectory.tab_summary"),
    }
}

// ── 检查器主体(tab 内容)──────────────────────────────────────

/// 信息行(96px 标签列)
fn dl_row(label: impl Into<gpui_kit::SharedString>, value: impl IntoElement) -> Div {
    let label = label.into();
    div()
        .flex()
        .items_start()
        .gap(px(8.))
        .py(px(2.))
        .child(
            div()
                .w(px(96.))
                .flex_shrink_0()
                .text_size(px(11.))
                .text_color(theme::CAPTION())
                .child(label.to_string()),
        )
        .child(
            div()
                .min_w(px(0.))
                .flex_1()
                .text_size(px(12.))
                .text_color(theme::LABEL_2())
                .line_height(gpui_kit::relative(1.5))
                .child(value),
        )
}

/// 小节标题(Stateful 供调用方链 on_click)
fn section(id: &'static str, title: impl Into<gpui_kit::SharedString>) -> gpui_kit::Stateful<Div> {
    let title = title.into();
    let sel = id.to_string();
    // 标题 + `>` 跳转箭头(点击进完整 tab;调用方挂 on_click)
    div()
        .id(id)
        .mt(px(6.))
        .mb(px(2.))
        .flex()
        .items_center()
        .gap(px(2.))
        .cursor_pointer()
        .text_size(px(11.))
        .font_weight(gpui_kit::FontWeight::MEDIUM)
        .text_color(theme::CAPTION())
        .hover(|st| st.text_color(theme::LABEL_3()))
        .child(title.to_string())
        .child(fixed(IconName::ChevronRight, 11.).text_color(theme::CAPTION()))
        .debug_selector(move || sel.clone())
}

/// 层级跳转链接(文字 + 小箭头)
fn nav_link(id: &'static str, text: impl Into<gpui_kit::SharedString>) -> gpui_kit::Stateful<Div> {
    let text = text.into();
    let sel = id.to_string();
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(2.))
        .font_family("Menlo")
        .text_size(px(12.))
        .text_color(theme::BRAND())
        .cursor_pointer()
        .hover(|st| st.opacity(0.8))
        .child(text)
        .child(fixed(IconName::ChevronRight, 10.).text_color(theme::BRAND()))
        .debug_selector(move || sel.clone())
}

/// 等宽文本块(限高容器内滚动;长非断行 token 水平裁剪不折行——
/// 溢出由检查器 overflow_hidden 收口)
fn mono_block(id: impl Into<gpui_kit::ElementId>, text: &str, color: Rgba) -> impl IntoElement {
    div()
        .id(id)
        .min_w(px(0.))
        .overflow_hidden()
        .rounded(px(8.))
        .bg(theme::CODE())
        .p(px(10.))
        .text_size(px(12.))
        .text_color(color)
        .font_family("Menlo")
        .line_height(gpui_kit::relative(1.55))
        .child(text.to_string())
}

/// JSON 语法高亮 token 类别
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JKind {
    /// 对象键(`"…":`)
    Key,
    /// 字符串值
    Str,
    /// 数字
    Num,
    /// true / false / null
    Kw,
    /// 标点/缩进/其他
    Plain,
}

/// 单行 JSON 分词(键 = 字符串后紧跟冒号;转义引号不截断字符串)
fn json_tokens(line: &str) -> Vec<(JKind, String)> {
    fn flush(plain: &mut String, out: &mut Vec<(JKind, String)>) {
        if !plain.is_empty() {
            out.push((JKind::Plain, std::mem::take(plain)));
        }
    }
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            let i = i.min(chars.len());
            let s: String = chars[start..i].iter().collect();
            // 后续(跳空白)是冒号 → 键
            let mut j = i;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            let is_key = j < chars.len() && chars[j] == ':';
            flush(&mut plain, &mut out);
            out.push((if is_key { JKind::Key } else { JKind::Str }, s));
        } else if c.is_ascii_digit()
            || (c == '-' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
        {
            let start = i;
            i += 1;
            while i < chars.len()
                && (chars[i].is_ascii_digit() || matches!(chars[i], '.' | 'e' | 'E' | '+' | '-'))
            {
                i += 1;
            }
            flush(&mut plain, &mut out);
            out.push((JKind::Num, chars[start..i].iter().collect()));
        } else if c.is_ascii_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            let w: String = chars[start..i].iter().collect();
            flush(&mut plain, &mut out);
            let kind = match w.as_str() {
                "true" | "false" | "null" => JKind::Kw,
                _ => JKind::Plain,
            };
            out.push((kind, w));
        } else {
            plain.push(c);
            i += 1;
        }
    }
    flush(&mut plain, &mut out);
    out
}

fn jkind_color(kind: JKind) -> Rgba {
    match kind {
        JKind::Key => theme::BRAND(),
        JKind::Str => JSON_STRING(),
        JKind::Num => JSON_NUMBER(),
        JKind::Kw => ASSISTANT_VIOLET(),
        JKind::Plain => theme::LABEL_3(),
    }
}

// ── JSON 树────────────────────────────────────────
//
// 顶层恒展开、子级默认折叠(展开集合在 store);可展开节点 = 箭头 +
// 键名 + 单行内联预览(object ≤4 项 / array ≤5 项 / 深度 ≤2,超出 …);
// 键名无引号;12px/16px code,token 分色随主题双盘。行内 token 为
// 固有宽 flex 子项,长行溢出裁切不折行(单行语义)。

fn jt_span(color: Rgba, text: impl Into<String>) -> gpui_kit::AnyElement {
    div()
        .text_color(color)
        .child(text.into())
        .into_any_element()
}

fn jt_row(depth: usize, children: Vec<gpui_kit::AnyElement>) -> gpui_kit::AnyElement {
    div()
        .flex()
        .min_w(px(0.))
        .overflow_hidden()
        .items_start()
        .when(depth > 0, |el| el.pl(px(14. * depth as f32)))
        .children(children)
        .into_any_element()
}

fn json_brackets(v: &serde_json::Value) -> (&'static str, &'static str) {
    if v.is_array() { ("[", "]") } else { ("{", "}") }
}

fn json_entries(v: &serde_json::Value) -> Vec<(String, &serde_json::Value)> {
    match v {
        serde_json::Value::Object(m) => m.iter().map(|(k, val)| (k.clone(), val)).collect(),
        serde_json::Value::Array(a) => a
            .iter()
            .enumerate()
            .map(|(i, val)| (i.to_string(), val))
            .collect(),
        _ => Vec::new(),
    }
}

/// 节点路径编码:数组下标 `n{i}`,字符串键 `s{len}:{key}`
fn jt_child_path(key_path: &str, key: &str, index: usize, is_array: bool) -> String {
    if is_array {
        format!("{key_path}/n{index}")
    } else {
        format!("{key_path}/s{}:{}", key.chars().count(), key)
    }
}

fn json_leaf_token(value: &serde_json::Value) -> (Rgba, String) {
    match value {
        serde_json::Value::String(s) => {
            (JSON_STRING(), serde_json::to_string(s).unwrap_or_default())
        }
        serde_json::Value::Number(n) => (JSON_NUMBER(), n.to_string()),
        serde_json::Value::Bool(b) => (JSON_NUMBER(), b.to_string()),
        serde_json::Value::Null => (JSON_NUMBER(), "null".into()),
        _ => (JSON_PUNCT(), value.to_string()),
    }
}

/// 折叠态单行内联预览;键名取标点色,深度 ≥2 的容器只显示 `{…}`
fn json_preview_tokens(value: &serde_json::Value, depth: usize) -> Vec<(Rgba, String)> {
    let mut out = Vec::new();
    match value {
        serde_json::Value::Object(m) => {
            out.push((JSON_PUNCT(), "{".into()));
            let limit = 4;
            if depth < 2 && !m.is_empty() {
                for (i, (k, v)) in m.iter().enumerate() {
                    if i >= limit {
                        out.push((JSON_PUNCT(), ", …".into()));
                        break;
                    }
                    if i > 0 {
                        out.push((JSON_PUNCT(), ", ".into()));
                    }
                    out.push((JSON_PUNCT(), format!("{k}: ")));
                    out.extend(json_preview_tokens(v, depth + 1));
                }
            } else if !m.is_empty() {
                out.push((JSON_PUNCT(), "…".into()));
            }
            out.push((JSON_PUNCT(), "}".into()));
        }
        serde_json::Value::Array(a) => {
            out.push((JSON_PUNCT(), "[".into()));
            let limit = 5;
            if depth < 2 && !a.is_empty() {
                for (i, v) in a.iter().enumerate() {
                    if i >= limit {
                        out.push((JSON_PUNCT(), ", …".into()));
                        break;
                    }
                    if i > 0 {
                        out.push((JSON_PUNCT(), ", ".into()));
                    }
                    out.extend(json_preview_tokens(v, depth + 1));
                }
            } else if !a.is_empty() {
                out.push((JSON_PUNCT(), "…".into()));
            }
            out.push((JSON_PUNCT(), "]".into()));
        }
        _ => {
            let (c, t) = json_leaf_token(value);
            out.push((c, t));
        }
    }
    out
}

/// JSON 树渲染上下文(store/记录索引随递归不变)
struct JtCtx<'a, 'b> {
    store: &'a Entity<AppStore>,
    s: &'a Snap<'b>,
    ix: u64,
}

/// 单个子树的全部行(header 行 + 展开时的 children)
fn json_tree_rows(
    ctx: &JtCtx,
    key_path: String,
    field: Option<&str>,
    value: &serde_json::Value,
    last: bool,
    depth: usize,
) -> Vec<gpui_kit::AnyElement> {
    let JtCtx { store, s, ix } = *ctx;
    let mut row: Vec<gpui_kit::AnyElement> = Vec::new();
    if let Some(f) = field {
        row.push(jt_span(JSON_PROPERTY(), format!("{f}:")));
    }
    match value {
        serde_json::Value::Object(_) | serde_json::Value::Array(_)
            if !json_entries(value).is_empty() => {}
        _ => {
            // 叶子 / 空容器
            if value.is_object() || value.is_array() {
                let (o, c) = json_brackets(value);
                row.push(jt_span(JSON_PUNCT(), format!("{o}{c}")));
            } else {
                let (c, t) = json_leaf_token(value);
                row.push(jt_span(c, t));
            }
            if !last {
                row.push(jt_span(JSON_PUNCT(), ","));
            }
            return vec![jt_row(depth, row)];
        }
    }

    let expanded = s.json_expanded.contains(&format!("{ix}/{key_path}"));
    let (o, c) = json_brackets(value);
    let s2 = store.clone();
    let k2 = key_path.clone();
    row.push(
        div()
            .id(format!("jt-{ix}-{key_path}"))
            .flex_shrink_0()
            .cursor_pointer()
            .text_color(JSON_EXPANDER())
            .child(fixed(
                if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                },
                10.,
            ))
            .on_click(move |_, _, cx| {
                let k = format!("{ix}/{k2}");
                s2.update(cx, |st, cx| st.toggle_json_node(&k, cx));
            })
            .into_any_element(),
    );
    row.push(jt_span(JSON_PUNCT(), o));
    for t in json_preview_tokens(value, 0) {
        row.push(jt_span(t.0, t.1));
    }
    row.push(jt_span(JSON_PUNCT(), c));
    if !last {
        row.push(jt_span(JSON_PUNCT(), ","));
    }
    let mut rows = vec![jt_row(depth, row)];
    if expanded {
        let is_array = value.is_array();
        let entries = json_entries(value);
        let n = entries.len();
        for (i, (k, v)) in entries.into_iter().enumerate() {
            let child_path = jt_child_path(&key_path, &k, i, is_array);
            rows.extend(json_tree_rows(
                ctx,
                child_path,
                Some(&k),
                v,
                i == n - 1,
                depth + 1,
            ));
        }
    }
    rows
}

/// JSON 树整体(顶层 `{` / children / `}`;调用方保证 object/array)
fn json_tree_block(
    store: &Entity<AppStore>,
    s: &Snap<'_>,
    ix: u64,
    value: &serde_json::Value,
) -> Div {
    let ctx = JtCtx { store, s, ix };
    let is_array = value.is_array();
    let entries = json_entries(value);
    let n = entries.len();
    let (open, close) = json_brackets(value);
    let mut rows: Vec<gpui_kit::AnyElement> = vec![jt_span(JSON_PUNCT(), open)];
    for (i, (k, v)) in entries.into_iter().enumerate() {
        let path = jt_child_path("", &k, i, is_array);
        rows.extend(json_tree_rows(&ctx, path, Some(&k), v, i == n - 1, 1));
    }
    rows.push(jt_span(JSON_PUNCT(), close));
    div()
        .v_flex()
        .text_size(px(12.))
        .font_family("Menlo")
        .line_height(gpui_kit::relative(1.33))
        .children(rows)
}

/// JSON 高亮块:逐行分色 token(pretty JSON 行短,行内不换行)。
/// 高度不封顶——检查器主体(inspector-body)整页滚,内容全高展开
fn json_block(id: &'static str, text: &str) -> impl IntoElement {
    div()
        .id(id)
        .min_w(px(0.))
        .overflow_hidden()
        .rounded(px(8.))
        .bg(theme::CODE())
        .p(px(10.))
        .v_flex()
        .text_size(px(12.))
        .font_family("Menlo")
        .line_height(gpui_kit::relative(1.55))
        .children(text.lines().map(|line| {
            // 行内 token 为固有宽 flex 子项,长行溢出裁切不折行(单行
            // 语义)——由本块 overflow_hidden 收口
            div().flex().min_w(px(0.)).overflow_hidden().children(
                json_tokens(line)
                    .into_iter()
                    .map(|(kind, s)| div().text_color(jkind_color(kind)).child(s))
                    .collect::<Vec<_>>(),
            )
        }))
}

/// 代码块选择:内容为 JSON 对象/数组 → 高亮;否则等宽纯文本
fn code_block(id: &'static str, text: &str, plain_color: Rgba) -> gpui_kit::AnyElement {
    let is_json = serde_json::from_str::<serde_json::Value>(text)
        .map(|v| v.is_object() || v.is_array())
        .unwrap_or(false);
    if is_json {
        json_block(id, text).into_any_element()
    } else {
        mono_block(id, text, plain_color).into_any_element()
    }
}

/// JSON 美化(失败原样)
fn pretty_json(s: &str) -> String {
    serde_json::from_str::<serde_json::Value>(s)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .unwrap_or_else(|_| s.to_string())
}

/// 记录所属请求:message 优先 request_number;工具按 turn + Step N 归属
fn owning_request<'a>(
    r: &TrajectoryRecord,
    requests: &'a [TrajectoryRequest],
) -> Option<&'a TrajectoryRequest> {
    if r.kind == "message"
        && let Some(n) = r.request_number
        && let Some(q) = requests.iter().find(|q| q.number == n)
    {
        return Some(q);
    }
    let step = r.group.strip_prefix("Step ")?.parse::<u64>().ok()?;
    requests
        .iter()
        .find(|q| q.turn == r.turn.unwrap_or(0) && q.step == step)
}

/// Summary tab(记录):
/// Hierarchy 跳转 → Status(工具含 Pending)→ Tokens/Duration →
/// Payload/Result 预览 → Request Timing / Timing 小节
fn summary_body(store: &Entity<AppStore>, s: &Snap<'_>, r: &TrajectoryRecord) -> Div {
    let requests = &s.view.requests;
    let req = owning_request(r, requests);
    let mut col = div().v_flex().gap(px(2.));

    // CONTEXT:Source › / Status / Duration + Preview 小节;
    // Preview › 跳渲染页
    if r.kind == "context" {
        if let Some(source) = &r.source {
            let s2 = store.clone();
            col = col.child(dl_row(
                t!("trajectory.row_source"),
                nav_link("goto-source", message_source_label(source)).on_click(move |_, _, cx| {
                    s2.update(cx, |st, cx| st.set_inspector_tab("source", cx));
                }),
            ));
        }
        col = col.child(dl_row(
            t!("trajectory.row_status"),
            t!("trajectory.status_completed"),
        ));
        col = col.child(dl_row(
            t!("trajectory.row_duration"),
            fmt_ms(rec_total_ms(r).unwrap_or(0)),
        ));
        let s2 = store.clone();
        col = col
            .child(
                section("sec-preview", t!("trajectory.tab_preview")).on_click(move |_, _, cx| {
                    s2.update(cx, |st, cx| st.set_inspector_tab("preview", cx));
                }),
            )
            .child(context_markdown_body(
                r,
                &format!("traj-ctx-prev-{}", r.index),
            ));
        return col;
    }

    // Hierarchy:Request #N(所属请求)+ Assistant Message(工具发起消息;
    // 无子工具调用数据,不渲染嵌套链接)
    let parent = if r.kind == "tool" {
        s.index.and_then(|i| i.parent_message(&s.view.records, r))
    } else {
        None
    };
    if req.is_some() || parent.is_some() {
        let mut dd = div().v_flex().gap(px(2.));
        if let Some(q) = req {
            let s2 = store.clone();
            let n = q.number;
            dd = dd.child(
                nav_link("goto-request", t!("trajectory.request_n", n = n)).on_click(
                    move |_, _, cx| {
                        s2.update(cx, |st, cx| st.select_trajectory_request(n, cx));
                    },
                ),
            );
        }
        if let Some(p) = parent {
            let s2 = store.clone();
            let ix = p.index;
            dd = dd.child(
                nav_link("goto-message", t!("trajectory.nav_assistant_message")).on_click(
                    move |_, _, cx| {
                        s2.update(cx, |st, cx| st.select_trajectory_record(ix, cx));
                    },
                ),
            );
        }
        // 列名:所属请求在场 = 来源,否则 层级
        let dt = if req.is_some() {
            t!("trajectory.row_source")
        } else {
            t!("trajectory.row_hierarchy")
        };
        col = col.child(dl_row(dt, dd.into_any_element()));
    }

    // Status(message:Failed 红 / Completed)
    if r.kind == "message" {
        let (label, color) = if r.is_error {
            (t!("trajectory.status_failed"), Some(theme::DANGER()))
        } else {
            (t!("trajectory.status_completed"), None)
        };
        col = col.child(dl_row(
            t!("trajectory.row_status"),
            div()
                .when_some(color, |el, c| el.text_color(c))
                .child(label),
        ));
    }

    // Preview 小节(message 首节,内容 = rendered
    // preview 形态:Thinking 折叠 + 正文 + 工具调用行)
    if r.kind == "message" {
        let s2 = store.clone();
        col = col
            .child(
                section("sec-preview", t!("trajectory.tab_preview")).on_click(move |_, _, cx| {
                    s2.update(cx, |st, cx| st.set_inspector_tab("preview", cx));
                }),
            )
            .child(assistant_preview_body(store, s, r));
    }

    // Status(Failed 红 / Pending 无结果 / Completed)
    if r.kind == "tool" {
        let (label, color) = if r.is_error {
            (t!("trajectory.status_failed"), Some(theme::DANGER()))
        } else if r.result.is_none() {
            (t!("trajectory.status_pending"), Some(theme::WARN()))
        } else {
            (t!("trajectory.status_completed"), None)
        };
        col = col.child(dl_row(
            t!("trajectory.row_status"),
            div()
                .when_some(color, |el, c| el.text_color(c))
                .child(label),
        ));
    }

    // Tokens(message)
    if r.kind == "message" && r.output.is_some() {
        let out = r.output.unwrap_or(0);
        let think = r.think.unwrap_or(0);
        col = col
            .child(dl_row(
                t!("trajectory.row_tokens"),
                format!("{} tok", fmt_tok(out)),
            ))
            .child(dl_row(t!("trajectory.row_reasoning"), fmt_tok(think)))
            .child(dl_row(
                t!("trajectory.row_content"),
                fmt_tok(out.saturating_sub(think)),
            ));
    }
    // Duration(user)
    if r.kind == "user"
        && let Some(sec) = r.time_seconds
    {
        col = col.child(dl_row(
            t!("trajectory.row_duration"),
            fmt_ms((sec * 1000.) as i64),
        ));
    }

    // Payload / Result / Schema 小节(非 markdown 记录专属——
    // 工具恒渲染,缺席显示缺省文案;message 走 Preview/Raw 不进这里,
    // 否则跳转会落进 tab 集不存在的孤页)
    if r.kind != "message" && (r.kind == "tool" || r.payload.is_some()) {
        let s2 = store.clone();
        col = col
            .child(
                section("sec-payload", t!("trajectory.tab_payload")).on_click(move |_, _, cx| {
                    s2.update(cx, |st, cx| st.set_inspector_tab("payload", cx));
                }),
            )
            .child(preview_block(store, s, r, "payload"));
    }
    if r.kind != "message" && (r.kind == "tool" || r.result.is_some() || r.output_detail.is_some())
    {
        let s2 = store.clone();
        col = col
            .child(
                section("sec-result", t!("trajectory.tab_result")).on_click(move |_, _, cx| {
                    s2.update(cx, |st, cx| st.set_inspector_tab("result", cx));
                }),
            )
            .child(preview_block(store, s, r, "result"));
    }
    if r.kind == "tool" {
        let s2 = store.clone();
        col = col
            .child(
                section("sec-schema", t!("trajectory.tab_schema")).on_click(move |_, _, cx| {
                    s2.update(cx, |st, cx| st.set_inspector_tab("schema", cx));
                }),
            )
            .child(preview_block(store, s, r, "schema"));
    }

    let total = rec_total_ms(r);

    // Request Timing 小节(所属请求存在;标题点击 → 请求检查器 Timing;
    // 内容 = 本记录计时:助手含生成指标,工具为 Started/Duration)
    if let Some(q) = req {
        let s2 = store.clone();
        let n = q.number;
        let timing_rows: Vec<Div> = if r.kind == "message" {
            let generation = match (r.ttft_ms, total) {
                (Some(t), Some(ms)) if ms > t => fmt_ms(ms - t),
                _ => "—".into(),
            };
            let throughput = match (r.output, total) {
                (Some(tok), Some(ms)) if ms > 0 => {
                    format!("{:.1} tok/s", tok as f64 / (ms as f64 / 1000.))
                }
                _ => "—".into(),
            };
            vec![
                dl_row(
                    t!("trajectory.row_started"),
                    r.started_at.map(fmt_clock).unwrap_or_else(|| "—".into()),
                ),
                dl_row(t!("trajectory.row_total"), fmt_ms(q.duration_ms)),
                dl_row("TTFT", r.ttft_ms.map(fmt_ms).unwrap_or_else(|| "—".into())),
                dl_row(t!("trajectory.row_generation"), generation),
                dl_row(t!("trajectory.row_throughput"), throughput),
                dl_row(t!("trajectory.tab_timing"), timing_source(total.is_some())),
            ]
        } else {
            vec![
                dl_row(
                    t!("trajectory.row_started"),
                    r.started_at.map(fmt_clock).unwrap_or_else(|| "—".into()),
                ),
                dl_row(
                    t!("trajectory.row_duration"),
                    total.map(fmt_ms).unwrap_or_else(|| "—".into()),
                ),
                dl_row(t!("trajectory.tab_timing"), timing_source(total.is_some())),
            ]
        };
        col = col
            .child(
                section("sec-req-timing", t!("trajectory.sec_request_timing")).on_click(
                    move |_, _, cx| {
                        s2.update(cx, |st, cx| {
                            st.select_trajectory_request(n, cx);
                            st.set_inspector_tab("timing", cx);
                        });
                    },
                ),
            )
            .children(timing_rows);
    }

    // Timing 小节(工具;三行:Started / Duration / Timing source)
    if r.kind == "tool" {
        let s2 = store.clone();
        let timing_sec =
            section("sec-timing", t!("trajectory.tab_timing")).on_click(move |_, _, cx| {
                s2.update(cx, |st, cx| st.set_inspector_tab("timing", cx));
            });
        col = col
            .child(timing_sec)
            .child(dl_row(
                t!("trajectory.row_started"),
                r.started_at.map(fmt_clock).unwrap_or_else(|| "—".into()),
            ))
            .child(dl_row(
                t!("trajectory.row_duration"),
                total.map(fmt_ms).unwrap_or_else(|| "—".into()),
            ))
            .child(dl_row(
                t!("trajectory.row_timing_source"),
                timing_source(total.is_some()),
            ));
    }
    col
}

/// 预览块(全文内部滚;标题 `>` 负责跳转,预览本身不抢点击)。
/// 缺席显示缺省文案(No payload / No result / Schema unavailable);
/// JSON 容器内容走 JsonTree 紧凑形态
fn preview_block(
    store: &Entity<AppStore>,
    s: &Snap<'_>,
    r: &TrajectoryRecord,
    tab: &'static str,
) -> impl IntoElement {
    let missing = match tab {
        "payload" => t!("trajectory.no_payload"),
        "result" => t!("trajectory.no_result"),
        _ => t!("trajectory.schema_na"),
    };
    let text = match tab {
        // SYSTEM:快照在场时预览真实 prompt 头部(字符数行只是信封摘要)
        "payload" if r.kind == "system" => r
            .system_prompt
            .clone()
            .or_else(|| r.payload.clone())
            .unwrap_or_default(),
        "payload" => r.payload.clone().unwrap_or_default(),
        "result" => r
            .output_detail
            .clone()
            .or_else(|| r.result.clone())
            .unwrap_or_default(),
        _ => r.schema_detail.clone().unwrap_or_default(),
    };
    // Schema 小节:工具名 + 描述 + Parameters 树
    if tab == "schema"
        && let Some(spec) = serde_json::from_str::<serde_json::Value>(&text).ok()
        && spec.is_object()
    {
        let name = spec_name(&spec);
        let description = spec_field(&spec, "description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if let Some(params) = spec_field(&spec, "parameters") {
            let mut col = div().v_flex();
            if !name.is_empty() {
                col = col.child(
                    div()
                        .text_size(px(12.))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(theme::LABEL())
                        .child(name),
                );
            }
            if !description.is_empty() {
                col = col.child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme::LABEL_2())
                        .line_height(gpui_kit::relative(1.5))
                        .child(description),
                );
            }
            col = col
                .child(
                    div()
                        .mt(px(4.))
                        .text_size(px(11.))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(theme::CAPTION())
                        .child(t!("trajectory.sec_parameters")),
                )
                .child(json_tree_block(store, s, r.index, params));
            return div()
                .id(("sec-tree", r.index))
                .max_h(px(96.))
                .overflow_y_scroll()
                .child(col);
        }
    }
    let shown = if text.is_empty() {
        missing.to_string()
    } else {
        text
    };
    let s2 = store.clone();
    div()
        .id((tab, 1usize))
        .max_h(px(96.))
        .overflow_y_scroll()
        .rounded(px(8.))
        .bg(theme::CODE())
        .p(px(10.))
        .text_size(px(12.))
        .text_color(theme::LABEL_3())
        .font_family("Menlo")
        .line_height(gpui_kit::relative(1.55))
        .child(shown)
        .on_click(move |_, _, cx| {
            s2.update(cx, |st, cx| st.set_inspector_tab(tab, cx));
        })
}

/// Payload tab(JSON 容器 → JsonTree;否则原文等宽)
fn payload_body(store: &Entity<AppStore>, s: &Snap<'_>, r: &TrajectoryRecord) -> Div {
    match &r.payload {
        None => div().child(empty_text(t!("trajectory.no_payload"))),
        Some(p) => match serde_json::from_str::<serde_json::Value>(p) {
            Ok(v) if v.is_object() || v.is_array() => {
                div().child(json_tree_block(store, s, r.index, &v))
            }
            _ => div().child(code_block(
                "mono-payload",
                &pretty_json(p),
                theme::LABEL_3(),
            )),
        },
    }
}

/// Result tab(错误全套红;JSON 容器 → JsonTree)
fn result_body(store: &Entity<AppStore>, s: &Snap<'_>, r: &TrajectoryRecord) -> Div {
    let color = if r.is_error {
        theme::DANGER()
    } else {
        theme::LABEL_3()
    };
    let text = match (&r.output_detail, &r.result) {
        (None, None) => {
            return div().child(empty_text(t!("trajectory.no_result")));
        }
        (Some(d), _) => d,
        (None, Some(s)) => s,
    };
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(v) if v.is_object() || v.is_array() => {
            div().child(json_tree_block(store, s, r.index, &v))
        }
        _ => div().child(code_block("mono-result", text, color)),
    }
}

/// Raw tab(ASSISTANT:Thinking 折叠 + 输出全文)
fn raw_body(store: &Entity<AppStore>, s: &Snap<'_>, r: &TrajectoryRecord) -> Div {
    let mut col = div().v_flex().gap(px(8.));
    // CONTEXT:注入文本为单 text 块,「Block #1 text」头 + 等宽原文
    if r.kind == "context" {
        return col.child(context_source_block(r));
    }
    // MESSAGE:thinking/text/tool-call 连续编号
    if r.kind == "message" {
        return assistant_source_blocks(store, s, r);
    }
    if let Some(t) = &r.thinking_detail {
        let open = s.raw_thinking;
        let s2 = store.clone();
        let t2 = t.clone();
        col = col.child(
            div()
                .id("inspector-thinking")
                .v_flex()
                .gap(px(4.))
                .border_l_2()
                .border_color(theme::BORDER())
                .pl(px(10.))
                .child(
                    div()
                        .id("inspector-thinking-toggle")
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .cursor_pointer()
                        .text_size(px(11.))
                        .text_color(theme::CAPTION())
                        .hover(|st| st.text_color(theme::LABEL_3()))
                        .child(t!("trajectory.thinking"))
                        .child(fixed(
                            if open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            },
                            12.,
                        ))
                        .on_click(move |_, _, cx| {
                            s2.update(cx, |st, cx| st.toggle_inspector_thinking(cx));
                        }),
                )
                .when(open, move |el| {
                    el.child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme::LABEL_3())
                            .line_height(gpui_kit::relative(1.55))
                            .child(t2.clone()),
                    )
                }),
        );
    }
    match &r.output_detail {
        Some(o) => col = col.child(mono_block("mono-raw", o, theme::LABEL_3())),
        None => col = col.child(empty_text(t!("trajectory.na"))),
    }
    col
}

/// Timing tab(记录)
fn timing_body(r: &TrajectoryRecord) -> Div {
    let total = rec_total_ms(r);
    let mut col = div().v_flex().gap(px(2.));
    col = col.child(dl_row(
        t!("trajectory.row_started"),
        r.started_at.map(fmt_clock).unwrap_or_else(|| "—".into()),
    ));
    col = col.child(dl_row(
        t!("trajectory.row_duration"),
        total.map(fmt_ms).unwrap_or_else(|| "—".into()),
    ));
    if r.kind == "message" {
        col = col.child(dl_row(
            "TTFT",
            r.ttft_ms.map(fmt_ms).unwrap_or_else(|| "—".into()),
        ));
        col = col.child(dl_row(
            t!("trajectory.row_generation"),
            match (r.ttft_ms, total) {
                (Some(t), Some(ms)) if ms > t => fmt_ms(ms - t),
                _ => "—".into(),
            },
        ));
    }
    col.child(dl_row(
        t!("trajectory.tab_timing"),
        timing_source(total.is_some()),
    ))
}

/// Summary tab(请求):Status/Provider/Model/Tool calls/
/// …/Result 跳转行——链到该请求产出的助手消息或压缩记录)
fn request_summary_body(store: &Entity<AppStore>, s: &Snap<'_>, q: &TrajectoryRequest) -> Div {
    let mut col = div().v_flex().gap(px(2.));
    col = col.child(dl_row(
        t!("trajectory.row_status"),
        if q.status == "error" {
            div()
                .text_color(theme::DANGER())
                .child(t!("trajectory.status_failed"))
        } else {
            div().child(t!("trajectory.status_complete"))
        },
    ));
    col = col.child(dl_row(t!("trajectory.row_provider"), q.provider.clone()));
    col = col.child(dl_row(t!("trajectory.row_model"), q.model.clone()));
    col = col.child(dl_row(
        t!("trajectory.row_tool_calls"),
        fmt_tok(q.tool_calls),
    ));
    if let Some(e) = &q.reasoning_effort {
        col = col.child(dl_row(t!("trajectory.row_reasoning"), e.clone()));
    }
    col = col.child(dl_row(
        t!("trajectory.row_started"),
        fmt_clock(q.started_at),
    ));
    // Result:该请求产出的记录(message/compacted),`>` 跳转其 Summary
    if let Some(res) = s.view.records.iter().find(|r| {
        r.request_number == Some(q.number) && matches!(r.kind.as_str(), "message" | "compacted")
    }) {
        let label = if res.kind == "compacted" {
            t!("trajectory.request_result_compacted")
        } else {
            t!("trajectory.request_result_assistant")
        };
        let s2 = store.clone();
        let ix = res.index;
        col = col.child(dl_row(
            t!("trajectory.row_result"),
            nav_link("goto-req-result", label).on_click(move |_, _, cx| {
                s2.update(cx, |st, cx| st.select_trajectory_record(ix, cx));
            }),
        ));
    }
    col
}

/// Usage tab(请求:This request / Session cumulative 两组五桶)
fn usage_body(q: &TrajectoryRequest) -> Div {
    let mut col = div().v_flex().gap(px(2.));
    match &q.usage {
        None => col = col.child(empty_text(t!("trajectory.usage_na"))),
        Some(u) => col = col.child(usage_group(t!("trajectory.usage_this"), u)),
    }
    col.child(section("sec-cumulative", t!("trajectory.usage_cumulative")))
        .child(usage_group("", &q.cumulative))
}

/// 用量组(Input/Cached/Other/Output/Reasoning/Content)
fn usage_group(title: impl Into<gpui_kit::SharedString>, u: &TrajectoryUsage) -> Div {
    let title = title.into();
    div()
        .v_flex()
        .when(!title.is_empty(), |el| {
            el.child(
                div()
                    .mb(px(2.))
                    .text_size(px(11.))
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .text_color(theme::CAPTION())
                    .child(title.to_string()),
            )
        })
        .child(dl_row(
            t!("trajectory.legend_input"),
            format!("{} tok", fmt_tok(u.input)),
        ))
        .child(dl_row(t!("trajectory.row_cached"), fmt_tok(u.cached)))
        .child(dl_row(t!("trajectory.row_kind_other"), fmt_tok(u.other)))
        .child(dl_row(
            t!("trajectory.row_output"),
            format!("{} tok", fmt_tok(u.output)),
        ))
        .child(dl_row(t!("trajectory.row_reasoning"), fmt_tok(u.reasoning)))
        .child(dl_row(t!("trajectory.row_content"), fmt_tok(u.content())))
}

/// Timing tab(请求)
fn request_timing_body(q: &TrajectoryRequest) -> Div {
    div()
        .v_flex()
        .gap(px(2.))
        .child(dl_row(
            t!("trajectory.row_started"),
            fmt_clock(q.started_at),
        ))
        .child(dl_row(
            t!("trajectory.row_completed"),
            if q.completed_at > 0 {
                fmt_clock(q.completed_at)
            } else {
                "—".into()
            },
        ))
        .child(dl_row(t!("trajectory.row_total"), fmt_ms(q.duration_ms)))
        .child(dl_row(
            "TTFT",
            q.ttft_ms.map(fmt_ms).unwrap_or_else(|| "—".into()),
        ))
}

/// 缺失文案
fn empty_text(text: impl Into<gpui_kit::SharedString>) -> Div {
    let text = text.into();
    div()
        .py(px(14.))
        .text_size(px(12.))
        .text_color(theme::LABEL_3())
        .child(text.to_string())
}

// ── 决策记录(decision/* 折叠面)────────────────────

/// 场景名:守卫/哨兵/裁判/咨询四个既定义场景词典化;协议外场景逐字
/// 跟随(不猜也不翻)。
fn decision_scenario_label(scenario: Option<&str>) -> String {
    match scenario {
        Some("guard") => t!("trajectory.scenario_guard").to_string(),
        Some("stop") => t!("trajectory.scenario_stop").to_string(),
        Some("context") => t!("trajectory.scenario_context").to_string(),
        Some("tool") => t!("trajectory.scenario_tool").to_string(),
        Some("fold") => t!("trajectory.scenario_fold").to_string(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

/// 裁决词 + 配色(守卫行尾标记与决策页共用);**无裁决维度时 None**。
///
/// 取 `verdict` 问的选项;无该问则退回 choice 型应答里 id 最小的那条
/// (显式排序,不依赖 JSON map 的迭代序——该序随 serde_json 的
/// preserve_order 特性而变,拿它当选择依据会静默漂移)。`proceed`/`block`
/// 是守卫问题的既定义项(见 liuma-decision 的 guard 场景),其余选项名逐字
/// 跟随——选项名是**问题定义的一部分**,不是界面文案,翻译它等于篡改量纲。
///
/// None = 这份 receipt 本就没有「裁决」这一维:`decide` 工具(scenario=tool)
/// 的答案是模型自拟的问题,常是 noul,拿它当裁决会显示成「未决」——把一次
/// 正常应答说成没结论。调用方见 None 改显答案摘要。
fn decision_verdict(d: &liuma_core::trajectory::DecisionRecord) -> Option<(String, Rgba)> {
    if d.error.is_some() {
        return Some((t!("trajectory.verdict_failed").to_string(), theme::DANGER()));
    }
    let choice = d.answers.as_ref().and_then(|a| {
        let by_id = &a["verdict"]["choice"];
        if by_id.is_string() {
            return by_id.as_str().map(String::from);
        }
        let choices = a.as_object()?;
        let mut ids: Vec<&String> = choices
            .iter()
            .filter(|(_, ans)| ans["type"].as_str() == Some("choice"))
            .map(|(id, _)| id)
            .collect();
        ids.sort();
        let first = choices.get(*ids.first()?)?;
        first["choice"].as_str().map(String::from)
    })?;
    Some(match choice.as_str() {
        "proceed" => (
            t!("trajectory.verdict_proceed").to_string(),
            theme::SUCCESS(),
        ),
        "block" => (t!("trajectory.verdict_block").to_string(), theme::DANGER()),
        other => (other.to_string(), theme::LABEL_2()),
    })
}

/// score 答案的档位名:取**最接近的等级索引**的档名(见
/// `liuma_core::trajectory::DecisionRecord` 的 answers——score 是等级
/// 索引的概率加权期望,可落两级之间,如 1.43)。
///
/// 为什么翻档名而不是照数显示:0–3 的**档位分**混进 0–1 的**概率**里
/// 当数字看是错的量纲(noul/confidence 是概率,score 是档位)。
///
/// 就近取最接近的**键**而非「四舍五入后查表」:后者遇档位索引不连续
/// (如 0/10/20)或分值越界就查空。等距时取**更高**档——风险/情绪量表
/// 上这是偏保守的一侧,宁可把中间态说得重一点。
///
/// legend 键非整数(协议外形状)、全不可解析、或 score 非有限值 → None:
/// 调用方退回显示原数字,不猜也不伪造档名。
fn score_legend_label(answer: &serde_json::Value, score: f64) -> Option<String> {
    // 非有限值先挡掉:NaN 与任何数比较恒假,不挡则「更优」判据对首个候选
    // 恒真(空 best 直接收),结果凭空认领字典序第一档——那是编的档名
    if !score.is_finite() {
        return None;
    }
    let legend = answer["legend"].as_object()?;
    let mut best: Option<(i64, f64, &str)> = None;
    for (key, level) in legend {
        let (Ok(index), Some(level)) = (key.parse::<i64>(), level.as_str()) else {
            continue;
        };
        let distance = (score - index as f64).abs();
        // 显式比较而非依赖迭代序(BTreeMap 按**字典序**出键,"10" 在 "2"
        // 前,同距时靠「后见者胜」会挑中较低的档)
        let better = match best {
            None => true,
            Some((best_index, best_distance, _)) => {
                distance < best_distance || (distance == best_distance && index > best_index)
            }
        };
        if better {
            best = Some((index, distance, level));
        }
    }
    best.map(|(_, _, level)| level.to_string())
}

/// 答案摘要:id=值(按应答书写序)。三类量纲各说各话,不混:
///   noul   = 0..1 概率 → 照数显示
///   choice = 选项 + confidence(0..1)
///   score  = **等级索引**(同 types.rs 口径),不是概率 → 翻成档位名
///            (legend 由应答携带;缺 legend 才退回数字,免得伪造档名)
fn decision_summary(d: &liuma_core::trajectory::DecisionRecord) -> String {
    let Some(answers) = d.answers.as_ref().and_then(|a| a.as_object()) else {
        return d.error.clone().unwrap_or_default();
    };
    answers
        .iter()
        .filter_map(|(id, a)| match a["type"].as_str() {
            Some("noul") => Some(format!("{id}={:.2}", a["noul"].as_f64().unwrap_or(0.))),
            Some("choice") => Some(format!(
                "{id}={}·{:.2}",
                a["choice"].as_str().unwrap_or("?"),
                a["confidence"].as_f64().unwrap_or(0.)
            )),
            Some("score") => {
                let score = a["score"].as_f64().unwrap_or(0.);
                Some(match score_legend_label(a, score) {
                    Some(level) => format!("{id}={level}"),
                    None => format!("{id}={score:.2}"),
                })
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// 工具行尾的裁决标记(方案甲:守卫裁决是**本次调用的一个阶段**,不
/// 另立行——见 `liuma_core::trajectory::TrajectoryRecord::decision`)。
/// `ix` = 记录行号(调试选择器用,与台账行同号)。
fn decision_chip(ix: u64, d: &liuma_core::trajectory::DecisionRecord) -> Div {
    // 无裁决维度(decide 工具一类)→ 显答案摘要,颜色与字重退到次级:
    // 那份 receipt 没有「放行/拦下」这回事,不该借守卫的词说话
    let (text, color, weight) = match decision_verdict(d) {
        Some((verdict, color)) => (verdict, color, gpui_kit::FontWeight::SEMIBOLD),
        None => (
            decision_summary(d),
            theme::CAPTION(),
            gpui_kit::FontWeight::NORMAL,
        ),
    };
    div()
        .debug_selector(move || format!("traj-decision-chip-{ix}"))
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(4.))
        .pl(px(2.))
        .child(
            div()
                .text_size(px(11.))
                .text_color(theme::CAPTION())
                .child(decision_scenario_label(Some(&d.scenario))),
        )
        .child(
            div()
                .text_size(px(11.))
                .font_weight(weight)
                .text_color(color)
                .child(text),
        )
}

/// 折叠行 chip(触发来源 + 条数 + prefix token + 已裁条数)
fn fold_chip(ix: u64, f: &liuma_core::trajectory::FoldRecord) -> Div {
    let mut text = t!(
        "trajectory.fold_chip",
        items = f.items,
        tokens = f.prefix_tokens
    )
    .into_owned();
    if f.pruned_items > 0 {
        text.push_str(&t!("trajectory.fold_chip_pruned", n = f.pruned_items));
    }
    div()
        .debug_selector(move || format!("traj-fold-chip-{ix}"))
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(4.))
        .pl(px(2.))
        .child(
            div()
                .text_size(px(11.))
                .text_color(theme::CAPTION())
                .child(fold_trigger_label(&f.trigger)),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(theme::LABEL_3())
                .child(text),
        )
}

/// 触发来源标签(空串 = 旧日志无此字段)
fn fold_trigger_label(trigger: &str) -> String {
    match trigger {
        "auto" => t!("trajectory.trigger_auto").to_string(),
        "manual" => t!("trajectory.trigger_manual").to_string(),
        "overflow" => t!("trajectory.trigger_overflow").to_string(),
        _ => t!("trajectory.trigger_unknown").to_string(),
    }
}

/// 折叠页正文:触发 / 压力 / 门槛 / 保留尾 / 遮蔽范围 / 条数 / 前缀
/// token / 价值裁定计数 / 本次耗时。全部来自落档事件,没有第二权威。
fn fold_tab_body(r: &TrajectoryRecord) -> Div {
    let mut col = div()
        .v_flex()
        .gap(px(2.))
        .debug_selector(|| "inspector-fold-body".to_string());
    let Some(f) = &r.fold else {
        return col.child(empty_text(t!("trajectory.no_fold")));
    };
    col = col.child(dl_row(
        t!("trajectory.row_trigger"),
        fold_trigger_label(&f.trigger),
    ));
    if f.pressure_tokens > 0 {
        col = col.child(dl_row(
            t!("trajectory.row_pressure"),
            format!("{} tok", fmt_tok(f.pressure_tokens)),
        ));
    }
    if let Some(t) = f.threshold_tokens {
        col = col.child(dl_row(
            t!("trajectory.row_threshold"),
            format!("{} tok", fmt_tok(t)),
        ));
    }
    if f.retain_tokens > 0 {
        col = col.child(dl_row(
            t!("trajectory.row_retain"),
            format!("{} tok", fmt_tok(f.retain_tokens)),
        ));
    }
    if f.shadowed_end > 0 {
        col = col.child(dl_row(
            t!("trajectory.row_shadowed"),
            div().font_family("Menlo").text_size(px(11.)).child(t!(
                "trajectory.seq_range",
                a = f.shadowed_start.min(f.shadowed_end),
                b = f.shadowed_end
            )),
        ));
    }
    col = col.child(dl_row(
        t!("trajectory.row_fold_items"),
        t!("trajectory.fold_items_count", n = f.items),
    ));
    col = col.child(dl_row(
        t!("trajectory.row_prefix_tokens"),
        format!("{} tok", fmt_tok(f.prefix_tokens)),
    ));
    // 价值裁定:候选分母诚实(评估 M / 共 N),裁掉分「已生效」与
    // 「判为无价值」(仅记录档两者不同)
    if f.total_candidates > 0 || f.judged_candidates > 0 {
        let mut line = t!(
            "trajectory.judge_line",
            judged = f.judged_candidates,
            total = f.total_candidates,
            no_value = f.no_value_candidates,
            pruned = f.pruned_items
        )
        .into_owned();
        let unjudged = f.total_candidates.saturating_sub(f.judged_candidates);
        if unjudged > 0 {
            line.push_str(&t!("trajectory.judge_unjudged", n = unjudged));
        }
        col = col.child(dl_row(t!("trajectory.row_judge"), line));
    } else {
        col = col.child(dl_row(
            t!("trajectory.row_judge"),
            t!("trajectory.judge_none"),
        ));
    }
    col = col.child(dl_row(
        t!("trajectory.row_duration"),
        rec_total_ms(r).map(fmt_ms).unwrap_or_else(|| "—".into()),
    ));
    col
}

/// 决策页正文:场景 / 模型 / 裁决 / 应答 / 问题 / 耗时 / 状态摘要;
/// 失败给原因(裁决词已由 decision_verdict 置为「已失败」)。
fn decision_tab_body(r: &TrajectoryRecord) -> Div {
    let mut col = div().v_flex().gap(px(2.));
    let Some(d) = &r.decision else {
        return col.child(empty_text(t!("trajectory.no_decision")));
    };
    col = col.child(dl_row(
        t!("trajectory.row_scenario"),
        decision_scenario_label(Some(&d.scenario)),
    ));
    col = col.child(dl_row(t!("trajectory.row_model"), d.model.clone()));
    if let Some((verdict, color)) = decision_verdict(d) {
        col = col.child(dl_row(
            t!("trajectory.row_verdict"),
            div().text_color(color).child(verdict),
        ));
    }
    let summary = decision_summary(d);
    if !summary.is_empty() {
        col = col.child(dl_row(t!("trajectory.row_answers"), summary));
    }
    if !d.questions.is_empty() {
        col = col.child(dl_row(
            t!("trajectory.row_questions"),
            d.questions.join(", "),
        ));
    }
    col = col.child(dl_row(
        t!("trajectory.row_duration"),
        if d.duration_ms > 0 {
            fmt_ms(d.duration_ms)
        } else {
            "—".into()
        },
    ));
    if let Some(n) = d.pruned {
        col = col.child(dl_row(
            t!("trajectory.row_pruned"),
            t!("trajectory.pruned_count", n = n),
        ));
    }
    if let Some(digest) = &d.state_digest {
        col = col.child(dl_row(
            t!("trajectory.row_state_digest"),
            div()
                .font_family("Menlo")
                .text_size(px(11.))
                .truncate()
                .child(digest.clone()),
        ));
    }
    match &d.error {
        Some(reason) => col.child(
            div().pt(px(6.)).child(
                div()
                    .text_size(px(12.))
                    .text_color(theme::DANGER())
                    .child(reason.clone()),
            ),
        ),
        None => col,
    }
}

/// 记录总时长 ms
fn rec_total_ms(r: &TrajectoryRecord) -> Option<i64> {
    r.time_seconds.map(|s| (s * 1000.) as i64)
}

/// Source 标签(kind 特例 + 首字母大写兜底,en)
fn message_source_label(source: &serde_json::Value) -> String {
    let kind = source["kind"].as_str().unwrap_or_default();
    match kind {
        "user" => t!("trajectory.source_user").into_owned(),
        "plugin" => match source["plugin"].as_str() {
            Some(p) if !p.is_empty() => t!("trajectory.source_plugin_named", name = p).into_owned(),
            _ => t!("trajectory.source_plugin").into_owned(),
        },
        "goal" => match source["round"].as_u64() {
            Some(round) if round > 0 => {
                t!("trajectory.source_goal_round", round = round).into_owned()
            }
            _ => t!("trajectory.source_goal").into_owned(),
        },
        "" => t!("trajectory.source_unknown").into_owned(),
        // 兜底 = 宿主 kind 原文首字母大写(线上数据逐字,不进文案文件)
        other => format!("{}{}", other[..1].to_uppercase(), &other[1..]),
    }
}

// ── ASSISTANT(message)详情(Summary / Preview / Raw)──

/// 工具调用行(单行形态:扳手 + name + 空格 + args 同行截断——
/// args 取 text 的紧凑段,payload 是 pretty 多行 JSON 不可用;
/// 12px Menlo,名称 LABEL_2 / 参数 LABEL_3。点击跳工具记录)
fn assistant_tool_call_row(store: &Entity<AppStore>, call: &TrajectoryRecord) -> impl IntoElement {
    let (name, args) = match call.text.split_once(' ') {
        Some((n, a)) => (n, a),
        None => (call.text.as_str(), ""),
    };
    let s2 = store.clone();
    let ix = call.index;
    div()
        .id(("assistant-call", ix))
        .flex()
        .items_center()
        .gap(px(5.))
        .min_w(px(0.))
        .py(px(1.))
        .cursor_pointer()
        .hover(|st| st.opacity(0.8))
        .child(fixed(LiumaIcon::Wrench, 12.))
        .child(
            div()
                .flex_shrink_0()
                .font_family("Menlo")
                .text_size(px(12.))
                .text_color(theme::LABEL_2())
                .child(name.to_string()),
        )
        .child(
            div()
                .min_w(px(0.))
                .font_family("Menlo")
                .text_size(px(12.))
                .text_color(theme::LABEL_3())
                .truncate()
                .child(args.to_string()),
        )
        .on_click(move |_, _, cx| {
            s2.update(cx, |st, cx| st.select_trajectory_record(ix, cx));
        })
        .debug_selector(move || format!("assistant-call-{ix}"))
}

/// Preview 页(Summary 小节同款):Thinking 折叠(默认收)+ 正文
/// markdown + 工具调用行
fn assistant_preview_body(store: &Entity<AppStore>, s: &Snap<'_>, r: &TrajectoryRecord) -> Div {
    let mut col = div()
        .debug_selector(move || format!("traj-preview-{}", r.index))
        .v_flex();
    if let Some(t) = &r.thinking_detail {
        let open = s.raw_thinking;
        let s2 = store.clone();
        let t2 = t.clone();
        col = col.child(
            div()
                .id("assistant-preview-thinking")
                .v_flex()
                .border_l_2()
                .border_color(theme::BORDER())
                .pl(px(10.))
                .child(
                    div()
                        .id("assistant-preview-thinking-toggle")
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .cursor_pointer()
                        .text_size(px(12.))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(theme::LABEL_2())
                        .child(t!("trajectory.thinking"))
                        .child(fixed(
                            if open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            },
                            12.,
                        ))
                        .on_click(move |_, _, cx| {
                            s2.update(cx, |st, cx| st.toggle_inspector_thinking(cx));
                        }),
                )
                .when(open, move |el| {
                    el.child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme::LABEL_2())
                            .line_height(gpui_kit::relative(1.5))
                            .child(t2.clone()),
                    )
                }),
        );
    }
    if let Some(o) = &r.output_detail {
        col = col.child(div().mt(px(6.)).child(crate::kits::markdown_tv::tv_static(
            gpui_kit::SharedString::from(format!("traj-preview-{}", r.index)),
            o,
        )));
    }
    if r.output_detail.is_none() && r.thinking_detail.is_none() {
        col = col.child(empty_text(t!("trajectory.no_content")));
    }
    for c in s
        .index
        .map(|i| i.step_tool_calls(&s.view.records, r).collect::<Vec<_>>())
        .unwrap_or_default()
    {
        col = col.child(assistant_tool_call_row(store, c));
    }
    col
}

/// Raw 块头(「Block #N type」;tool-call 带 › 跳工具记录)
fn source_block_header(
    store: &Entity<AppStore>,
    n: usize,
    kind: &'static str,
    jump: Option<u64>,
) -> gpui_kit::AnyElement {
    let label = format!("Block #{n} {kind}");
    match jump {
        Some(ix) => {
            let s2 = store.clone();
            div()
                .id(("block-jump", ix))
                .flex()
                .items_center()
                .gap(px(2.))
                .cursor_pointer()
                .hover(|st| st.opacity(0.8))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme::CAPTION())
                        .child(label),
                )
                .child(fixed(IconName::ChevronRight, 12.))
                .on_click(move |_, _, cx| {
                    s2.update(cx, |st, cx| st.select_trajectory_record(ix, cx));
                })
                .debug_selector(move || format!("block-jump-{ix}"))
                .into_any_element()
        }
        None => div()
            .text_size(px(11.))
            .text_color(theme::CAPTION())
            .child(label)
            .into_any_element(),
    }
}

/// Raw 页块形态:thinking / text / tool-call 按模型
/// 输出序连续编号;块序 = reasoning → 正文 → 同步工具调用
fn assistant_source_blocks(store: &Entity<AppStore>, s: &Snap<'_>, r: &TrajectoryRecord) -> Div {
    let mut col = div().v_flex().gap(px(6.));
    let mut n = 0usize;
    if let Some(t) = &r.thinking_detail {
        n += 1;
        col = col
            .child(source_block_header(store, n, "thinking", None))
            .child(mono_block(("mono-block", n), t, theme::LABEL()));
    }
    if let Some(o) = &r.output_detail {
        n += 1;
        col = col
            .child(source_block_header(store, n, "text", None))
            .child(mono_block(("mono-block", n), o, theme::LABEL()));
    }
    if n == 0 {
        n += 1;
        col = col
            .child(source_block_header(store, n, "text", None))
            .child(mono_block(("mono-block", n), &r.text, theme::LABEL()));
    }
    for c in s
        .index
        .map(|i| i.step_tool_calls(&s.view.records, r).collect::<Vec<_>>())
        .unwrap_or_default()
    {
        n += 1;
        col = col
            .child(source_block_header(store, n, "tool-call", Some(c.index)))
            .child(mono_block(
                ("mono-block", n),
                c.payload.as_deref().unwrap_or_default(),
                theme::LABEL(),
            ));
    }
    col
}

/// Summary 的注入文本预览 + Preview tab(markdown 渲染;
/// Summary 内为 preview 紧凑形)
fn context_markdown_body(r: &TrajectoryRecord, key: &str) -> Div {
    let mut col = div().v_flex();
    match r.payload.as_deref() {
        Some(text) if !text.is_empty() => {
            col = col.child(crate::kits::markdown_tv::tv_static(key.to_string(), text));
        }
        _ => col = col.child(empty_text(t!("trajectory.no_content"))),
    }
    col
}

/// Preview tab(完整渲染)
fn preview_tab_body(r: &TrajectoryRecord) -> Div {
    let key = format!("traj-preview-{}", r.index);
    let mut col = div().debug_selector(move || key.clone()).v_flex();
    match r.payload.as_deref() {
        Some(text) if !text.is_empty() => {
            col = col.child(crate::kits::markdown_tv::tv_static(
                gpui_kit::SharedString::from(format!("traj-preview-{}", r.index)),
                text,
            ));
        }
        _ => col = col.child(empty_text(t!("trajectory.no_content"))),
    }
    col
}

/// Raw tab 的块形态(text 块 = 「Block #1 text」头 + 原文)
fn context_source_block(r: &TrajectoryRecord) -> Div {
    let mut col = div().v_flex();
    let Some(text) = r.payload.as_deref() else {
        return col.child(empty_text(t!("trajectory.no_content")));
    };
    col = col
        .child(
            div()
                .text_size(px(11.))
                .text_color(theme::CAPTION())
                .pb(px(4.))
                .child("Block #1 text"),
        )
        .child(mono_block("mono-context-raw", text, theme::LABEL()));
    col
}

/// Source tab(染色对象数据:以「Message JSON」标签 + 高亮块呈现)
fn source_tab_body(r: &TrajectoryRecord) -> Div {
    let mut col = div().v_flex();
    let Some(source) = &r.source else {
        return col.child(empty_text(t!("trajectory.source_not_recorded")));
    };
    col = col
        .child(
            div()
                .text_size(px(11.))
                .text_color(theme::CAPTION())
                .pb(px(4.))
                .child(t!("trajectory.message_json")),
        )
        .child(
            div().child(
                json_block(
                    "mono-context-source",
                    &serde_json::to_string_pretty(source).unwrap_or_default(),
                )
                .into_any_element(),
            ),
        );
    col
}

// ── SYSTEM 详情(System Prompt / Tools / Diff)与 TOOL Schema ────

/// System Prompt 页:Markdown 渲染(空则缺省文案)
fn system_body(r: &TrajectoryRecord) -> Div {
    let mut col = div().v_flex();
    match &r.system_prompt {
        Some(p) if !p.is_empty() => {
            let key = format!("traj-sys-{}", r.index);
            col = col.child(crate::kits::markdown_tv::tv_static(key.clone(), p));
        }
        _ => col = col.child(empty_text(t!("trajectory.no_system_prompt"))),
    }
    col
}

/// 工具 spec 取字段(OpenAI function 包裹优先,扁平兜底;非 null 即在场)
fn spec_field<'a>(t: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    let v = &t["function"][key];
    if !v.is_null() {
        return Some(v);
    }
    let v = &t[key];
    (!v.is_null()).then_some(v)
}

/// spec 名(目录卡标识 / schema 头)
fn spec_name(t: &serde_json::Value) -> String {
    spec_field(t, "name")
        .and_then(|v| v.as_str())
        .unwrap_or("(unnamed)")
        .to_string()
}

/// Tools 页:工具目录(扁平行 + 底部分隔线;折叠行 =
/// chevron + 图标 + mono 名称 + 内联灰描述单行截断;展开 = 完整描述 +
/// 参数 JSON)
fn tools_body(store: &Entity<AppStore>, s: &Snap<'_>, r: &TrajectoryRecord) -> Div {
    let mut col = div().v_flex();
    let Some(catalog) = &r.tools_catalog else {
        return col.child(empty_text(t!("trajectory.no_tools")));
    };
    if catalog.is_empty() {
        return col.child(empty_text(t!("trajectory.no_tools")));
    }
    for (ti, t) in catalog.iter().enumerate() {
        let name = spec_name(t);
        let description = spec_field(t, "description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let parameters = spec_field(t, "parameters").cloned();
        let open = s.expanded_tools.contains(&name);
        let s2 = store.clone();
        let toggle_name = name.clone();
        // 折叠行:12px chevron 列 + 12px 图标列 + 名称 + 弹性宽描述
        // (单行截断);min-h 30,padding 4/12
        let header = div()
            .flex()
            .items_center()
            .min_h(px(30.))
            .px(px(12.))
            .py(px(4.))
            .gap(px(5.))
            .hover(|st| st.bg(theme::DOCK()))
            .child(fixed(
                if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                },
                12.,
            ))
            .child(crate::kits::icons::tool_icon(&name))
            .child(
                div()
                    .font_family("Menlo")
                    .text_size(px(12.))
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .text_color(theme::LABEL())
                    .child(name.clone()),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .text_size(px(12.))
                    .text_color(theme::LABEL_3())
                    .truncate()
                    .child(description.clone()),
            );
        // 条目:行 + 展开体,底部分隔线;
        // 点击只绑折叠行——展开体内点击不收起
        let mut item = div()
            .v_flex()
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme::BORDER())
            .child(
                div()
                    .id(("inspector-tool", ti))
                    .cursor_pointer()
                    .child(header)
                    .on_click(move |_, _, cx| {
                        s2.update(cx, |st, cx| st.toggle_inspector_tool(&toggle_name, cx));
                    }),
            );
        // 展开体(左缩进 29px,对齐名称列)
        if open {
            if !description.is_empty() {
                item = item.child(
                    div()
                        .pl(px(29.))
                        .pt(px(6.))
                        .pb(px(4.))
                        .pr(px(14.))
                        .text_size(px(12.))
                        .text_color(theme::LABEL_2())
                        .line_height(gpui_kit::relative(1.5))
                        .child(description.clone()),
                );
            }
            if let Some(p) = parameters {
                item = item.child(
                    div()
                        .v_flex()
                        .pl(px(29.))
                        .pb(px(8.))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(theme::CAPTION())
                                .mb(px(4.))
                                .child(format!("{name} parameters JSON")),
                        )
                        .child(
                            div().pr(px(6.)).child(
                                json_block(
                                    "mono-tool-params",
                                    &serde_json::to_string_pretty(&p).unwrap_or_default(),
                                )
                                .into_any_element(),
                            ),
                        ),
                );
            }
        }
        col = col.child(item);
    }
    col
}

/// Diff 行操作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffOp {
    Same,
    Add,
    Del,
}

/// 行级 diff(LCS;系统提示/工具目录均为百行内,DP 足够)
fn line_diff(a: &[&str], b: &[&str]) -> Vec<(DiffOp, String)> {
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((DiffOp::Same, a[i].to_string()));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push((DiffOp::Del, a[i].to_string()));
            i += 1;
        } else {
            out.push((DiffOp::Add, b[j].to_string()));
            j += 1;
        }
    }
    while i < n {
        out.push((DiffOp::Del, a[i].to_string()));
        i += 1;
    }
    while j < m {
        out.push((DiffOp::Add, b[j].to_string()));
        j += 1;
    }
    out
}

/// Diff 页:对照前一 SYSTEM 快照,分 System Prompt / Tools 两节
/// (行级 LCS diff)
fn diff_body(s: &Snap<'_>, r: &TrajectoryRecord) -> Div {
    let prev = s
        .index
        .and_then(|i| i.previous_system_snapshot(&s.view.records, r));
    let mut col = div().v_flex().gap(px(8.));
    let Some(prev) = prev else {
        return col.child(empty_text(t!("trajectory.na")));
    };
    let mut has_diff = false;
    // System Prompt 节
    if let (Some(a), Some(b)) = (prev.system_prompt.as_deref(), r.system_prompt.as_deref())
        && a != b
    {
        has_diff = true;
        col = col
            .child(section(
                "sec-diff-system",
                t!("trajectory.tab_system_prompt"),
            ))
            .child(diff_block("diff-system", a, b));
    }
    // Tools 节(目录 pretty 序列化后行 diff)
    if let (Some(a), Some(b)) = (&prev.tools_catalog, &r.tools_catalog)
        && a != b
    {
        has_diff = true;
        let pa = serde_json::to_string_pretty(a).unwrap_or_default();
        let pb = serde_json::to_string_pretty(b).unwrap_or_default();
        col = col
            .child(section("sec-diff-tools", t!("trajectory.tab_tools")))
            .child(diff_block("diff-tools", &pa, &pb));
    }
    if !has_diff {
        col = col.child(empty_text(t!("trajectory.na")));
    }
    col
}

/// diff 渲染块(加绿/删红/同灰;等宽 11px)
fn diff_block(id: &'static str, a: &str, b: &str) -> impl IntoElement {
    let la: Vec<&str> = a.lines().collect();
    let lb: Vec<&str> = b.lines().collect();
    div()
        .id(id)
        .max_h(px(360.))
        .overflow_y_scroll()
        .rounded(px(8.))
        .bg(theme::CODE())
        .py(px(6.))
        .v_flex()
        .font_family("Menlo")
        .text_size(px(11.))
        .line_height(gpui_kit::relative(1.5))
        .children(line_diff(&la, &lb).into_iter().map(|(op, line)| {
            let (prefix, color, bg) = match op {
                DiffOp::Add => (
                    "+ ",
                    theme::SUCCESS(),
                    gpui_kit::Rgba {
                        a: 0.10,
                        ..theme::SUCCESS()
                    },
                ),
                DiffOp::Del => (
                    "- ",
                    theme::DANGER(),
                    gpui_kit::Rgba {
                        a: 0.10,
                        ..theme::DANGER()
                    },
                ),
                DiffOp::Same => ("  ", theme::LABEL_3(), theme::TRANSPARENT()),
            };
            div()
                .flex()
                .px(px(8.))
                .bg(bg)
                .text_color(color)
                .child(format!("{prefix}{line}"))
        }))
}

/// Schema 页(TOOL):name + description + Parameters(高亮)
fn schema_body(store: &Entity<AppStore>, s: &Snap<'_>, r: &TrajectoryRecord) -> Div {
    let Some(raw) = &r.schema_detail else {
        return div().v_flex().child(empty_text(t!("trajectory.schema_na")));
    };
    let Ok(spec) = serde_json::from_str::<serde_json::Value>(raw) else {
        return div()
            .v_flex()
            .child(mono_block("mono-schema", raw, theme::LABEL_3()));
    };
    let name = spec_name(&spec);
    let description = spec_field(&spec, "description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let parameters = spec_field(&spec, "parameters").cloned();
    let mut col = div().v_flex().gap(px(6.));
    if !name.is_empty() {
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(crate::kits::icons::tool_icon(&name))
                .child(
                    div()
                        .font_family("Menlo")
                        .text_size(px(13.))
                        .text_color(theme::LABEL_2())
                        .child(name),
                ),
        );
    }
    if !description.is_empty() {
        col = col.child(
            div()
                .text_size(px(12.))
                .text_color(theme::LABEL_3())
                .line_height(gpui_kit::relative(1.5))
                .child(description),
        );
    }
    match parameters {
        Some(p) if p.is_object() || p.is_array() => {
            col = col
                .child(section(
                    "sec-schema-params",
                    t!("trajectory.sec_parameters"),
                ))
                .child(json_tree_block(store, s, r.index, &p))
        }
        Some(p) => {
            let pretty = serde_json::to_string_pretty(&p).unwrap_or_default();
            col = col
                .child(section(
                    "sec-schema-params",
                    t!("trajectory.sec_parameters"),
                ))
                .child(code_block("mono-schema-params", &pretty, theme::LABEL_3()))
        }
        None => col = col.child(empty_text(t!("trajectory.schema_na"))),
    }
    col
}

#[cfg(test)]
mod tests {
    use super::*;
    use liuma_core::trajectory::DecisionRecord;

    /// 决策记录构造(仅填断言用到的字段)
    fn decision(scenario: &str, answers: Option<serde_json::Value>) -> DecisionRecord {
        DecisionRecord {
            id: "d".into(),
            scenario: scenario.into(),
            model: "jev-latest".into(),
            questions: vec!["q".into()],
            state_digest: None,
            answers,
            error: None,
            duration_ms: 10,
            pruned: None,
        }
    }

    /// 回归锁:裁决词**只在该 receipt 真有「裁决」这一维时**出现。
    ///
    /// decide 工具(scenario=tool)的答案是模型自拟的问题,常是 noul;
    /// 拿它当裁决会显示成「未决」——把一次正常应答说成没结论。
    #[test]
    fn verdict_only_for_receipts_that_have_one() {
        // 守卫:proceed/block 是既定义项
        let proceed = decision(
            "guard",
            Some(serde_json::json!({
                "verdict": { "type": "choice", "choice": "proceed", "confidence": 0.94 },
            })),
        );
        assert_eq!(
            decision_verdict(&proceed).map(|(v, _)| v).as_deref(),
            Some("放行")
        );
        let block = decision(
            "guard",
            Some(serde_json::json!({
                "verdict": { "type": "choice", "choice": "block", "confidence": 0.97 },
            })),
        );
        assert_eq!(
            decision_verdict(&block).map(|(v, _)| v).as_deref(),
            Some("拦下")
        );

        // 无裁决维度:noul 应答 → None(调用方改显答案摘要)
        let advisory = decision(
            "tool",
            Some(serde_json::json!({
                "is_transient": { "type": "noul", "noul": 0.93 },
            })),
        );
        assert!(decision_verdict(&advisory).is_none(), "noul 应答没有裁决词");
        assert_eq!(decision_summary(&advisory), "is_transient=0.93");

        // 无 verdict 问但有 choice 应答:退回该选项(逐字,不翻)
        let other = decision(
            "tool",
            Some(serde_json::json!({
                "which": { "type": "choice", "choice": "retry", "confidence": 0.8 },
            })),
        );
        assert_eq!(
            decision_verdict(&other).map(|(v, _)| v).as_deref(),
            Some("retry")
        );

        // 未收口/失败
        assert!(decision_verdict(&decision("guard", None)).is_none());
        let mut failed = decision("guard", None);
        failed.error = Some("decision timeout".into());
        assert_eq!(
            decision_verdict(&failed).map(|(v, _)| v).as_deref(),
            Some("已失败")
        );
    }

    /// 回归锁:score 答案翻档位名(0–3 的档位分不得冒充 0–1 的概率)
    #[test]
    fn score_answer_renders_as_level_name_not_a_probability_number() {
        let risk = serde_json::json!({
            "type": "score", "score": 2.5,
            "legend": { "0": "Harmless", "1": "Low risk", "2": "Risky", "3": "Severe" },
        });
        // 落在两级之间:等距取更高档(风险量表上偏保守)
        assert_eq!(score_legend_label(&risk, 2.5).as_deref(), Some("Severe"));
        assert_eq!(score_legend_label(&risk, 2.4).as_deref(), Some("Risky"));
        // 加权期望的小数(如 1.43)就近落档
        assert_eq!(score_legend_label(&risk, 1.43).as_deref(), Some("Low risk"));
        // 越界分值夹到端点档,不虚报也不退回数字
        assert_eq!(score_legend_label(&risk, 9.0).as_deref(), Some("Severe"));

        // 档位索引不连续(协议允许任意索引):就近取键,不查空
        let sparse =
            serde_json::json!({ "legend": { "0": "Calm", "10": "Frustrated", "20": "Angry" } });
        assert_eq!(
            score_legend_label(&sparse, 12.0).as_deref(),
            Some("Frustrated")
        );
        assert_eq!(score_legend_label(&sparse, 19.0).as_deref(), Some("Angry"));

        // 无从下判:显式 None,由调用方退回原数字(不伪造档名)
        assert_eq!(
            score_legend_label(&serde_json::json!({ "legend": {} }), 1.0),
            None
        );
        assert_eq!(score_legend_label(&serde_json::json!({}), 1.0), None);
        assert_eq!(
            score_legend_label(&serde_json::json!({ "legend": { "a": "甲" } }), 1.0),
            None,
            "键非整数 = 协议外形状,不猜"
        );
        assert_eq!(score_legend_label(&risk, f64::NAN), None, "NaN 不 panic");

        // 摘要行:三类量纲各说各话
        let d = decision(
            "guard",
            Some(serde_json::json!({
                "verdict": { "type": "choice", "choice": "proceed", "confidence": 0.94 },
                "risk": { "type": "score", "score": 2.5,
                          "legend": { "0": "Harmless", "1": "Low risk", "2": "Risky", "3": "Severe" } },
            })),
        );
        // 摘要按应答书写序(展示用,不承载语义)
        assert_eq!(decision_summary(&d), "verdict=proceed·0.94 · risk=Severe");
    }

    fn rec(index: u64, kind: &str, turn: Option<u64>, group: &str) -> TrajectoryRecord {
        TrajectoryRecord {
            index,
            seq: index,
            kind: kind.into(),
            turn,
            group: group.into(),
            turn_start: false,
            text: "x".into(),
            result: None,
            is_error: false,
            time_seconds: None,
            started_at: Some(1000 + index as i64 * 100),
            request_number: None,
            input: None,
            output: None,
            think: None,
            ttft_ms: None,
            payload: None,
            output_detail: None,
            thinking_detail: None,
            system_prompt: None,
            tools_catalog: None,
            schema_detail: None,
            source: None,
            decision: None,
            fold: None,
        }
    }

    fn collapse() -> CollapseState {
        CollapseState {
            all_turns: false,
            turns: HashSet::new(),
            all_calls: false,
            calls: HashSet::new(),
        }
    }

    /// 搜索 AND 分词 + 字段命中
    #[test]
    fn search_tokens_and_fields() {
        let mut r = rec(1, "tool", Some(1), "Step 1");
        r.text = "bash ls -la".into();
        r.result = Some("file-a".into());
        assert!(search_hit(&r, "bash"));
        assert!(search_hit(&r, "BASH file"));
        assert!(search_hit(&r, "turn 1"));
        assert!(!search_hit(&r, "bash missing"));
        // 空查询 = 全过
        assert!(search_hit(&r, "  "));
    }

    /// turn 折叠:保留首条 + 摘要行;步数/工具数正确
    #[test]
    fn turn_collapse_keeps_first_and_summarizes() {
        let records = vec![
            rec(1, "user", Some(1), "Message"),
            rec(2, "message", Some(1), "Step 1"),
            rec(3, "tool", Some(1), "Step 1"),
            rec(4, "tool", Some(1), "Step 1"),
            rec(5, "user", Some(2), "Message"),
        ];
        let mut c = collapse();
        c.turns.insert(1);
        let rows = build_rows(&records, &[true; 5], &c);
        let kinds: Vec<String> = rows
            .iter()
            .map(|r| match r {
                LedgerRow::Record { rec_ix, .. } => records[*rec_ix].index.to_string(),
                LedgerRow::TurnSummary { turn, .. } => format!("sum:{turn}"),
                LedgerRow::CallSummary { .. } => "call".into(),
                LedgerRow::LoadEarlier => "load".into(),
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["1", "sum:1", "5"],
            "turn1 首条+摘要,turn2 不折叠"
        );
        if let LedgerRow::TurnSummary { steps, tools, .. } = &rows[1] {
            assert_eq!((*steps, *tools), (3, 2));
        } else {
            panic!("第二行应为 TurnSummary");
        }
    }

    /// calls 折叠:assistant 步内连续工具行并入摘要(同组界止)
    #[test]
    fn call_collapse_groups_tool_run() {
        let records = vec![
            rec(1, "user", Some(1), "Message"),
            rec(2, "message", Some(1), "Step 1"),
            rec(3, "tool", Some(1), "Step 1"),
            rec(4, "tool", Some(1), "Step 1"),
            rec(5, "message", Some(1), "Step 2"),
        ];
        let mut c = collapse();
        c.calls.insert(2);
        let rows = build_rows(&records, &[true; 5], &c);
        let shape: Vec<&str> = rows
            .iter()
            .filter_map(|r| match r {
                LedgerRow::Record { rec_ix, .. } => Some(match records[*rec_ix].kind.as_str() {
                    "tool" => "tool",
                    _ => "rec",
                }),
                LedgerRow::CallSummary { .. } => Some("call"),
                LedgerRow::TurnSummary { .. } => None,
                LedgerRow::LoadEarlier => None,
            })
            .collect();
        assert_eq!(shape, vec!["rec", "rec", "call", "rec"]);
    }

    /// 时间线等宽投影 + TTFT 分界
    #[test]
    fn spans_sequence_and_ttft() {
        let mut records = vec![
            rec(1, "system", None, "Message"),
            rec(2, "user", Some(1), "Message"),
        ];
        let spans = build_spans(&records, false);
        assert_eq!(spans.len(), 2);
        assert!((spans[0].x1 - spans[1].x0).abs() < 1e-9);
        let mut m = rec(3, "message", Some(1), "Step 1");
        m.ttft_ms = Some(300);
        m.time_seconds = Some(1.5); // 1500ms
        records.push(m);
        let spans = build_spans(&records, false);
        let split = spans[2].ttft_split.expect("分界存在");
        assert!((split - 0.2).abs() < 1e-9, "300/1500");
        assert!(spans[0].ttft_split.is_none());
    }

    /// duration 模式:按耗时累计,零总时退化等宽
    #[test]
    fn spans_duration_mode() {
        let mut a = rec(1, "tool", Some(1), "Step 1");
        a.time_seconds = Some(1.0);
        let mut b = rec(2, "tool", Some(1), "Step 1");
        b.time_seconds = Some(3.0);
        let spans = build_spans(&[a, b], true);
        assert!((spans[0].x0 - 0.).abs() < 1e-9);
        assert!((spans[0].x1 - 0.25).abs() < 1e-9, "1s/4s = 0.25");
        assert!((spans[1].x1 - 1.).abs() < 1e-9);
        // 全零:退化
        let z = vec![
            rec(1, "tool", Some(1), "Step 1"),
            rec(2, "tool", Some(1), "Step 1"),
        ];
        let spans = build_spans(&z, true);
        assert!((spans[0].x1 - 0.5).abs() < 1e-9);
    }

    /// 千分位
    #[test]
    fn thousands_groups() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1542), "1,542");
        assert_eq!(thousands(1200000), "1,200,000");
        assert_eq!(thousands(-1542), "-1,542");
    }

    /// 行 diff:公共行保持、增删配对
    #[test]
    fn line_diff_ops() {
        let a = vec!["l1", "l2", "l3"];
        let b = vec!["l1", "x", "l3"];
        let d = line_diff(&a, &b);
        assert_eq!(
            d,
            vec![
                (DiffOp::Same, "l1".into()),
                (DiffOp::Del, "l2".into()),
                (DiffOp::Add, "x".into()),
                (DiffOp::Same, "l3".into()),
            ]
        );
        // 纯追加
        let d = line_diff(&["a"], &["a", "b"]);
        assert_eq!(
            d,
            vec![(DiffOp::Same, "a".into()), (DiffOp::Add, "b".into())]
        );
        // 空侧
        assert_eq!(line_diff(&[], &["z"]), vec![(DiffOp::Add, "z".into())]);
    }

    /// spec 字段提取:OpenAI function 包裹优先,扁平兜底,对象值不漏
    #[test]
    fn spec_field_shapes() {
        let wrapped = serde_json::json!({
            "type": "function",
            "function": { "name": "bash", "description": "d", "parameters": { "type": "object" } }
        });
        assert_eq!(spec_name(&wrapped), "bash");
        assert_eq!(
            spec_field(&wrapped, "parameters").and_then(|v| v["type"].as_str()),
            Some("object")
        );
        let flat = serde_json::json!({ "name": "read", "parameters": { "type": "object" } });
        assert_eq!(spec_name(&flat), "read");
        assert!(spec_field(&flat, "description").is_none());
    }

    /// 工具按 turn + Step N 归属请求;消息优先 request_number
    #[test]
    fn owning_request_matches_step() {
        use liuma_core::trajectory::TrajectoryRequest;
        let req = |number: u64, turn: u64, step: u64| TrajectoryRequest {
            number,
            turn,
            step,
            model: "m".into(),
            provider: "p".into(),
            reasoning_effort: None,
            status: "complete".into(),
            started_at: 0,
            completed_at: 0,
            duration_ms: 0,
            ttft_ms: None,
            usage: None,
            cumulative: Default::default(),
            tool_calls: 0,
        };
        let requests = vec![req(1, 1, 1), req(2, 1, 2), req(3, 2, 1)];
        // 工具:Step 2 的工具 → Request #2
        let tool = rec(4, "tool", Some(1), "Step 2");
        assert_eq!(owning_request(&tool, &requests).map(|q| q.number), Some(2));
        // 消息:request_number 优先(即便 group 归属另指)
        let mut m = rec(3, "message", Some(1), "Step 1");
        m.request_number = Some(3);
        assert_eq!(owning_request(&m, &requests).map(|q| q.number), Some(3));
        // 轮外/Message 组 → 无归属
        let user = rec(2, "user", Some(1), "Message");
        assert!(owning_request(&user, &requests).is_none());
    }

    /// 记录派生索引:父消息 / 前一个带快照 SYSTEM / 同步工具(检查器三处
    /// 查找的生产路径;此前是逐次全表扫)
    #[test]
    fn record_index_lookups() {
        let mut sys1 = rec(1, "system", None, "Message");
        sys1.system_prompt = Some("提示词 v1".into());
        let mut sys2 = rec(6, "system", None, "Message");
        sys2.system_prompt = None; // 无快照:不作 Diff 左值
        let records = vec![
            sys1,
            rec(2, "user", Some(1), "Message"),
            rec(3, "message", Some(1), "Step 1"),
            rec(4, "tool", Some(1), "Step 1"),
            rec(5, "tool", Some(1), "Step 1"),
            sys2,
            rec(7, "message", Some(1), "Step 2"),
        ];
        let ix = RecordIndex::build(&records);
        // 工具的发起消息 = 同轮同步中更早的 assistant message
        assert_eq!(
            ix.parent_message(&records, &records[4]).map(|r| r.index),
            Some(3)
        );
        assert_eq!(
            ix.parent_message(&records, &records[6]).map(|r| r.index),
            None
        );
        // 消息自身无父(records[2] = 该 message 本身)
        assert!(ix.parent_message(&records, &records[2]).is_none());
        // 前一个带快照 SYSTEM:sys2 无快照 → 不遮蔽 sys1
        assert_eq!(
            ix.previous_system_snapshot(&records, &records[6])
                .map(|r| r.index),
            Some(1)
        );
        // sys1 自身之前没有 SYSTEM → 无
        assert!(ix.previous_system_snapshot(&records, &records[0]).is_none());
        // 同 (turn, group) 的工具齐出(Step 1 两个;Step 2 无工具)
        let sibs: Vec<u64> = ix
            .step_tool_calls(&records, &records[3])
            .map(|r| r.index)
            .collect();
        assert_eq!(sibs, vec![4, 5]);
        assert_eq!(ix.step_tool_calls(&records, &records[6]).count(), 0);
        // 按 index 直取(检查器选中记录)
        assert_eq!(ix.get(&records, 5).map(|r| r.index), Some(5));
        assert!(ix.get(&records, 99).is_none());
    }

    /// JSON 分词:键/字符串/数字/关键字/标点,转义引号不截断
    #[test]
    fn json_tokens_classify() {
        let toks = json_tokens("  \"command\": \"echo hi\",");
        assert_eq!(
            toks,
            vec![
                (JKind::Plain, "  ".into()),
                (JKind::Key, "\"command\"".into()),
                (JKind::Plain, ": ".into()),
                (JKind::Str, "\"echo hi\"".into()),
                (JKind::Plain, ",".into()),
            ]
        );
        let toks = json_tokens("    \"n\": -42.5, \"ok\": true, \"x\": null");
        assert!(toks.contains(&(JKind::Num, "-42.5".into())));
        assert!(toks.contains(&(JKind::Kw, "true".into())));
        assert!(toks.contains(&(JKind::Kw, "null".into())));
        // 值内转义引号:字符串完整读出,不被截断
        let toks = json_tokens("\"a\": \"x\\\"y\"");
        assert_eq!(toks[0], (JKind::Key, "\"a\"".into()));
        assert_eq!(toks[2], (JKind::Str, "\"x\\\"y\"".into()));
        // 非键字符串(数组元素)
        let toks = json_tokens("  \"ls -la\"");
        assert_eq!(toks[1], (JKind::Str, "\"ls -la\"".into()));
    }
}
