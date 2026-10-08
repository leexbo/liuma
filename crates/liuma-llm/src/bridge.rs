//! rig `HttpClientExt` 传输桥:liuma 的 reqwest 连接语义垫在 rig provider 之下。
//!
//! 桥拥有「连接语义」:连接池/TLS/超时(connect/read)、非 2xx 归类
//! (状态+头+体保留,交 [`crate::http`] 的 liuma 分类器)、200+非 SSE
//! 错误体嗅探(方言钩子,如 GLM 顶层 `{code,msg}`)。重试不在此层——
//! D41 语义在 engine(liuma-agent-loop)。
//!
//! rig 0.43 的传输契约是单向的(`HttpClientExt` 一次性 send_streaming,
//! 错误逐块上抛;SSE `retry:` 只解析不重连——四评源码级核实),与 D41
//! 「用户可见/可取消/audit 配对」不冲突,本桥是该契约的 liuma 实现。

use std::future::Future;
use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;
use liuma_agent_loop::TransportError;
use rig_http::http_client::{
    BoxedStream, Error as HttpError, HttpClientExt, LazyBody, MultipartForm, Request, Response,
    Result as HttpResult, StreamingResponse,
};
use rig_http::wasm_compat::WasmCompatSend;

use crate::http::{classify_reqwest, looks_like_sse};

/// 200 + 非 SSE JSON 错误体的方言归类钩子
pub(crate) type BodyErrorHook = Arc<dyn Fn(&str) -> Option<TransportError> + Send + Sync>;

/// liuma 传输桥:方言无关的连接语义 + 方言错误体钩子。
#[derive(Clone)]
pub(crate) struct Bridge {
    /// reqwest 客户端(连接池 + connect/read 超时由构建方注入)
    pub client: reqwest::Client,
    /// 200 + 非 SSE 错误体归类(方言差异)
    pub body_error: BodyErrorHook,
}

fn unused_surface() -> HttpError {
    HttpError::instance(TransportError::Other(
        "liuma 桥只承载流式补全面,单次/多部件面未使用".into(),
    ))
}

