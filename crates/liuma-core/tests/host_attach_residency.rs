//! 宿主 attach 序列 RSS 实测(env 门控,默认跳过):把真实会话日志复制
//! 进临时布局,按桌面打开会话的调用序逐步采样进程 RSS——定位「打开
//! 大会话即 ~1GB 常驻」发生在宿主哪一步。跑法:
//!   LIUMA_MEASURE_LOG=<session.jsonl> \
//!   cargo test -p liuma-core --release --test host_attach_residency -- --nocapture

#[cfg(target_os = "macos")]
fn rss_mb() -> f64 {
    let Ok(out) = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
    else {
        return 0.;
    };
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u64>()
        .map(|kb| kb as f64 / 1024.0)
        .unwrap_or(0.)
}
#[cfg(not(target_os = "macos"))]
fn rss_mb() -> f64 {
    0.
}

#[tokio::test]
async fn host_attach_rss_curve() {
    let Ok(src) = std::env::var("LIUMA_MEASURE_LOG") else {
        eprintln!("[hr] 跳过(未设 LIUMA_MEASURE_LOG)");
        return;
    };
    let sid = "s-measure".to_string();
    let root = std::env::temp_dir().join(format!("liuma-hr-{}", uuid::Uuid::new_v4().simple()));
    let ws = root.join("ws");
    let sroot = root.join("liuma-home");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&sroot).unwrap();

    eprintln!("[hr] ① 基线(建宿主前)RSS = {:.0} MB", rss_mb());
    let host = std::sync::Arc::new(
        liuma_core::registry::AppHost::new_at(ws.clone(), true, "", sroot.clone()).unwrap(),
    );
    eprintln!("[hr] ② 宿主建成 RSS = {:.0} MB", rss_mb());

    // 会话落点由宿主决定(project_key(工作区));按公开 API 定位后放样本
    let log_path = host.session_log_path(&sid);
    std::fs::create_dir_all(log_path.parent().unwrap()).unwrap();
    std::fs::copy(&src, &log_path).expect("复制样本");

    // 小会话先开(复刻真机顺序:启动自动开首个小会话,再点开大会话)
    let small = "s-small".to_string();
    let small_path = host.session_log_path(&small);
    std::fs::create_dir_all(small_path.parent().unwrap()).unwrap();
    std::fs::write(
        &small_path,
        concat!(
            "{\"type\":\"turn/start\",\"seq\":1,\"time\":0,\"data\":{},\"ignorable\":false}\n",
            "{\"type\":\"user/message\",\"seq\":2,\"time\":0,\"data\":{\"content\":\"hi\"},\"ignorable\":false}\n",
            "{\"type\":\"turn/end\",\"seq\":3,\"time\":0,\"data\":{},\"ignorable\":false}\n"
        ),
    )
    .unwrap();
    let _ = host
        .history(&small, None, 200)
        .await
        .expect("small history");
    eprintln!("[hr] ②b 小会话已开 RSS = {:.0} MB", rss_mb());

    // ③ 纯 attach(轻 RPC 触发;内部 load_log 全档载入 + 驱动装配)
    let _ = host.session_stats(&sid).expect("stats");
    eprintln!("[hr] ③ attach(session_stats 触发)RSS = {:.0} MB", rss_mb());

    // ④ 历史尾窗(窗口翻译 prime 前缀)
    let page = host.history(&sid, None, 200).await.expect("history");
    eprintln!(
        "[hr] ④ history(200)RSS = {:.0} MB(窗口事件 {} 条,has_more={})",
        rss_mb(),
        page.events.len(),
        page.has_more
    );
    drop(page);

    let anchors = host.session_anchor_index(&sid).expect("anchors");
    eprintln!(
        "[hr] ⑤ anchor_index({} 锚)RSS = {:.0} MB",
        anchors.len(),
        rss_mb()
    );
    drop(anchors);

    let page = host.trajectory_page(&sid, 200, None).expect("trajectory");
    eprintln!(
        "[hr] ⑥ trajectory_page(total={})RSS = {:.0} MB",
        page.total,
        rss_mb()
    );
    drop(page);

    // ⑦ 深翻页到底(等效锚点 load-through 全量历史)
    let mut before = host.history(&sid, None, 200).await.expect("history 2").cut;
    let mut pages = 0;
    while before > 0 {
        let p = host.history(&sid, Some(before), 200).await.expect("page");
        let next = p.cut;
        drop(p);
        pages += 1;
        if next == 0 || next >= before {
            break;
        }
        before = next;
    }
    eprintln!("[hr] ⑦ 深翻页 ×{pages} 后 RSS = {:.0} MB", rss_mb());

    std::thread::sleep(std::time::Duration::from_millis(300));
    eprintln!("[hr] ⑧ 稳态 RSS = {:.0} MB", rss_mb());
    let _ = std::fs::remove_dir_all(&root);
}
