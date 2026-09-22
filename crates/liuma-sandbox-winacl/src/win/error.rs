//! Win32 调用错误:带上 API 名与错误码。
//!
//! 沙箱故障的排查难点在于「哪一步失败」——令牌、授权、Job、创建进程各有
//! 各的失败码,只报一个裸码等于没报。

use windows_sys::Win32::Foundation::GetLastError;

/// 一次 Win32 调用的失败
#[derive(Debug, Clone, thiserror::Error)]
#[error("{api} failed (Win32 error {code})")]
pub struct WinError {
    /// 失败的 API
    pub api: &'static str,
    /// `GetLastError` 的值
    pub code: u32,
    /// 附加上下文(如涉及的路径)
    pub context: Option<String>,
}

impl WinError {
    /// 取当前线程的上一次错误
    pub fn last(api: &'static str) -> Self {
        Self {
            api,
            code: unsafe { GetLastError() },
            context: None,
        }
    }

    /// 带上下文
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    /// 供 stderr 使用的单行描述
    pub fn render(&self) -> String {
        match &self.context {
            Some(ctx) => format!("{} ({}; {ctx})", self, self.api),
            None => format!("{} ({})", self, self.api),
        }
    }
}

/// Win32 布尔返回值判定(0 = 失败)
pub fn check(api: &'static str, ok: i32) -> Result<(), WinError> {
    if ok == 0 {
        Err(WinError::last(api))
    } else {
        Ok(())
    }
}
