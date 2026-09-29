//! 真实大会话日志的无损打包差分(env 门控,默认跳过):
//! 逐条 append(打包落盘)→ `load_jsonl` 读回 → 与源日志逐事件相等;
//! 并报告物理行数缩减。跑法:
//!   LIUMA_REAL_LOG=<session.jsonl 路径> \
//!   cargo test -p liuma-host --test real_log_roundtrip -- --nocapture

use liuma_host::{JsonlBackend, persistence::jsonl::load_jsonl};
use liuma_session::EventLog;

#[test]
fn real_log_packed_roundtrip() {
    let Ok(src_path) = std::env::var("LIUMA_REAL_LOG") else {
        eprintln!("[rt] 跳过(未设 LIUMA_REAL_LOG)");
        return;
    };
    let text = std::fs::read_to_string(&src_path).expect("读源日志");
    let mut src = EventLog::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let ev = liuma_session::decode_envelope_str(line).expect("源行解码");
        src.append(ev).expect("append");
    }
    let src_events: Vec<liuma_session::EventEnvelope> = src.iter().cloned().collect();

    let dir = std::env::temp_dir().join(format!("liuma-rt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("临时目录");
    let out = dir.join("packed.jsonl");
    let backend = JsonlBackend::create(&out).expect("create");
    let t = std::time::Instant::now();
    for ev in &src_events {
        backend.append(ev).expect("append");
    }
    backend.flush().expect("flush");
    let write_elapsed = t.elapsed();

    let src_lines = text.lines().filter(|l| !l.trim().is_empty()).count();
    let out_lines =
        std::fs::read_to_string(&out).unwrap().lines().filter(|l| !l.trim().is_empty()).count();
    let src_bytes = text.len();
    let out_bytes = std::fs::metadata(&out).unwrap().len() as usize;

    let t = std::time::Instant::now();
    let back = load_jsonl(&out).expect("读回");
    let read_elapsed = t.elapsed();

    assert_eq!(back.len(), src_events.len(), "事件数相等");
    let mismatches: Vec<usize> = back
        .iter()
        .zip(&src_events)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .take(5)
        .collect();
    assert!(mismatches.is_empty(), "首个不等下标: {mismatches:?}");
    eprintln!(
        "[rt] 源 {src_lines} 行 / {} → 打包 {out_lines} 行 / {}(行数 -{:.1}%,字节 -{:.1}%);写 {write_elapsed:?} 读 {read_elapsed:?};逐事件相等({} 条)",
        format_bytes(src_bytes),
        format_bytes(out_bytes),
        100.0 - out_lines as f64 / src_lines as f64 * 100.0,
        100.0 - out_bytes as f64 / src_bytes as f64 * 100.0,
        back.len()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn format_bytes(b: usize) -> String {
    format!("{:.1} MB", b as f64 / 1048576.0)
}
