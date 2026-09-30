//! 含打包行的会话文件经两条冷读路径(load_log / EventStore)读回:
//! 布局盲等价锁。回归背景:attach 曾对含打包行的文件直接逐行解信封,
//! 首个打包行(无 `type` 字段)即拒载——新会话一落打包行便无法重开。

use liuma_session::{EventEnvelope, EventLog};
use serde_json::json;

fn reasoning(time: i64, text: &str) -> EventEnvelope {
    EventEnvelope::new_ignorable("assistant/reasoning", time, json!({ "text": text }))
}

#[test]
fn load_log_and_event_store_read_packed_files() {
    let path = std::env::temp_dir().join(format!(
        "liuma-packed-load-{}-{}.jsonl",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let path_str = path.display().to_string();

    // 写侧:JsonlBackend 落盘(delta 连段经 ChunkPacker 成打包行)
    let backend = liuma_host::JsonlBackend::create(&path).expect("建后端");
    let mut log = EventLog::new();
    for (t, text) in [(0, "a"), (1, "b"), (2, "c"), (3, "d")] {
        log.append(reasoning(t, text)).expect("append");
    }
    log.append(EventEnvelope::new(
        "user/message",
        9,
        json!({ "content": "hi" }),
    ))
    .expect("append");
    // 逐条经后端落盘(生产写路径同款:连段在满溢出阈值时成行,
    // flush 收尾不足 MIN_RUN 的段)
    for ev in log.iter() {
        backend.append(&ev).expect("落盘");
    }
    backend.flush().expect("flush");

    // 文件里确有打包行(测试前提;否则本测试退化为普通行回读)
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.lines().any(|l| l.contains("\"row\":\"chunks\"")),
        "落盘文件应含打包行:\n{text}"
    );

    // 读侧一:load_log(attach / 会话重开路径)
    let loaded = liuma_app::load_log(&path_str).expect("load_log 读含打包行文件");
    let want: Vec<EventEnvelope> = log.iter().collect();
    let got: Vec<EventEnvelope> = loaded.iter().collect();
    assert_eq!(got, want, "load_log 逐事件相等");

    // 读侧二:EventStore 端口(with_session_log 冷路径)
    let all = liuma_session::EventStore::all(&liuma_app::JsonlEventStore::new(path_str))
        .expect("EventStore 读含打包行文件");
    assert_eq!(all, want, "EventStore 逐事件相等");

    let _ = std::fs::remove_file(&path);
}
