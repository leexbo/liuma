//! LLM transport 宿主侧:不变式闸门与假 provider。
//!
//! 闸门实现 agent-loop 的 [`LlmTransport`] 端口,包裹真实传输:
//! 出网请求的 messages/header 在边界做内容级 derive-and-compare
//! (期望侧 = 共享日志的 [`derive_messages`] + engine header),
//! 不一致即拒绝——组件无法绕过(「策略变机制」)。
//!
//! 假 provider 是可编程测试装备(llm-replay 思路):脚本化事件序列 +
//! 记录收到的请求供断言;e2e 不依赖真实网络。
//!
//! 总线传输(BusTransport)留在 liuma-host(依赖事件总线,属核心机制)。

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use liuma_agent_loop::{LlmEvent, LlmTransport, RequestHeader, TransportError};
use liuma_session::EventLog;
use liuma_session::events::derive_visible_messages;
use serde_json::Value;

use crate::invariant::{InvariantViolation, verify_request};

/// 不变式闸门:包裹内层传输,强制「模型可见 ⟺ 已记录」
pub struct InvariantGate<T> {
    inner: T,
    log: Arc<Mutex<EventLog>>,
}

/// 闸门拒绝(不变式违反)
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum GateError {
    /// 内容级不一致被拒
    #[error("invariant rejected: {0}")]
    Violation(#[from] InvariantViolation),
}

impl<T> InvariantGate<T> {
    /// 以共享日志视图包裹内层传输(与 LoopEngine 共享同一日志)
    pub fn new(inner: T, log: Arc<Mutex<EventLog>>) -> Self {
        Self { inner, log }
    }

    /// 共享日志视图(装配层与 engine 共用同一日志;不变式比对的期望侧)
    pub fn log(&self) -> Arc<Mutex<EventLog>> {
        Arc::clone(&self.log)
    }

    /// 内层传输引用(测试装备断言用)
    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// 解包内层传输
    pub fn into_inner(self) -> T {
        self.inner
    }

    /// 内容级校验:实际请求 vs 日志派生
    pub fn verify(&self, header: &RequestHeader, messages: &Value) -> Result<(), GateError> {
        let log = self
            .log
            .lock()
            .map_err(|_| GateError::Violation(internal_error("log 锁中毒")))?;
        // 期望侧与 engine 共用同一投影(裁剪/折叠策略栈;唯一实现)
        let derived_messages = derive_visible_messages(log.iter());
        let derived_header = header.to_json();
        verify_request(
            &derived_messages,
            messages,
            &derived_header,
            &derived_header,
        )?;
        Ok(())
    }
}

fn internal_error(msg: &str) -> InvariantViolation {
    InvariantViolation::MessagesDiverge {
        derived: msg.to_string(),
        actual: String::new(),
    }
}

/// 闸门转发一次性摘要调用(非会话面:不经 derive-and-compare;
/// 持久化由 engine 的 audit/call + compaction/summary 承担)。
/// **契约**:摘要请求 = 前缀 + 指令,本就不等于会话派生面,此处的
/// `summarize`/`summarize_stream` 因此只转发、永不校验——给它们补
/// derive-and-compare 会让每次折叠 100% 被拒。
impl<T: liuma_agent_loop::Summarizer> liuma_agent_loop::Summarizer for InvariantGate<T> {
    fn summarize<'a>(
        &'a mut self,
        header: &'a RequestHeader,
        messages: &'a Value,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
        self.inner.summarize(header, messages)
    }

    /// 进度回调必须一并转发:落到 trait 默认实现会退化为非流式
    /// (进度在闸门处静默丢失,同 `stream_events` 曾漏覆写的旧疾)
    fn summarize_stream<'a>(
        &'a mut self,
        header: &'a RequestHeader,
        messages: &'a Value,
        on_progress: &'a mut (dyn FnMut(usize) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
        self.inner.summarize_stream(header, messages, on_progress)
    }
}

impl<T: LlmTransport + Send> LlmTransport for InvariantGate<T> {
    async fn stream(
        &mut self,
        header: &RequestHeader,
        messages: &Value,
    ) -> Result<Vec<LlmEvent>, TransportError> {
        // 闸门拒绝 = 我方不变式违反,归 Other(不重试,直通 turn/error)
        self.verify(header, messages)
            .map_err(|e| TransportError::Other(e.to_string()))?;
        self.inner.stream(header, messages).await
    }

    /// 流式路径必须覆写并**转发**给内层——此前漏覆写,落到 trait
    /// 默认实现(先 `stream()` 攒完全量再整批发送),所有真实流量在
    /// 闸门处被攒批,下游表现为「假流式」(生产探针定位:
    /// reqwest/HttpTransport 均渐进,唯经闸门的路径整批)。
    /// 校验语义不变:请求侧 derive-and-compare 在发起前完成。
    async fn stream_events(
        &mut self,
        header: &RequestHeader,
        messages: &Value,
        tx: tokio::sync::mpsc::UnboundedSender<LlmEvent>,
    ) -> Result<(), TransportError> {
        if std::env::var_os("LIUMA_PROBE").is_some() {
            eprintln!("[g1] gate.stream_events 进入");
        }
        self.verify(header, messages)
            .map_err(|e| TransportError::Other(e.to_string()))?;
        if std::env::var_os("LIUMA_PROBE").is_some() {
            eprintln!("[g2] gate.verify 通过");
        }
        let r = self.inner.stream_events(header, messages, tx).await;
        if std::env::var_os("LIUMA_PROBE").is_some() {
            eprintln!("[g3] inner 完成: {}", r.is_ok());
        }
        r
    }
}

