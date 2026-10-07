//! 地址栏输入规范化:唯一入口 [`normalize_browser_url`]。只放行
//! http/https;无 scheme 的输入默认补 `https://`(浏览器通用习惯),
//! 例外 = IP 字面量(loopback/私网/任意 IPv4·IPv6,dev server 场景
//! `localhost:3000`、`192.168.1.10:5173` 落 http)。拒绝:空输入、
//! 内嵌凭据、空 host、超长输入。文件与脚本类 scheme
//! (file:/data:/javascript:/blob:)天然不在白名单内。

/// 地址上限(字节;对齐 dsh 侧栏浏览器 16KiB 上限)
const MAX_URL_BYTES: usize = 16 * 1024;

/// 规范化拒绝原因(逐类映射文案;携带 scheme 原文供展示)
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UrlReject {
    /// 空输入(不映射错误文案,静默忽略)
    Empty,
    /// scheme 不在 http/https 白名单
    Scheme(String),
    /// URL 内嵌 user:pass@ 凭据
    Credentials,
    /// host 为空
    NoHost,
    /// 超长输入
    TooLong,
    /// 解析失败(非白名单之外的畸形)
    Malformed,
}

/// 规范化地址栏输入为可加载 URL。
///
/// 补 scheme 规则:输入不含 `://` 且不以已知 scheme `:` 开头时,
/// loopback 主机(`localhost` / `127.x.x.x` / `[::1]`)补 `http://`,
/// 其余补 `https://`。返回规范化后的完整 URL 字符串。
pub(crate) fn normalize_browser_url(input: &str) -> Result<String, UrlReject> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(UrlReject::Empty);
    }
    if trimmed.len() > MAX_URL_BYTES {
        return Err(UrlReject::TooLong);
    }
    let candidate = if let Some(scheme) = bare_scheme(trimmed) {
        // `scheme:` 形式但不带 //(data:/javascript:/mailto: 之类)——
        // 直接按协议白名单拒绝(报协议名,而非笼统的解析失败)
        if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
            format!("{scheme}://{rest}", rest = &trimmed[scheme.len() + 1..])
        } else {
            return Err(UrlReject::Scheme(scheme.to_string()));
        }
    } else if trimmed.contains("://") {
        trimmed.to_string()
    } else if is_plain_http_host(trimmed) {
        format!("http://{trimmed}")
    } else {
        format!("https://{trimmed}")
    };
    let parsed = url::Url::parse(&candidate).map_err(|e| match e {
        url::ParseError::EmptyHost => UrlReject::NoHost,
        _ => UrlReject::Malformed,
    })?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(UrlReject::Scheme(parsed.scheme().to_string()));
    }
    if parsed.username() != "" || parsed.password().is_some() {
        return Err(UrlReject::Credentials);
    }
    let host = parsed.host_str().unwrap_or("");
    if host.is_empty() {
        return Err(UrlReject::NoHost);
    }
    Ok(parsed.to_string())
}

/// `scheme:` 形式输入的协议名(`data:text/html` → Some("data"))。
/// 判据:冒号前是合法 scheme 名(字母开头,字母数字+.-)且**不含**
/// `://`,且冒号后不是纯数字端口——`localhost:3000` / `[::1]:5173` 是
/// host:port,不是 scheme 输入
fn bare_scheme(input: &str) -> Option<&str> {
    let colon = input.find(':')?;
    let scheme = &input[..colon];
    let rest = &input[colon + 1..];
    let valid_name = scheme
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.');
    let is_port = !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit());
    (valid_name && !is_port).then_some(scheme)
}

/// 无 scheme 输入是否明文 http 主机:`localhost` 或 IP 字面量
/// (任意 IPv4/IPv6,含 loopback 与私网——dev server 心智;可带端口)
fn is_plain_http_host(input: &str) -> bool {
    let host = if let Some(end) = input.find(']') {
        &input[..=end]
    } else {
        input.split(':').next().unwrap_or("")
    };
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare == "localhost" || is_ipv4_literal(bare) || bare.contains(':')
}

/// IPv4 字面量判定(四段 0-255 数字)
fn is_ipv4_literal(bare: &str) -> bool {
    let mut parts = bare.split('.');
    (0..4).all(|_| match parts.next() {
        Some(p) => {
            !p.is_empty()
                && p.len() <= 3
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u16>().is_ok_and(|n| n <= 255)
        }
        None => false,
    }) && parts.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(input: &str) -> String {
        normalize_browser_url(input).unwrap()
    }

    fn reject(input: &str) -> UrlReject {
        normalize_browser_url(input).unwrap_err()
    }

    #[test]
    fn scheme_completion_defaults_to_https_except_ip_literals() {
        assert_eq!(ok("localhost:3000"), "http://localhost:3000/");
        assert_eq!(ok("localhost"), "http://localhost/");
        assert_eq!(ok("127.0.0.1:8080"), "http://127.0.0.1:8080/");
        assert_eq!(ok("192.168.1.10:5173"), "http://192.168.1.10:5173/");
        assert_eq!(ok("[::1]:5173"), "http://[::1]:5173/");
        assert_eq!(ok("[fe80::1]:3000"), "http://[fe80::1]:3000/");
        // 域名一律 https(显式 scheme 不受影响)
        assert_eq!(ok("example.com"), "https://example.com/");
    }

    #[test]
    fn scheme_completion_defaults_to_https() {
        assert_eq!(ok("example.com"), "https://example.com/");
        assert_eq!(ok("example.com/docs"), "https://example.com/docs");
    }

    #[test]
    fn explicit_scheme_preserved_regardless_of_host() {
        assert_eq!(ok("https://localhost:3000"), "https://localhost:3000/");
        assert_eq!(ok("HTTP://Example.com"), "http://example.com/");
    }

    #[test]
    fn non_http_schemes_rejected() {
        assert_eq!(
            reject("file:///etc/passwd"),
            UrlReject::Scheme("file".into())
        );
        assert_eq!(reject("data:text/html,x"), UrlReject::Scheme("data".into()));
        assert_eq!(
            reject("javascript:alert(1)"),
            UrlReject::Scheme("javascript".into())
        );
        assert_eq!(reject("blob:https://x"), UrlReject::Scheme("blob".into()));
    }

    #[test]
    fn credentials_and_empty_host_rejected() {
        assert_eq!(
            reject("https://user:pass@example.com"),
            UrlReject::Credentials
        );
        assert_eq!(reject("http://"), UrlReject::NoHost);
    }

    #[test]
    fn empty_and_oversize_rejected() {
        assert_eq!(reject("   "), UrlReject::Empty);
        assert_eq!(reject(&"a".repeat(16 * 1024 + 1)), UrlReject::TooLong);
    }

    #[test]
    fn malformed_rejected() {
        assert_eq!(reject("https://exa mple.com"), UrlReject::Malformed);
    }
}
