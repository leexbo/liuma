//! 批 4 验收测量(env 门控,默认跳过):EventLog 打包双表示的堆常驻,
//! 连同两个对照——逐条信封镜像(批 4 之前的表示)与存储打包行。
//! 跑法:
//!   LIUMA_MEASURE_LOG=<session.jsonl> \
//!   cargo test -p liuma-app --test packed_residency -- --nocapture

use liuma_session::EventLog;
use liuma_session::chunk_rows;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static LIVE: AtomicUsize = AtomicUsize::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            LIVE.fetch_add(l.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            if n >= l.size() {
                LIVE.fetch_add(n - l.size(), Ordering::Relaxed);
            } else {
                LIVE.fetch_sub(l.size() - n, Ordering::Relaxed);
            }
        }
        q
    }
}

#[global_allocator]
static A: Counting = Counting;

#[test]
fn packed_residency() {
    let Ok(path) = std::env::var("LIUMA_MEASURE_LOG") else {
        eprintln!("[pm] 跳过(未设 LIUMA_MEASURE_LOG)");
        return;
    };
    let text = std::fs::read_to_string(&path).expect("读日志");
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    // 基线在日志文本之后:文件缓冲不计入任何一段
    let base = LIVE.load(Ordering::Relaxed);

    // ① EventLog 双表示(打包行驻留)= 批 4 交付面
    let mut log = EventLog::new();
    for l in &lines {
        log.append(liuma_session::decode_envelope_str(l).expect("解码"))
            .expect("append");
    }
    let log_bytes = LIVE.load(Ordering::Relaxed) - base;

    // ② 展开镜像(Vec<EventEnvelope>)= 批 4 之前的逐条表示,对比基线
    let events: Vec<liuma_session::EventEnvelope> = log.iter().collect();
    let mirror_bytes = LIVE.load(Ordering::Relaxed) - base - log_bytes;

    // ③ 存储打包行(JsonlBackend 落盘形态)
    let records = chunk_rows::pack(&events);
    let record_bytes = LIVE.load(Ordering::Relaxed) - base - log_bytes - mirror_bytes;

    let mb = |n: usize| n as f64 / 1048576.0;
    eprintln!("[pm] 事件数 = {}", lines.len());
    eprintln!(
        "[pm] ① EventLog 打包双表示 = {:.1} MB(验收线 ≤60)",
        mb(log_bytes)
    );
    eprintln!(
        "[pm] ② 逐条信封镜像(批 4 前)= {:.1} MB → {:.1}×",
        mb(mirror_bytes),
        mirror_bytes as f64 / log_bytes as f64
    );
    eprintln!(
        "[pm] ③ 存储打包行 = {:.1} MB({} 条记录)",
        mb(record_bytes),
        records.len()
    );
    assert!(
        log_bytes <= 60 * 1048576,
        "EventLog 打包常驻 {:.1} MB 超验收线 60 MB",
        mb(log_bytes)
    );
}
