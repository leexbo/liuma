//! JSONL 主格式后端:追加式事实流。
//!
//! 长会话日志流式落盘、不进组件内存(重写方案 §7);每事件一行、写后即 flush
//! (正确性优先:append-only 日志的崩溃边界 = 行边界)。

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use liuma_session::EventEnvelope;
use liuma_session::chunk_rows::{self, ChunkRow, DeltaKind, ROW_TAG, StorageRecord};

use super::PersistenceError;

/// JSONL 落盘后端(线程安全:单写者经互斥锁串行化)。
/// Clone = 共享同一文件句柄(durable 队列改造:泵任务与驱动任务
/// 各持克隆落 splice/turn 事件,行完整性由写者互斥保证)
#[derive(Clone)]
pub struct JsonlBackend {
    path: PathBuf,
    writer: std::sync::Arc<Mutex<BufWriter<File>>>,
    /// 跨 append 的打包状态:同型 delta 连续段不足 `MIN_RUN` 时先攒着,
    /// 满足即整行落盘。`drop` 与 flush 收尾冲刷
    packer: std::sync::Arc<Mutex<ChunkPacker>>,
}

/// 写侧打包器:只认同型连续 delta;任何不连续/异型/异常形态即冲刷。
/// 与 `chunk_rows::pack` 同一契约,但按单事件增量驱动(append 一次一个)
#[derive(Default)]
struct ChunkPacker {
    kind: Option<DeltaKind>,
    run: Vec<EventEnvelope>,
}

impl ChunkPacker {
    /// 喂入一个事件:返回「现在就该落盘的记录」(0..n 条)
    fn feed(&mut self, ev: EventEnvelope) -> Vec<StorageRecord> {
        let Some(k) = chunk_rows::classify(&ev) else {
            let mut out = self.take_run();
            out.push(StorageRecord::Event(ev));
            return out;
        };
        if self.kind == Some(k)
            && self
                .run
                .last()
                .is_some_and(|p| chunk_rows::continues(p, &ev))
        {
            self.run.push(ev);
            if self.run.len() >= chunk_rows::MIN_RUN * 8 {
                return self.flush_row();
            }
            return Vec::new();
        }
        let out = self.take_run();
        self.kind = Some(k);
        self.run.push(ev);
        out
    }

    /// 收尾:取走当前 run(不足 MIN_RUN 则还原为事件)
    fn take_run(&mut self) -> Vec<StorageRecord> {
        let run = std::mem::take(&mut self.run);
        let kind = self.kind.take();
        match kind {
            Some(k) if run.len() >= chunk_rows::MIN_RUN => vec![build_row(k, run)],
            Some(k) => {
                let _ = k;
                run.into_iter().map(StorageRecord::Event).collect()
            }
            None => {
                debug_assert!(run.is_empty());
                Vec::new()
            }
        }
    }

    /// 满行冲刷:超长 run 也定期落一行(避免无限攒)
    fn flush_row(&mut self) -> Vec<StorageRecord> {
        // 调用方不变式:仅在有 run(含 kind)时调用(AGENTS.md §1 ①)
        #[allow(clippy::expect_used)]
        let kind = self.kind.expect("flush_row 仅在有 run 时调用");
        let run = std::mem::take(&mut self.run);
        vec![build_row(kind, run)]
    }
}

