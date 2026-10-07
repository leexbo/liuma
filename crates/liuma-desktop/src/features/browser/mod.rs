//! 浏览器功能切片(右栏「浏览器」标签):应用内网页视图,面向本地
//! dev server 预览与聊天链接联动。
//!
//! 结构:store(导航状态机:NavState 快照 + NavCommand 待执行命令)、
//! url(地址规范化:仅 http/https 含 loopback)、views(工具栏 + 正文)。
//! 原生 webview 挂载见 native.rs(仅 macOS;WKWebView 直挂 GPUIView)。
//! 范围外:书签/历史/下载/多窗口;标签列表不持久化(同终端/预览)。

pub(crate) mod store;
pub(crate) mod url;
pub(crate) mod views;

use gpui_kit::actions;

actions!(browser, [OpenPanelBrowser]);

use url::UrlReject;

/// 地址拒绝原因 → 用户可读文案(scheme 类带原文)
pub(crate) fn url_reject_message(reject: &UrlReject) -> String {
    use crate::kits::i18n::t;
    match reject {
        UrlReject::Empty => String::new(),
        UrlReject::Scheme(s) => t!("browser.err_scheme", scheme = s).to_string(),
        UrlReject::Credentials => t!("browser.err_credentials").to_string(),
        UrlReject::NoHost => t!("browser.err_no_host").to_string(),
        UrlReject::TooLong => t!("browser.err_too_long").to_string(),
        UrlReject::Malformed => t!("browser.err_malformed").to_string(),
    }
}

pub(crate) use views::render;
