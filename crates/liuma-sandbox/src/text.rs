//! 子进程输出的解码。
//!
//! 为什么需要这层:宿主一律按 UTF-8 解释工具输出,但**平台 shell 不保证
//! 写 UTF-8**。Windows 上尤其明显——受限令牌下 PowerShell 会进
//! `ConstrainedLanguage`,而把输出编码改成 UTF-8 的那句 .NET 属性设置正是
//! 被语言模式挡掉的,于是输出落在系统代码页里(实测 zh-CN 为 936),按
//! UTF-8 解就成了乱码。宿主这一侧按平台的代码页兜底,是唯一不依赖 shell
//! 配合的做法。

/// 把子进程输出解成字符串。
///
/// 顺序:UTF-8 优先(绝大多数现代工具的输出),不合法则按 **Windows 的
/// ANSI 代码页**兜底——.NET 在无控制台的进程里以 `Encoding.Default`(即
/// ANSI 代码页)作为 `Console.OutputEncoding`,受限令牌下的 PowerShell 正是
/// 这种情形。两平台都不存在「猜」:UTF-8 的合法性是可判定的,兜底用的
/// 代码页由系统给出。
pub fn decode_output(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        #[cfg(windows)]
        Err(_) => decode_ansi(bytes).unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned()),
        #[cfg(not(windows))]
        Err(_) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// 按系统 ANSI 代码页解码(不可用时返回 `None`,由调用方回落 lossy)
#[cfg(windows)]
fn decode_ansi(bytes: &[u8]) -> Option<String> {
    use windows_sys::Win32::Globalization::{GetACP, MultiByteToWideChar};
    if bytes.is_empty() {
        return Some(String::new());
    }
    let cp = unsafe { GetACP() };
    let len = i32::try_from(bytes.len()).ok()?;
    // 容量按 UTF-16 最坏情况给足:每个字节至多产出一个码元
    let mut wide = vec![0u16; bytes.len() + 1];
    // SAFETY: 源缓冲长度与目标容量都已给出;cp 由系统提供
    let written = unsafe {
        MultiByteToWideChar(
            cp,
            0,
            bytes.as_ptr(),
            len,
            wide.as_mut_ptr(),
            wide.len() as i32,
        )
    };
    if written <= 0 {
        return None;
    }
    wide.truncate(written as usize);
    Some(String::from_utf16_lossy(&wide))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf8_verbatim() {
        assert_eq!(decode_output("中文-ok".as_bytes()), "中文-ok");
        assert_eq!(decode_output(b"plain ascii"), "plain ascii");
        assert_eq!(decode_output(b""), "");
    }

    /// 非 UTF-8 的字节不会 panic、不会丢成空串(平台代码页兜底或 lossy)
    #[test]
    fn decodes_non_utf8_without_panicking() {
        let gbk = [0xD6u8, 0xD0, 0xCE, 0xC4]; // zh-CN 代码页下的「中文」
        let decoded = decode_output(&gbk);
        assert!(!decoded.is_empty(), "兜底解码不得产出空串");
        #[cfg(windows)]
        assert_eq!(decoded, "中文", "应按系统 ANSI 代码页解出原文");
    }
}