/// 假 provider:脚本化事件 + 请求录制(测试装备)
#[derive(Default)]
pub struct FakeProvider {
    /// 脚本:每次 stream 调用依序弹出一组事件;空则返回空序列
    pub script: Vec<Vec<LlmEvent>>,
    /// 录制:收到的 (header, messages)
    pub received: Vec<(RequestHeader, Value)>,
    /// 摘要脚本:summarize 调用依序弹出;空则回固定串
    pub summaries: Vec<String>,
    /// 摘要失败脚本:非空时 summarize 依序弹出错误(优先于 summaries)
    pub summary_errors: Vec<String>,
    /// 录制:summarize 收到的 (header, messages)——待折叠前缀逐字
    /// (回归锁:二次压缩前缀末条须 == through_seq 指向的消息)
    pub summary_inputs: Vec<(RequestHeader, Value)>,
}

impl FakeProvider {
    /// 空脚本
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一组事件(下一次 stream 调用返回)
    pub fn then(&mut self, events: Vec<LlmEvent>) -> &mut Self {
        self.script.push(events);
        self
    }
}

impl liuma_agent_loop::Summarizer for FakeProvider {
    fn summarize<'a>(
        &'a mut self,
        header: &'a RequestHeader,
        messages: &'a Value,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
        // 空回调走同一实现(回调须在 future 内构造,不可作临时值借出)
        Box::pin(async move {
            let mut noop = |_chars: usize| {};
            self.summarize_stream(header, messages, &mut noop).await
        })
    }

    /// 进度装备:按摘要长度吐两个递增 tick(半量 → 全量)再返回,
    /// 供引擎的节流/相位用例断言真实进度路径;失败脚本先行。
    fn summarize_stream<'a>(
        &'a mut self,
        header: &'a RequestHeader,
        messages: &'a Value,
        on_progress: &'a mut (dyn FnMut(usize) + Send),
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
        self.summary_inputs.push((header.clone(), messages.clone()));
        Box::pin(async move {
            if !self.summary_errors.is_empty() {
                return Err(self.summary_errors.remove(0));
            }
            let text: String = if self.summaries.is_empty() {
                "[fake summary]".into()
            } else {
                self.summaries.remove(0)
            };
            let total = text.chars().count();
            on_progress(total / 2);
            on_progress(total);
            Ok(text)
        })
    }
}

impl LlmTransport for FakeProvider {
    async fn stream(
        &mut self,
        header: &RequestHeader,
        messages: &Value,
    ) -> Result<Vec<LlmEvent>, TransportError> {
        self.received.push((header.clone(), messages.clone()));
        Ok(if self.script.is_empty() {
            Vec::new()
        } else {
            self.script.remove(0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use liuma_agent_loop::Summarizer as _;

    /// 只实现非流式入口的最小传输(host 测试里三处手写 impl 的形态)
    struct PlainTransport;

    impl liuma_agent_loop::Summarizer for PlainTransport {
        fn summarize<'a>(
            &'a mut self,
            _header: &'a RequestHeader,
            _messages: &'a Value,
        ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
            Box::pin(async { Ok("plain".to_string()) })
        }
    }

    fn header() -> RequestHeader {
        RequestHeader {
            model: "m".into(),
            system: String::new(),
            temperature: 0.0,
            reasoning_effort: None,
            tools: Vec::new(),
        }
    }

    /// 带进度入口的默认实现直通非流式入口:回调零调用、文本同源
    /// (非流式实现不因新方法被迫改造)
    #[tokio::test]
    async fn default_summarize_stream_matches_summarize() {
        let mut t = PlainTransport;
        let h = header();
        let empty = Value::Array(Vec::new());
        let mut ticks: Vec<usize> = Vec::new();
        let text = t
            .summarize_stream(&h, &empty, &mut |c| ticks.push(c))
            .await
            .unwrap();
        assert_eq!(text, "plain");
        assert!(ticks.is_empty(), "非流式实现不得伪造进度");
        assert_eq!(t.summarize(&h, &empty).await.unwrap(), text);
    }

    /// 闸门必须**转发**进度回调:落到 trait 默认实现 = 进度在闸门处
    /// 静默丢失(与 stream_events 曾漏覆写的旧疾同形)
    #[tokio::test]
    async fn gate_forwards_summarize_progress() {
        let log = Arc::new(Mutex::new(EventLog::new()));
        let mut provider = FakeProvider::new();
        provider.summaries.push("abcdefgh".into());
        provider.summaries.push("plain-again".into());
        let mut gate = InvariantGate::new(provider, log);
        let h = header();
        let mut ticks: Vec<usize> = Vec::new();
        let text = gate
            .summarize_stream(&h, &Value::Array(Vec::new()), &mut |c| ticks.push(c))
            .await
            .unwrap();
        assert_eq!(text, "abcdefgh");
        assert_eq!(ticks, vec![4, 8], "闸门透传两枚递增 tick");
        // 非流式入口与带进度入口同源(同一实现;脚本依序弹出第二枚)
        let plain = gate.summarize(&h, &Value::Array(Vec::new())).await.unwrap();
        assert_eq!(plain, "plain-again");
    }

    /// 摘要失败脚本先行(引擎终局 failed 相位的装备面)
    #[tokio::test]
    async fn fake_provider_summary_errors_win() {
        let mut provider = FakeProvider::new();
        provider.summaries.push("never".into());
        provider.summary_errors.push("boom".into());
        let h = header();
        let err = provider
            .summarize(&h, &Value::Array(Vec::new()))
            .await
            .unwrap_err();
        assert_eq!(err, "boom");
    }
}