// clippy:manual_async_fn 在此不可满足——trait 的 RPITIT 带 + 'static
// 界定,async fn 借用 &self 的 future 无 'static
#[allow(clippy::manual_async_fn)]
impl HttpClientExt for Bridge {
    fn send<T, U>(
        &self,
        _req: Request<T>,
    ) -> impl Future<Output = HttpResult<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes> + WasmCompatSend,
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        async { Err(unused_surface()) }
    }

    fn send_multipart<U>(
        &self,
        _req: Request<MultipartForm>,
    ) -> impl Future<Output = HttpResult<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        async { Err(unused_surface()) }
    }

    fn send_streaming<T>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = HttpResult<StreamingResponse>> + WasmCompatSend
    where
        T: Into<Bytes> + WasmCompatSend,
    {
        let client = self.client.clone();
        let body_error = Arc::clone(&self.body_error);
        async move {
            let (parts, body) = req.into_parts();
            let bytes: Bytes = body.into();
            let uri = parts.uri.to_string();
            let mut request = client.request(parts.method, &uri);
            // reqwest 与 rig-http 共用 http 1.x 类型(锁文件统一)
            request = request.headers(parts.headers);
            let response = request
                .body(bytes)
                .send()
                .await
                .map_err(|e| HttpError::instance(classify_reqwest(e)))?;
            let status = response.status();
            if !status.is_success() {
                let headers = response.headers().clone();
                let body = response
                    .text()
                    .await
                    .unwrap_or_else(|e| format!("(读错误响应体失败:{e})"));
                // 状态+头+体原样保留;下游(engine)经 liuma 分类器归
                // AUTH/CONTEXT_OVERFLOW/RATE_LIMIT/SERVER/INVALID_REQUEST
                return Err(HttpError::non_success_with_details(status, headers, body));
            }

            // 200 + 非 SSE 体嗅探:坏 key 的端点以 200 + JSON 错误体应答
            // (GLM 实测形态)。缓冲至 SSE 确认;确认前收到非 SSE 字节 →
            // 攒全量交方言钩子归类,钩子 None → 按非预期响应体报错
            // (旧引擎语义为「空响应」可重试,此处 Transport 同为可重试)
            let body_error = Arc::clone(&body_error);
            let stream = async_stream::stream! {
                let mut upstream = Box::pin(response.bytes_stream());
                let mut sse_confirmed = false;
                let mut error_body = String::new();
                while let Some(chunk) = upstream.next().await {
                    let chunk = match chunk {
                        Ok(c) => c,
                        Err(e) => {
                            yield Err(HttpError::instance(classify_reqwest(e)));
                            return;
                        }
                    };
                    if !sse_confirmed {
                        if !looks_like_sse(&chunk) {
                            error_body.push_str(&String::from_utf8_lossy(&chunk));
                            continue;
                        }
                        sse_confirmed = true;
                    }
                    yield Ok(chunk);
                }
                if !error_body.is_empty() {
                    let err = body_error(&error_body).unwrap_or_else(|| {
                        TransportError::Other(format!(
                            "provider 非预期的非 SSE 响应体:{error_body}"
                        ))
                    });
                    yield Err(HttpError::instance(err));
                }
            };
            let boxed: BoxedStream = Box::pin(stream);
            Response::builder()
                .status(status)
                .header("content-type", "text/event-stream")
                .body(boxed)
                .map_err(HttpError::Protocol)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::ProviderAdapter as _;
    use rig_http::http_client::framing::Framing;

    /// 非 2xx → rig Error 保留状态/头/体(engine 分类器据此归类)
    #[tokio::test]
    async fn non_success_preserves_status_body() {
        let bridge = test_bridge(|_| None);
        let resp = match bridge.send_streaming(fake_request("x")).await {
            Ok(response) => panic!(
                "401 应为 Err,实得响应:{status:?}",
                status = response.status()
            ),
            Err(e) => e,
        };
        assert_eq!(resp.non_success_status().map(|s| s.as_u16()), Some(401));
        assert!(resp.non_success_body().unwrap_or("").contains("nope"));
    }

    /// 200 + 非 SSE 错误体 → 方言钩子归类(GLM 顶层形态 → AUTH)
    #[tokio::test]
    async fn sse_sniff_routes_body_error_hook() {
        let bridge =
            test_bridge(|body| crate::adapters::GlmResponsesAdapter::default().body_error(body));
        let err = bridge
            .send_streaming(fake_request(
                "{\"code\":401,\"msg\":\"令牌已过期或验证不正确\",\"success\":false}",
            ))
            .await;
        // 错误在流尾帧(嗅探需攒全量),流本身以 Err 收尾
        let mut stream = match err {
            Ok(response) => response.into_body(),
            Err(e) => panic!("200 应进入流:{e:?}"),
        };
        let mut saw_err = false;
        while let Some(item) = stream.next().await {
            if item.is_err() {
                saw_err = true;
            }
        }
        assert!(saw_err, "非 SSE 错误体必须在流尾产出 Err");
    }

    /// SSE 体直通(帧内容由 rig 解码)
    #[tokio::test]
    async fn sse_body_passes_through() {
        let bridge = test_bridge(|_| None);
        let response = bridge
            .send_streaming(fake_request("data: {\"a\":1}\n\n"))
            .await
            .expect("SSE 应直通");
        let body = response.into_body();
        let bytes: Vec<Bytes> = body.map(|c| c.expect("chunk")).collect().await;
        let mut joined = bytes.concat();
        // mock 服务器关停的零填充裁除(仅语义对比:SSE 直通)
        while joined.last() == Some(&0) {
            joined.truncate(joined.len() - 1);
        }
        assert_eq!(joined, &b"data: {\"a\":1}\n\n"[..]);
        // rig 组帧层可解析(WireFrame 提取)
        let frames = Framing::Sse.split(&joined);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].as_str(), "{\"a\":1}");
    }

    fn test_bridge(hook: fn(&str) -> Option<TransportError>) -> Bridge {
        // 回环 mock 不进 env 代理(reqwest 读环境在建 Client 时,清在
        // 构建前)
        crate::clear_env_proxies();
        Bridge {
            client: reqwest::Client::new(),
            body_error: Arc::new(hook),
        }
    }

    fn fake_request(body: &'static str) -> Request<&'static str> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            use std::io::{Read, Write};
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            // 从 body 首字符判 401 还是 200+SSE/JSON
            let body_str = String::from_utf8_lossy(&buf);
            let payload = body_str
                .split("\r\n\r\n")
                .nth(1)
                .unwrap_or_default()
                .to_string();
            let reply = if payload.starts_with("x") {
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope"
            } else if payload.starts_with("data:") {
                &format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{payload}"
                )
            } else {
                &format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{payload}"
                )
            };
            let _ = s.write_all(reply.as_bytes());
            let _ = s.shutdown(std::net::Shutdown::Write);
        });
        let url: rig_http::http_client::Uri = format!("http://{addr}/responses").parse().unwrap();
        Request::post(url).body(body).unwrap()
    }
}
