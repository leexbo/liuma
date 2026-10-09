//! 集成测试:streamable-http 传输(**进程内** std TcpListener 手写 HTTP 端点)。
//! 验证:url 连接 + 工具发现 + 调用;配置 headers 原样到达 server
//! (Authorization 经 `echo_auth` 工具回显断言);URL 无效即时失败。
//!
//! fixture 是进程内 Rust 监听而非 python 子进程:CI runner 对「新起
//! 解释器 + 占口-释放-转交」的组合不稳定(子进程活着端口不通/首连
//! refused,形态逐台漂移),进程内监听与 liuma-core billing 测试同款,
//! runner 上实证可用。

// 集成测试基建(mock server/传输装配)允许 unwrap;clippy 的 allow-in-tests
// 只认 #[test] 函数与 cfg(test) 模块,盖不到本目录的辅助函数
#![allow(clippy::unwrap_used, clippy::expect_used)]

use liuma_agent_loop::CancelToken;
use liuma_agent_loop::tools::{ToolCallRequest, ToolPort};
use liuma_mcp::{McpServerConfig, McpServerPort, McpTransport};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// 起进程内 fixture:accept 循环线程 + 一连接一请求一响应
/// (Connection: close,同原 python BaseHTTPRequestHandler 的 HTTP/1.0
/// 逐请求重连语义;rmcp 客户端兼容)。线程随测试进程退出回收
fn start_fixture() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let auth: Arc<Mutex<String>> = Arc::new(Mutex::new("(none)".into()));
    let shared = Arc::clone(&auth);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let a = Arc::clone(&shared);
            // 单连接失败不倒线程(客户端侧断言兜底)
            let _ = serve_one(&mut s, &a);
        }
    });
    port
}

/// 单请求服务:读头 → POST 按 Content-Length 读体 → 按 JSON-RPC 路由
fn serve_one(s: &mut std::net::TcpStream, auth: &Mutex<String>) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    // 头体可能同包到达:分隔符可在缓冲区任意位置(要求「结尾恰是分隔
    // 符」会在头+体一包送达时死锁)
    let head_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        let n = s.read(&mut chunk)?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut body = buf[head_end + 4..].to_vec();
    let request_line = head.lines().next().unwrap_or_default().to_string();
    let is_post = request_line.starts_with("POST");
    if is_post {
        let want = header_value(&head, "content-length")
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while body.len() < want {
            let n = s.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
    }
    // Authorization 记账(echo_auth 回显的数据源)
    if let Some(v) = header_value(&head, "authorization") {
        *auth.lock().unwrap() = v.trim().to_string();
    }
    if !is_post {
        // GET → 405(照 MCP spec:无 SSE 流返回 405,client 须容忍);
        // DELETE → 200(shutdown)
        let status = if request_line.starts_with("DELETE") {
            "200 OK"
        } else {
            "405 Method Not Allowed"
        };
        return respond(s, status, "", None);
    }
    let body_str = String::from_utf8_lossy(&body).to_string();
    let Ok(m) = serde_json::from_str::<serde_json::Value>(&body_str) else {
        return respond(
            s,
            "400 Bad Request",
            r#"{"jsonrpc":"2.0","error":{"code":-32700,"message":"parse error"}}"#,
            None,
        );
    };
    // 通知(无 id)→ 202 空体
    if m.get("id").is_none() {
        return respond(s, "202 Accepted", "", None);
    }
    let id = m["id"].clone();
    match m["method"].as_str() {
        Some("initialize") => respond(
            s,
            "200 OK",
            &json!({
                "jsonrpc": "2.0", "id": id, "result": {
                    "protocolVersion": m["params"]["protocolVersion"].as_str().unwrap_or("2024-11-05"),
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "http-fixture", "version": "0"}
                }
            })
            .to_string(),
            Some("fixture-session"),
        ),
        Some("tools/list") => respond(
            s,
            "200 OK",
            &json!({
                "jsonrpc": "2.0", "id": id, "result": {"tools": [
                    {"name": "echo_auth", "description": "回显收到的 Authorization",
                     "inputSchema": {"type": "object"}}]}
            })
            .to_string(),
            None,
        ),
        Some("tools/call") => {
            let a = auth.lock().unwrap().clone();
            respond(
                s,
                "200 OK",
                &json!({
                    "jsonrpc": "2.0", "id": id, "result": {
                        "content": [{"type": "text", "text": format!("auth={a}")}],
                        "isError": false}
                })
                .to_string(),
                None,
            )
        }
        _ => respond(
            s,
            "200 OK",
            &json!({"jsonrpc": "2.0", "id": id,
                    "error": {"code": -32601, "message": "no method"}})
                .to_string(),
            None,
        ),
    }
}

