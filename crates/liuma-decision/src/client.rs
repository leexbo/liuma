//! System One HTTP 客户端:出网传输 + 有界重试 + 总预算硬截止。
//!
//! 独立 reqwest 实例(不与 LLM 传输共享连接池:低频决策调用无收益,
//! 且避免与流式超时配置耦合)。鉴权 = Bearer api_key(明文直存配置/
//! 设置文件,与 provider api_key 字段同策略)。
//!
//! 失败语义面向消费方的 fail-open 契约:任何 `Err` 一律退回原路径,
//! 绝不上抛阻塞主流程。

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde_json::Value;
use thiserror::Error;

use crate::thresholds::{BASE_BACKOFF_MS, RETRY_MAX_ATTEMPTS};
use crate::types::{DecisionAnswers, DecisionRequest};

/// 决策端错误。消费方契约:任何变体都触发 fail-open 退回原路径。
#[derive(Debug, Clone, Error)]
pub enum DecisionError {
    /// 未启用 / 未配置(如 api_key 缺席)
    #[error("decision disabled: {0}")]
    Disabled(String),
    /// 总预算硬截止超时(含重试等待)
    #[error("decision timeout")]
    Timeout,
    /// 鉴权失败(401)
    #[error("decision auth failed: status {status}")]
    Auth {
        /// HTTP 状态码
        status: u16,
    },
    /// 限流(429;带 retry-after 则尊重)
    #[error("decision rate limited")]
    RateLimit {
        /// 服务端建议的等待时长(可缺)
        retry_after: Option<Duration>,
    },
    /// 服务端错误(529 / 其他 5xx)
    #[error("decision server error: {status}")]
    Server {
        /// HTTP 状态码
        status: u16,
        /// 响应体(截断责任在调用方)
        body: String,
    },
    /// 请求被拒(422 等)或意外状态
    #[error("decision invalid request: {0}")]
    Invalid(String),
    /// 传输层失败(连接 / TLS / 超时中间态)
    #[error("decision transport: {0}")]
    Transport(String),
    /// 响应解码失败(协议形状不符)
    #[error("decision decode: {0}")]
    Decode(String),
}

impl DecisionError {
    /// 是否值得重试:仅限流(429)与官方明示的过载(529)。
    /// 其余(鉴权/请求形状/解码)重试无意义,直接 fail-open。
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            DecisionError::RateLimit { .. } | DecisionError::Server { status: 529, .. }
        )
    }

    /// 服务端建议的等待时长(仅 429 可能携带)
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            DecisionError::RateLimit { retry_after } => *retry_after,
            _ => None,
        }
    }
}

/// 有界重试判定(纯函数,仿 `RetryPolicy::decide` 的可测形态)。
///
/// `attempt` 自 0 计(已重试次数);达 `max_attempts` 或不可重试即放弃。
/// 退避 = 指数(BASE_BACKOFF_MS << attempt),服务端 retry-after 更长则从之。
pub fn retry_decision(err: &DecisionError, attempt: u32, max_attempts: u32) -> Option<Duration> {
    if attempt >= max_attempts || !err.retryable() {
        return None;
    }
    let backoff = Duration::from_millis(BASE_BACKOFF_MS << attempt.min(4));
    Some(match err.retry_after() {
        Some(after) if after > backoff => after,
        _ => backoff,
    })
}