/// 由一段连续同型事件构造打包行
fn build_row(kind: DeltaKind, run: Vec<EventEnvelope>) -> StorageRecord {
    let dt: Vec<i64> = run.windows(2).map(|w| w[1].time - w[0].time).collect();
    let field = kind_field(kind);
    let texts: Vec<String> = run
        .iter()
        .map(|ev| {
            ev.data
                .get(field)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    StorageRecord::Packed(
        ChunkRow {
            seq0: run[0].seq,
            time0: run[0].time,
            dt,
            texts,
        },
        kind,
    )
}

fn kind_field(kind: DeltaKind) -> &'static str {
    match kind {
        DeltaKind::Reasoning => "text",
        DeltaKind::Chunk => "delta",
    }
}

impl JsonlBackend {
    /// 新建(截断既有文件)——新会话的事实流起点
    pub fn create(path: impl Into<PathBuf>) -> Result<Self, PersistenceError> {
        let path = path.into();
        let file = File::create(&path)?;
        Ok(Self {
            path,
            writer: std::sync::Arc::new(Mutex::new(BufWriter::new(file))),
            packer: std::sync::Arc::new(Mutex::new(ChunkPacker::default())),
        })
    }

    /// 打开(或创建)既有会话日志:追加模式,不截断——重开会话的
    /// 正确语义(load_log 重载历史后继续 append)
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, PersistenceError> {
        let path = path.into();
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        Ok(Self {
            path,
            writer: std::sync::Arc::new(Mutex::new(BufWriter::new(file))),
            packer: std::sync::Arc::new(Mutex::new(ChunkPacker::default())),
        })
    }

    /// 日志文件路径
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 追加单事件:经写侧打包器,delta 连续段成行落盘,其余一行一条;
    /// 写后 flush
    pub fn append(&self, ev: &EventEnvelope) -> Result<(), PersistenceError> {
        let records = {
            let mut packer = self
                .packer
                .lock()
                .map_err(|_| PersistenceError::Io(std::io::Error::other("jsonl packer 锁中毒")))?;
            packer.feed(ev.clone())
        };
        self.write_records(&records)
    }

    /// 冲刷打包器尾部(会话关闭/重开前调用;不足 MIN_RUN 的尾段还原为事件行)
    pub fn flush(&self) -> Result<(), PersistenceError> {
        let records = {
            let mut packer = self
                .packer
                .lock()
                .map_err(|_| PersistenceError::Io(std::io::Error::other("jsonl packer 锁中毒")))?;
            packer.take_run()
        };
        self.write_records(&records)
    }

    fn write_records(&self, records: &[StorageRecord]) -> Result<(), PersistenceError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| PersistenceError::Io(std::io::Error::other("jsonl writer 锁中毒")))?;
        for rec in records {
            match rec {
                StorageRecord::Event(ev) => serde_json::to_writer(&mut *writer, ev)?,
                StorageRecord::Packed(row, kind) => {
                    let tagged = TaggedRow {
                        row: ROW_KEY_SENTINEL_VALUE,
                        kind: kind.row_tag(),
                        row_fields: row,
                    };
                    serde_json::to_writer(&mut *writer, &tagged)?;
                }
            }
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    /// 全量读取:逐行解码,读取方守卫生效(未知未标 ignorable → 拒绝整份日志)。
    /// 读前先冲刷打包器——尾部不足 `MIN_RUN` 的段还在 packer 里,不冲则刚
    /// append 的最后几条读不回来(单写者语义下重写安全)
    pub fn load(&self) -> Result<Vec<EventEnvelope>, PersistenceError> {
        self.flush()?;
        load_jsonl(&self.path)
    }
}

/// 顶层行判别键与判别值
const ROW_KEY_SENTINEL_VALUE: &str = ROW_TAG;

/// 打包行的落盘形态:顶层 `{ "row":"chunks", "kind":"reasoning", ...row }`
#[derive(serde::Serialize)]
struct TaggedRow<'a> {
    row: &'static str,
    kind: &'static str,
    #[serde(flatten)]
    row_fields: &'a ChunkRow,
}

/// 从 JSONL 文件全量读取(静态入口,重放/重建路径共用)。
/// 布局盲判别收口在 [`liuma_session::chunk_rows::decode_line_events`]
/// (打包行展开成员;普通行信封直解 + 守卫)——与 load_log /
/// EventStore 同一实现,读写两侧不漂移
pub fn load_jsonl(path: &Path) -> Result<Vec<EventEnvelope>, PersistenceError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let mut line_events = chunk_rows::decode_line_events(&line)
            .map_err(|e| PersistenceError::Turso(format!("{path:?}: 第 {} 行 {e}", idx + 1)))?;
        events.append(&mut line_events);
    }
    Ok(events)
}

#[cfg(test)]
mod chunk_row_tests {
    use super::*;
    use serde_json::json;

    fn reasoning(seq: u64, time: i64, text: &str) -> EventEnvelope {
        let mut ev =
            EventEnvelope::new_ignorable("assistant/reasoning", time, json!({ "text": text }));
        ev.seq = seq;
        ev
    }

    fn user(seq: u64, time: i64, content: &str) -> EventEnvelope {
        let mut ev = EventEnvelope::new("user/message", time, json!({ "content": content }));
        ev.seq = seq;
        ev
    }