/// 头部取值(大小写不敏感;首个命中)
fn header_value(head: &str, name: &str) -> Option<String> {
    head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.to_string())
    })
}

/// 写响应:HTTP/1.1 + Content-Length + Connection: close + 可选会话头
fn respond(
    s: &mut std::net::TcpStream,
    status: &str,
    body: &str,
    session: Option<&str>,
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(sid) = session {
        head.push_str(&format!("mcp-session-id: {sid}\r\n"));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes())?;
    s.write_all(body.as_bytes())?;
    s.flush()
}

fn http_config(url: String, headers: BTreeMap<String, String>) -> McpServerConfig {
    McpServerConfig {
        server_name: "httpsrv".into(),
        transport: McpTransport::StreamableHttp { url, headers },
        tool_call_timeout: std::time::Duration::from_secs(10),
    }
}

async fn wait_tools(port: &mut McpServerPort) {
    for _ in 0..80 {
        if !port.specs().is_empty() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("HTTP 工具未在期限内被发现");
}

#[tokio::test]
async fn http_connect_discover_call_and_headers_passthrough() {
    // 回归锁:环境代理不得劫持回环端点。带死代理跑本测试——修复前
    // initialize 逐次 error sending request(reqwest 吃 http_proxy 不豁
    // 回环;CI runner 的代理环境实证本缺陷)。reqwest 在 Client 构建
    // 时读环境,故必须在 McpServerPort::start 之前下毒
    unsafe {
        std::env::set_var("http_proxy", "http://127.0.0.1:9");
        std::env::set_var("https_proxy", "http://127.0.0.1:9");
    }
    let port = start_fixture();
    let mut headers = BTreeMap::new();
    headers.insert(
        "Authorization".to_string(),
        "Bearer e2e-test-token".to_string(),
    );
    let mut mcp_port = McpServerPort::start(
        http_config(format!("http://127.0.0.1:{port}/mcp"), headers),
        CancelToken::new(),
        None,
        None,
    );
    wait_tools(&mut mcp_port).await;
    assert_eq!(
        mcp_port.specs()[0]["function"]["name"],
        "mcp__httpsrv__echo_auth",
        "公共名 mcp__<server>__<raw>"
    );
    let out = ToolPort::execute(
        &mut mcp_port,
        &ToolCallRequest {
            name: "mcp__httpsrv__echo_auth".into(),
            arguments: json!({}),

            id: String::new(),
        },
    )
    .await;
    assert!(out.success, "{}", out.output);
    // 配置 headers 原样到达 server(headers dict 不 scrub 不改写)
    assert_eq!(
        out.output, "auth=Bearer e2e-test-token",
        "Authorization 原样透传"
    );
    mcp_port.shutdown();
    // 毒环境用后即清(进程级变量不外溢给同二进制的其它用例)
    unsafe {
        std::env::remove_var("http_proxy");
        std::env::remove_var("https_proxy");
    }
}

#[tokio::test]
async fn http_invalid_url_fails_fast_with_status() {
    let mcp_port = McpServerPort::start(
        http_config("not a url".into(), BTreeMap::new()),
        CancelToken::new(),
        None,
        None,
    );
    // 无效 URL 在构建传输时即失败 → 状态 Failed(带文案),工具零声明
    for _ in 0..50 {
        if matches!(mcp_port.status(), liuma_mcp::McpStatus::Failed(_)) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("无效 URL 应快速落 Failed");
}