/// state 截断守卫:超长字符串态截尾并注明;对象/数组态超限时降级为
/// 截断字符串(协议对超长 state 会拒绝或截断,主动截断保证请求可发)。
pub fn fit_state(state: Value, max_chars: usize) -> Value {
    let len = match &state {
        Value::String(s) => s.chars().count(),
        other => serde_json::to_string(other)
            .map(|s| s.chars().count())
            .unwrap_or(0),
    };
    if len <= max_chars {
        return state;
    }
    let text = match &state {
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    let cut: String = text.chars().take(max_chars).collect();
    Value::String(format!("[truncated {len} → {max_chars} chars]\n{cut}"))
}

/// 解析 Retry-After 头(HTTP 标准为秒;非数字忽略)
fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// System One 协议 HTTP 客户端。
///
/// endpoint = `base_url` + `/v1/systemone`:TypeSafe 官方
/// (`https://api.typesafe.ai`)与百炼 compatible-mode 前缀
/// (`https://{ws}.{region}.maas.aliyuncs.com/compatible-mode`)同形拼接。
/// 鉴权 = Bearer api_key(与 provider 同策略:配置/设置文件直存明文)。
pub struct SystemOneClient {
    endpoint: String,
    /// API key(明文直存,同 provider api_key 字段;缺席 = ask 期 Disabled)
    api_key: Option<String>,
    timeout: Duration,
    http: reqwest::Client,
}

impl SystemOneClient {
    /// 构造(连接失败 = Transport;装配方 fail-open 判 None)
    pub fn new(
        base_url: &str,
        api_key: Option<String>,
        timeout_ms: u64,
    ) -> Result<Self, DecisionError> {
        let http = reqwest::Client::builder()
            .build()
            .map_err(|e| DecisionError::Transport(e.to_string()))?;
        Ok(Self {
            endpoint: format!("{}/v1/systemone", base_url.trim_end_matches('/')),
            api_key: api_key
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty()),
            timeout: Duration::from_millis(timeout_ms),
            http,
        })
    }

    /// key 解析(缺席 = Disabled,消费方据此跳过)
    fn resolve_key(&self) -> Result<String, DecisionError> {
        self.api_key
            .as_deref()
            .map(|k| format!("Bearer {k}"))
            .ok_or_else(|| DecisionError::Disabled("api_key 未配置".into()))
    }

    /// 单次请求(不含重试/截止;HTTP 状态 → 错误分类)
    async fn ask_once(&self, req: &DecisionRequest) -> Result<DecisionAnswers, DecisionError> {
        let key = self.resolve_key()?;
        let resp = self
            .http
            .post(&self.endpoint)
            .header("Authorization", key)
            .header("Content-Type", "application/json")
            .body(req.to_body().to_string())
            .send()
            .await
            .map_err(|e| DecisionError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_retry_after);
        let text = resp
            .text()
            .await
            .map_err(|e| DecisionError::Transport(e.to_string()))?;
        match status {
            200..=299 => {
                serde_json::from_str(&text).map_err(|e| DecisionError::Decode(e.to_string()))
            }
            401 => Err(DecisionError::Auth { status }),
            429 => Err(DecisionError::RateLimit { retry_after }),
            422 => Err(DecisionError::Invalid(text)),
            s if s >= 500 => Err(DecisionError::Server {
                status: s,
                body: text,
            }),
            s => Err(DecisionError::Invalid(format!(
                "unexpected status {s}: {}",
                &text[..text.chars().count().min(200)]
            ))),
        }
    }
}

