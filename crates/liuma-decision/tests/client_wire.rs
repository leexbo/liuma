//! wire 集成回归:本地 TCP 假端点上的全链路请求体/响应解码。
//!
//! 断言面:请求为协议形状(Authorization Bearer、state/model/questions)、
//! 官方与百炼两套响应样例均可解码、429 退避后重试成功、超时按
//! DecisionError::Timeout 收口。

// 集成测试的 serve 辅助函数不在 #[test] 体内,clippy 的
// allow-expect-in-tests 检测不到;本文件整体是测试 crate,显式豁免
#![allow(clippy::expect_used)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::time::Duration;

use liuma_decision::types::{DecisionRequest, Question};
use liuma_decision::{DecisionError, DecisionPort, SystemOneClient};

/// 起一个一次性本地端点:读入首个 HTTP 请求(原样交给 inspector),
/// 按 `status`/`body`(可含次数语义)回响应。返回 (url, join 句柄)。
fn serve_once(
    status: &'static str,
    body: &'static str,
    inspector: std::sync::Arc<dyn Fn(&str) + Send + Sync>,
) -> (String, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 8192];
        let read = stream.read(&mut buf).expect("read");
        let raw = String::from_utf8_lossy(&buf[..read]).to_string();
        inspector(&raw);
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).expect("write");
    });
    (format!("http://127.0.0.1:{port}"), handle)
}

fn sample_request() -> DecisionRequest {
    let mut criteria = BTreeMap::new();
    criteria.insert("yes_opt".to_string(), String::new());
    criteria.insert("other".to_string(), String::new());
    DecisionRequest {
        state: serde_json::json!({ "cmd": "rm -rf /tmp/build" }),
        questions: vec![(
            "risk".to_string(),
            Question::Choice {
                instructions: serde_json::json!("How risky is `cmd`?"),
                criteria,
            },
        )],
        model: "jev-latest".into(),
    }
}

#[tokio::test]
async fn happy_path_official_sample() {
    let body = r#"{"model":"jev-1.13.0","answers":{"risk":{"type":"choice","choice":"yes_opt","confidence":0.81,"probabilities":{"yes_opt":0.88,"other":0.12}}},"usage":{"input_tokens":318,"output_tokens":34}}"#;
    let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> = std::sync::Arc::default();
    let seen2 = seen.clone();
    let (url, handle) = serve_once(
        "200 OK",
        body,
        std::sync::Arc::new(move |raw| {
            seen2.lock().unwrap().push(raw.to_string());
        }),
    );
    let client = SystemOneClient::new(&url, Some("secret-key".into()), 4000).expect("client");
    let answers = DecisionPort::ask(&client, sample_request())
        .await
        .expect("answers");
    handle.join().expect("serve");
    assert_eq!(answers.model, "jev-1.13.0");
    assert_eq!(answers.usage.input_tokens, 318);
    let raw = &seen.lock().unwrap()[0];
    assert!(raw.contains("POST /v1/systemone"), "endpoint 拼接:{raw}");
    assert!(raw.contains("authorization: Bearer secret-key"), "鉴权头");
    assert!(raw.contains("\"model\":\"jev-latest\""), "model 字段");
    assert!(raw.contains("\"risk\""), "问题 id 保留");
    assert!(raw.contains("rm -rf /tmp/build"), "state 原样透传");
}

#[tokio::test]
async fn aliyun_style_response_without_output_tokens() {
    let body = r#"{"model":"decision-model-preview","request_id":"abc-1","answers":{"risk":{"type":"choice","choice":"other","confidence":0.4,"probabilities":{"yes_opt":0.5,"other":0.5}}},"usage":{"input_tokens":50},"latency_ms":31.5}"#;
    let (url, handle) = serve_once("200 OK", body, std::sync::Arc::new(|_| {}));
    let client = SystemOneClient::new(&url, Some("k".into()), 4000).expect("client");
    let answers = DecisionPort::ask(&client, sample_request())
        .await
        .expect("answers");
    handle.join().expect("serve");
    assert_eq!(answers.request_id.as_deref(), Some("abc-1"));
    assert_eq!(answers.usage.output_tokens, None);
}

/// 429(带 Retry-After)后重试成功:两连发,第一次 429 第二次 200
#[tokio::test]
async fn rate_limit_retries_then_succeeds() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let server = std::thread::spawn(move || {
        let bodies = [
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ];
        // 第一次:429
        {
            let (mut stream, _) = listener.accept().expect("accept1");
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            stream.write_all(bodies[0].as_bytes()).expect("w1");
        }
        // 第二次:200
        let (mut stream, _) = listener.accept().expect("accept2");
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        let body = r#"{"model":"m","answers":{},"usage":{"input_tokens":1}}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).expect("w2");
    });
    let client = SystemOneClient::new(&format!("http://127.0.0.1:{port}"), Some("k".into()), 8000)
        .expect("client");
    let answers = DecisionPort::ask(&client, sample_request())
        .await
        .expect("重试后成功");
    server.join().expect("serve");
    assert_eq!(answers.model, "m");
}

/// 鉴权失败:401 不重试直接返回 Auth
#[tokio::test]
async fn auth_error_is_fatal() {
    let (url, handle) = serve_once(
        "401 Unauthorized",
        r#"{"error":"bad key"}"#,
        std::sync::Arc::new(|_| {}),
    );
    let client = SystemOneClient::new(&url, Some("wrong".into()), 4000).expect("client");
    let err = DecisionPort::ask(&client, sample_request())
        .await
        .unwrap_err();
    handle.join().expect("serve");
    assert!(
        matches!(err, DecisionError::Auth { status: 401 }),
        "{err:?}"
    );
}

/// 硬截止:端点挂起 → DecisionError::Timeout(消费方 fail-open 依据)
#[tokio::test]
async fn deadline_enforced() {
    // 不 accept 的监听者:连接建立但永不响应
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        let keep = listener.accept();
        std::thread::sleep(Duration::from_secs(5));
        drop(keep);
    });
    let client = SystemOneClient::new(&format!("http://127.0.0.1:{port}"), Some("k".into()), 300)
        .expect("client");
    let err = DecisionPort::ask(&client, sample_request())
        .await
        .unwrap_err();
    assert!(matches!(err, DecisionError::Timeout), "{err:?}");
}
