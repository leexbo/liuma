//! attach 路径分阶段计时(env 门控,默认跳过;dev profile 与
//! `just desktop-run` 同条件)。跑法:
//!   LIUMA_MEASURE_LOG=<session.jsonl> \
//!   cargo test -p liuma-app --test attach_stage_timing -- --nocapture --test-threads=1

use std::time::Instant;

/// 模拟 attach 全链:load → 修复扫描 → inbox 重放 → 直播预热 →
/// 轨迹补喂(分批)→ 锚点扫。各阶段独立计时,总 attaching 语义对齐
/// registry 的 attach 装配序
#[test]
fn attach_stage_timing() {
    let Ok(path) = std::env::var("LIUMA_MEASURE_LOG") else {
        eprintln!("[at] 跳过(未设 LIUMA_MEASURE_LOG)");
        return;
    };
    let total = Instant::now();

    // ① load_log(解析 + 打包合并;attach 装配第一步)
    let t = Instant::now();
    let log = liuma_app::load_log(&path).expect("load");
    eprintln!(
        "[at] ① load_log = {:.2}s({} 事件)",
        t.elapsed().as_secs_f64(),
        log.high_water()
    );

    // ② repair_dangling_calls 形态的整表扫描(for_each 借用)
    let t = Instant::now();
    let mut scan_hits = 0u64;
    log.for_each(|ev| {
        if matches!(ev.r#type.as_str(), "tool/call" | "tool/result" | "turn/end") {
            scan_hits += 1;
        }
    });
    eprintln!(
        "[at] ② 修复扫描(for_each)= {:.2}s(命中 {scan_hits})",
        t.elapsed().as_secs_f64()
    );

    // ③ replay_inbox 形态(user/message 收集 + spliced 折叠)
    let t = Instant::now();
    let mut claimed = std::collections::HashSet::new();
    log.for_each(|ev| {
        if ev.r#type == "user/message"
            && let Some(id) = ev.data["id"].as_str()
        {
            claimed.insert(id.to_string());
        }
    });
    eprintln!(
        "[at] ③ inbox 重放(for_each)= {:.2}s({} 条 claimed)",
        t.elapsed().as_secs_f64(),
        claimed.len()
    );

    // ④ 直播预热(translator 计数 + stats + 构成;以三次轻 fold 模拟
    // translator/stats/bd 三份逐事件工作)
    let t = Instant::now();
    let mut n1 = 0u64;
    let mut n2 = 0u64;
    let mut n3 = 0u64;
    log.for_each(|ev| {
        if ev.r#type == "audit/call" {
            n1 += 1;
        }
        if ev.r#type == "turn/start" {
            n2 += 1;
        }
        if ev.r#type == "assistant/message" {
            n3 += 1;
        }
    });
    eprintln!(
        "[at] ④ 直播预热(for_each)= {:.2}s({n1}/{n2}/{n3})",
        t.elapsed().as_secs_f64()
    );

    // ⑤ 轨迹补喂:for_each_from 借用直喂(与 sync_trajectory_from_log 同款)
    let t = Instant::now();
    let mut fed = 0u64;
    log.for_each_from(1, |_| fed += 1);
    eprintln!(
        "[at] ⑤ 轨迹补喂(for_each_from 借用)= {:.2}s({fed} 条)",
        t.elapsed().as_secs_f64()
    );

    // ⑥ 锚点扫(for_each)
    let t = Instant::now();
    let mut anchors = 0u64;
    log.for_each(|ev| {
        if ev.r#type == "user/message"
            && ev.data["source"]["kind"].as_str().unwrap_or("user") == "user"
        {
            anchors += 1;
        }
    });
    eprintln!(
        "[at] ⑥ 锚点扫(for_each)= {:.2}s({anchors} 锚点)",
        t.elapsed().as_secs_f64()
    );

    // ⑦ 对照:单次 owned 全量展开(session_events 类 RPC 的响应面)
    let t = Instant::now();
    let all: Vec<liuma_session::EventEnvelope> = log.iter().collect();
    eprintln!(
        "[at] ⑦ owned 全量展开 = {:.2}s({} 条)",
        t.elapsed().as_secs_f64(),
        all.len()
    );
    drop(all);

    eprintln!("[at] 总计 = {:.2}s", total.elapsed().as_secs_f64());
}