impl crate::DecisionPort for SystemOneClient {
    fn ask(
        &self,
        req: DecisionRequest,
    ) -> Pin<Box<dyn Future<Output = Result<DecisionAnswers, DecisionError>> + Send + '_>> {
        Box::pin(async move {
            // 总预算硬截止:请求与重试退避都计入,耗尽即 Timeout
            let deadline = tokio::time::Instant::now() + self.timeout;
            let mut attempt = 0u32;
            loop {
                match tokio::time::timeout_at(deadline, self.ask_once(&req)).await {
                    // 预算耗尽
                    Err(_) => return Err(DecisionError::Timeout),
                    Ok(Ok(answers)) => return Ok(answers),
                    Ok(Err(err)) => match retry_decision(&err, attempt, RETRY_MAX_ATTEMPTS) {
                        Some(wait) => {
                            attempt += 1;
                            if tokio::time::Instant::now() + wait >= deadline {
                                return Err(DecisionError::Timeout);
                            }
                            tokio::time::sleep(wait).await;
                        }
                        None => return Err(err),
                    },
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn retry_only_rate_limit_and_overload() {
        let retriable = [
            DecisionError::RateLimit { retry_after: None },
            DecisionError::Server {
                status: 529,
                body: "overloaded".into(),
            },
        ];
        for err in &retriable {
            assert!(err.retryable(), "{err:?} 应可重试");
            assert!(retry_decision(err, 0, 2).is_some());
        }
        let fatal = [
            DecisionError::Auth { status: 401 },
            DecisionError::Invalid("bad".into()),
            DecisionError::Decode("x".into()),
            DecisionError::Transport("io".into()),
            DecisionError::Timeout,
            DecisionError::Disabled("k".into()),
            DecisionError::Server {
                status: 500,
                body: String::new(),
            },
        ];
        for err in &fatal {
            assert!(!err.retryable(), "{err:?} 不应重试");
            assert!(retry_decision(err, 0, 2).is_none());
        }
        // 上界:attempt 达 max_attempts 即放弃
        let rl = DecisionError::RateLimit { retry_after: None };
        assert!(retry_decision(&rl, 2, 2).is_none());
        assert!(retry_decision(&rl, 1, 2).is_some());
    }

    #[test]
    fn backoff_exponential_and_respects_retry_after() {
        let rl = DecisionError::RateLimit { retry_after: None };
        assert_eq!(retry_decision(&rl, 0, 2), Some(Duration::from_millis(200)));
        assert_eq!(retry_decision(&rl, 1, 2), Some(Duration::from_millis(400)));
        // retry-after 更长则从之;更短则取退避
        let long = DecisionError::RateLimit {
            retry_after: Some(Duration::from_secs(5)),
        };
        assert_eq!(retry_decision(&long, 0, 2), Some(Duration::from_secs(5)));
        let short = DecisionError::RateLimit {
            retry_after: Some(Duration::from_millis(50)),
        };
        assert_eq!(
            retry_decision(&short, 0, 2),
            Some(Duration::from_millis(200))
        );
    }

    #[test]
    fn fit_state_truncates_long_only() {
        let short = json!("hello");
        assert_eq!(fit_state(short.clone(), 100), short);
        let long = "x".repeat(300);
        let fit = fit_state(json!(long.clone()), 100);
        let s = fit.as_str().expect("截断后为字符串");
        assert!(s.starts_with("[truncated 300 → 100 chars]"));
        assert!(s.contains(&"x".repeat(100)));
        // 对象态超限:降级为截断字符串
        let obj = fit_state(json!({ "k": "x".repeat(300) }), 50);
        assert!(obj.is_string());
    }

    #[test]
    fn endpoint_concatenation_matches_both_providers() {
        let official = SystemOneClient::new("https://api.typesafe.ai/", Some("K".into()), 2000)
            .expect("client");
        assert_eq!(official.endpoint, "https://api.typesafe.ai/v1/systemone");
        let aliyun = SystemOneClient::new(
            "https://ws.cn-beijing.maas.aliyuncs.com/compatible-mode",
            Some("K".into()),
            2000,
        )
        .expect("client");
        assert_eq!(
            aliyun.endpoint,
            "https://ws.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/systemone"
        );
    }

    #[test]
    fn missing_key_is_disabled() {
        let client = SystemOneClient::new("https://api.typesafe.ai", None, 2000).expect("client");
        let err = futures_block_on(client.ask_once(&DecisionRequest {
            state: json!("s"),
            questions: vec![],
            model: "m".into(),
        }));
        assert!(matches!(err, Err(DecisionError::Disabled(_))), "{err:?}");
    }

    /// 单测内极简 block_on(仅覆盖同步路径;wire 全链路在 tests/client_wire)
    fn futures_block_on<F: Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("rt")
            .block_on(fut)
    }
}
