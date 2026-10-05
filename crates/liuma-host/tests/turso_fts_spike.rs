//! turso FTS 支持性验证 spike。
//! turso 的 FTS **不是** SQLite FTS5 虚表,而是自研索引方法:
//! `CREATE INDEX ... USING fts (列) WITH (tokenizer='...')`(tantivy 后端),
//! 查询用元组 MATCH。分词器五选(default/raw/simple/whitespace/ngram),
//! ngram(2-3)对中文子串友好。本 spike 验证:CJK 子串/整词、英文前缀、
//! 单字限制、highlight 函数。

// 集成测试基建(wasm 实例化/mock 装配)允许 unwrap/expect;clippy 的
// allow-in-tests 只认 #[test] 函数与 cfg(test) 模块,盖不到本目录辅助函数
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{SystemTime, UNIX_EPOCH};

fn rand_tag() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0)
}

async fn tmp_db(tag: &str) -> (turso::Connection, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "liuma-fts-spike-{tag}-{}.db",
        std::process::id() * 1000 + rand_tag() as u32
    ));
    let db = turso::Builder::new_local(path.to_str().unwrap())
        .experimental_index_method(true) // fts 索引方法为实验特性
        .build()
        .await
        .expect("建库");
    let conn = db.connect().expect("连接");
    (conn, path)
}

async fn texts(conn: &turso::Connection, sql: &str) -> Vec<String> {
    let mut out = vec![];
    let Ok(mut rows) = conn.query(sql, ()).await else {
        eprintln!("[spike] 查询失败: {sql}");
        return out;
    };
    while let Ok(Some(row)) = rows.next().await {
        if let Ok(turso::Value::Text(t)) = row.get_value(0) {
            out.push(t.to_string());
        }
    }
    out
}

#[tokio::test]
async fn fts_ngram_cjk_and_prefix() {
    let (conn, path) = tmp_db("ngram").await;

    conn.execute(
        "CREATE TABLE docs (id INTEGER PRIMARY KEY, content TEXT)",
        (),
    )
    .await
    .expect("建表");
    let r = conn
        .execute(
            "CREATE INDEX docs_fts ON docs USING fts (content) WITH (tokenizer='ngram')",
            (),
        )
        .await;
    assert!(r.is_ok(), "fts 索引应可建: {r:?}");

    let docs = [
        (1, "修复了队列持久化的 bug"),
        (2, "added durable queue splice replay"),
        (3, "设置页面增加了 provider 注册表"),
        (4, "provider registry with credentials"),
    ];
    for (id, d) in docs {
        conn.execute(
            &format!("INSERT INTO docs (id, content) VALUES ({id}, '{d}')"),
            (),
        )
        .await
        .expect("插入");
    }

    // 中文子串(ngram 2-3)
    let sub = texts(&conn, "SELECT content FROM docs WHERE content MATCH '持久'").await;
    assert_eq!(sub.len(), 1, "子串「持久」应命中 1 条;got {sub:?}");

    // 中文整词
    let whole = texts(
        &conn,
        "SELECT content FROM docs WHERE content MATCH '队列持久化'",
    )
    .await;
    assert_eq!(
        whole.len(),
        1,
        "整词「队列持久化」应命中 1 条;got {whole:?}"
    );

    // 英文前缀:`*` 后缀在 0.8.1 仍不命中(PhrasePrefix 与 ngram 索引
    // 不联动);剥 `*` 后的裸词走 gram 短语 = 子串语义,命中
    let pfx = texts(
        &conn,
        "SELECT content FROM docs WHERE content MATCH 'provid*'",
    )
    .await;
    assert_eq!(pfx.len(), 0, "前缀 provid* 不命中(应用层剥 * 的依据)");
    let bare = texts(
        &conn,
        "SELECT content FROM docs WHERE content MATCH 'provid'",
    )
    .await;
    assert_eq!(bare.len(), 2, "裸前缀 provid 应命中两条;got {bare:?}");

    // 单字查询:ngram(2,3) 产不出 token——裸单字与单字 + `*` 都是
    // Ok 空结果(lenient 解析器吞掉 parser 错误),不会报错但永不命中;
    // 应用层走 LIKE 兜底才有语义(search.rs rewrite_query 的依据)
    let single = conn
        .query("SELECT content FROM docs WHERE content MATCH '队'", ())
        .await;
    assert!(single.is_ok(), "裸单字 MATCH 应 Ok(空结果)");
    let starred = conn
        .query("SELECT content FROM docs WHERE content MATCH '队*'", ())
        .await;
    assert!(starred.is_ok(), "单字 + `*` 亦 Ok(lenient 吞 parser 错误)");
    assert_eq!(
        texts(&conn, "SELECT content FROM docs WHERE content MATCH '队'")
            .await
            .len(),
        0,
        "单字 MATCH 永不命中,应用层走 LIKE 兜底"
    );

    // highlight 函数(fts_highlight):0.8.1 下对本用法完全惰性——
    // CJK 与英文整词都原样返回;应用层高亮是唯一路径(desktop hit_preview)
    let hl = texts(
        &conn,
        "SELECT fts_highlight(content, '持久', '[', ']') FROM docs WHERE id = 1",
    )
    .await;
    assert_eq!(hl.len(), 1);
    assert_eq!(
        hl[0], "修复了队列持久化的 bug",
        "fts_highlight 对 CJK 不标注"
    );
    let hl_en = texts(
        &conn,
        "SELECT fts_highlight(content, 'durable', '[', ']') FROM docs WHERE id = 2",
    )
    .await;
    assert_eq!(
        hl_en[0], "added durable queue splice replay",
        "fts_highlight 对英文整词亦不标注(函数惰性)"
    );

    // 英文子串(ngram 对查询侧同样切 gram)
    let en = texts(
        &conn,
        "SELECT content FROM docs WHERE content MATCH 'queue'",
    )
    .await;
    assert_eq!(en.len(), 1, "子串 queue 只在 id 2;got {en:?}");

    // 多词(OR 组合):设置 → 3;provider → 3/4
    let multi = texts(
        &conn,
        "SELECT content FROM docs WHERE content MATCH '设置 provider'",
    )
    .await;
    assert_eq!(multi.len(), 2, "多词 OR 应命中 id 3/4;got {multi:?}");

    let _ = std::fs::remove_file(&path);
}