    fn tmp_path(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("liuma-jsonl-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir.join("session.jsonl")
    }

    /// 契约:混合事件流 append → 读回逐事件相等;delta 连续段确实成行落盘
    #[test]
    fn append_then_load_round_trips_and_packs() {
        let path = tmp_path("roundtrip");
        let backend = JsonlBackend::create(&path).expect("create");
        let src = vec![
            user(1, 100, "hi"),
            reasoning(2, 200, "The"),
            reasoning(3, 201, " user"),
            reasoning(4, 203, " asks"),
            user(5, 300, "go"),
        ];
        for ev in &src {
            backend.append(ev).expect("append");
        }
        backend.flush().expect("flush");

        // 磁盘形态:5 个逻辑事件只占 3 行(user 行 + 1 打包行 + user 行)
        let text = std::fs::read_to_string(&path).expect("读盘");
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 3, "delta 段应打包成一行: {text:?}");
        let packed_line = serde_json::from_str::<serde_json::Value>(lines[1]).expect("行 JSON");
        assert_eq!(packed_line["row"], "chunks", "行标签在: {packed_line:?}");
        assert_eq!(packed_line["kind"], "reasoning");
        assert_eq!(packed_line["seq0"], 2);
        assert_eq!(
            packed_line["texts"],
            serde_json::json!(["The", " user", " asks"])
        );

        // 读回逐事件相等
        let back = load_jsonl(&path).expect("load");
        assert_eq!(back, src, "逐事件还原");
        let _ = std::fs::remove_dir_all(path.parent().expect("父目录"));
    }

    /// 契约:短 run(< MIN_RUN)不打包,尾段冲刷后还原为事件行
    #[test]
    fn short_tail_unwinds_to_events() {
        let path = tmp_path("tail");
        let backend = JsonlBackend::create(&path).expect("create");
        let src = vec![reasoning(1, 1, "a"), reasoning(2, 2, "b"), user(3, 3, "x")];
        for ev in &src {
            backend.append(ev).expect("append");
        }
        backend.flush().expect("flush");
        let text = std::fs::read_to_string(&path).expect("读盘");
        assert_eq!(text.lines().count(), 3, "尾段两条还原为两行: {text:?}");
        assert_eq!(load_jsonl(&path).expect("load"), src);
        let _ = std::fs::remove_dir_all(path.parent().expect("父目录"));
    }

    /// 契约:跨 run 长度上限(超长段定期落行,不无限攒内存)
    #[test]
    fn long_runs_spill_rows_periodically() {
        let path = tmp_path("spill");
        let backend = JsonlBackend::create(&path).expect("create");
        for i in 0..(chunk_rows::MIN_RUN * 8 * 3) {
            backend
                .append(&reasoning(i as u64 + 1, i as i64, "t"))
                .expect("append");
        }
        backend.flush().expect("flush");
        let text = std::fs::read_to_string(&path).expect("读盘");
        let lines = text.lines().filter(|l| !l.trim().is_empty()).count();
        assert!(lines < 8, "超长段应整行落盘: {lines} 行");
        let back = load_jsonl(&path).expect("load");
        assert_eq!(back.len(), chunk_rows::MIN_RUN * 8 * 3);
        assert_eq!(
            back.last().expect("尾事件").seq,
            chunk_rows::MIN_RUN as u64 * 24
        );
        let _ = std::fs::remove_dir_all(path.parent().expect("父目录"));
    }

    /// 契约:坏行 fail-loud(拒载,不静默丢流)
    #[test]
    fn malformed_packed_row_refuses_load() {
        let path = tmp_path("bad");
        std::fs::write(&path, concat!(
            r#"{"row":"chunks","kind":"reasoning","seq0":1,"time0":0,"dt":[1,2],"texts":["a","b"]}"#,
            "\n",
        ))
        .expect("写坏行");
        let err = load_jsonl(&path).expect_err("必须拒载");
        assert!(err.to_string().contains("dt length"), "{err}");
        let _ = std::fs::remove_dir_all(path.parent().expect("父目录"));
    }

    /// 契约:布局盲——同一读入口装下 packed / unpacked / 混排
    #[test]
    fn reader_is_layout_blind() {
        let path = tmp_path("blind");
        let unpacked = JsonlBackend::create(&path).expect("create");
        let src = vec![
            reasoning(1, 1, "a"),
            reasoning(2, 2, "b"),
            reasoning(3, 3, "c"),
        ];
        for ev in &src {
            unpacked.append(ev).expect("append");
        }
        unpacked.flush().expect("flush");
        // 直接手写打包行(与上面三条逻辑等价)
        let packed_text = r#"{"row":"chunks","kind":"reasoning","seq0":1,"time0":1,"dt":[1,1],"texts":["a","b","c"]}"#;
        std::fs::write(
            &path,
            format!(
                "{}\n{packed_text}\n",
                std::fs::read_to_string(&path).unwrap()
            ),
        )
        .expect("追加");
        let mixed = load_jsonl(&path).expect("load");
        assert_eq!(mixed.len(), 6, "3 条事件行 + 1 行展开成 3 条");
        assert_eq!(&mixed[..3], &src, "前半段逐事件");
        assert_eq!(&mixed[3..], &src, "打包行还原出同样三条");
        let _ = std::fs::remove_dir_all(path.parent().expect("父目录"));
    }
}
