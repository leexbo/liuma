//! 按键 → 终端字节编码(xterm.js 序列表;无状态纯函数,先行可测)。
//!
//! 范围:v1 覆盖方向键/导航键/功能键/修饰键组合/可打印字符/粘贴;
//! 鼠标事件编码与 kitty keyboard protocol 不做(见切片模块注释)。
//! `platform` 修饰(macOS ⌘ / 其他平台 ctrl)**永不编码**——保留给
//! UI 快捷键体系,终端只收其余修饰组合。

use alacritty_terminal::term::TermMode;
use gpui_kit::Keystroke;

/// 修饰键参数 `m`(CSI 1;<m>X 语义):1 + shift + alt*2 + ctrl*4。
/// 返回 None = 无修饰(序列里省略 `;m` 段)
fn modifier_param(mods: &gpui_kit::Modifiers) -> Option<u8> {
    let m = 1u8 + mods.shift as u8 + 2 * mods.alt as u8 + 4 * mods.control as u8;
    (m > 1).then_some(m)
}

/// 方向/导航键的基准序列:APP_CURSOR 开(DECCKM)时方向键与 Home/End
/// 走 SS3 单字节尾,否则 CSI(`1;<m>` 形);返回 None = 不在本表
fn navigation_sequence(key: &str, app_cursor: bool) -> Option<(&'static str, bool)> {
    // (尾字符/数字, is_ss3)
    Some(match key {
        "up" | "arrowup" => ("A", app_cursor),
        "down" | "arrowdown" => ("B", app_cursor),
        "right" | "arrowright" => ("C", app_cursor),
        "left" | "arrowleft" => ("D", app_cursor),
        "home" => ("H", app_cursor),
        "end" => ("F", app_cursor),
        _ => return None,
    })
}

/// 带 `~` 结尾的导航键(无 SS3 形态;修饰版本走 `CSI n;<m>~`)
fn tilde_navigation(key: &str) -> Option<&'static str> {
    Some(match key {
        "insert" => "2",
        "delete" => "3",
        "pageup" => "5",
        "pagedown" => "6",
        _ => return None,
    })
}

/// 功能键 F1–F12(xterm.js 序列:F1–F4 SS3 P Q R S,F5–F12 CSI 编号,
/// 编号缺 16/20 等 xterm 历史空洞)
fn function_sequence(key: &str) -> Option<(&'static str, bool)> {
    Some(match key {
        "f1" => ("P", true),
        "f2" => ("Q", true),
        "f3" => ("R", true),
        "f4" => ("S", true),
        "f5" => ("15", false),
        "f6" => ("17", false),
        "f7" => ("18", false),
        "f8" => ("19", false),
        "f9" => ("20", false),
        "f10" => ("21", false),
        "f11" => ("23", false),
        "f12" => ("24", false),
        _ => return None,
    })
}

/// Ctrl + 字母 → 控制字节(letter-'a'+1;h 特例 = BS,i/j/m 与
/// tab/enter 同码由字母表自然得出)
fn ctrl_byte(key: &str) -> Option<u8> {
    let mut chars = key.chars();
    let (c, rest) = (chars.next()?, chars.next());
    if rest.is_some() || !c.is_ascii_lowercase() {
        return None;
    }
    Some(match c {
        'h' => 0x08,
        letter => letter as u8 - b'a' + 1,
    })
}

/// 编码一次按键。返回 None = 交给 UI(未映射/⌘ 修饰)。
///
/// 字符面:可打印键优先取 `key_char`(布局真实产出字符,如 Option 组合
/// 在 macOS 的合成字符**不可靠**,Alt 路径改用 `key` 基键);控制路径
/// (Ctrl/导航/功能)以 `key` 名称为准
pub fn encode_key(keystroke: &Keystroke, mode: &TermMode) -> Option<Vec<u8>> {
    let mods = &keystroke.modifiers;
    // ⌘/Win 永不进终端;function(Fn)由平台消费
    if mods.platform || mods.function {
        return None;
    }
    let key = keystroke.key.as_str();
    let app_cursor = mode.contains(TermMode::APP_CURSOR);

    // 命名键面:Enter/Backspace/Tab/Esc + 导航 + 功能
    match key {
        "enter" => {
            let mut out = vec![0x1b];
            if mods.alt {
                out.push(0x0d);
            } else {
                return Some(vec![0x0d]);
            }
            return Some(out);
        }
        "backspace" => {
            if mods.alt {
                return Some(vec![0x1b, 0x7f]);
            }
            return Some(vec![0x7f]);
        }
        "tab" => {
            return Some(if mods.shift {
                b"\x1b[Z".to_vec()
            } else {
                vec![0x09]
            });
        }
        "escape" => return Some(vec![0x1b]),
        _ => {}
    }

    if let Some((tail, is_ss3)) = navigation_sequence(key, app_cursor) {
        return Some(match (is_ss3, modifier_param(mods)) {
            (true, None) => vec![0x1b, b'O', tail.as_bytes()[0]],
            (true, Some(_)) => {
                // SS3 无修饰扩展形态:退回 CSI 1;<m> 尾(xterm 同款)
                format!("\x1b[1;{m}{tail}", m = modifier_param(mods)?).into_bytes()
            }
            (false, None) => format!("\x1b[{tail}").into_bytes(),
            (false, Some(m)) => format!("\x1b[1;{m}{tail}").into_bytes(),
        });
    }

    if let Some(n) = tilde_navigation(key) {
        return Some(match modifier_param(mods) {
            None => format!("\x1b[{n}~").into_bytes(),
            Some(m) => format!("\x1b[{n};{m}~").into_bytes(),
        });
    }

    if let Some((tail, is_ss3)) = function_sequence(key) {
        return Some(match (is_ss3, modifier_param(mods)) {
            (true, None) => vec![0x1b, b'O', tail.as_bytes()[0]],
            (true, Some(m)) => format!("\x1b[1;{m}{tail}").into_bytes(),
            (false, None) => format!("\x1b[{tail}~").into_bytes(),
            (false, Some(m)) => format!("\x1b[{tail};{m}~").into_bytes(),
        });
    }

    // 空格:Ctrl+Space = NUL,普通空格走字符面
    if key == "space" {
        return Some(if mods.control {
            vec![0x00]
        } else {
            b" ".to_vec()
        });
    }

    // Ctrl + 字母 → 控制字节(Shift 按下不改变控制码语义)
    if mods.control
        && !mods.alt
        && let Some(b) = ctrl_byte(key)
    {
        return Some(vec![b]);
    }

    // 字符面:单字符 key 才可编码;Alt 前缀 ESC + 基键字节(用 key 而
    // 非 key_char——Option 组合的 key_char 是 å 一类合成字符)
    let mut chars = key.chars();
    let (c, rest) = (chars.next()?, chars.next());
    if rest.is_some() {
        return None;
    }
    if mods.alt {
        return Some({
            let mut out = vec![0x1b];
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            out
        });
    }
    if mods.control || mods.shift && !c.is_ascii_graphic() && !c.is_ascii_whitespace() {
        // Ctrl+非字母(数字/符号)与怪异 shift 组合 v1 不编码
        return None;
    }
    let text = keystroke.key_char.as_deref().unwrap_or(key);
    if text.chars().count() == 1 {
        Some(text.as_bytes().to_vec())
    } else {
        None
    }
}

/// 编码粘贴文本:BRACKETED_PASTE 开时包 `ESC[200~ … ESC[201~`(shell/
/// TUI 据此把多行粘成编辑缓冲而非立即执行),换行统一归一为 `\r`
pub fn encode_paste(text: &str, mode: &TermMode) -> Vec<u8> {
    let normalized: String = text.replace("\r\n", "\r").replace('\n', "\r");
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let mut out = b"\x1b[200~".to_vec();
        out.extend_from_slice(normalized.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        normalized.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(key: &str, ctrl: bool, alt: bool, shift: bool) -> Keystroke {
        Keystroke {
            modifiers: gpui_kit::Modifiers {
                control: ctrl,
                alt,
                shift,
                platform: false,
                function: false,
            },
            key: key.to_string(),
            key_char: (!key.is_empty()).then(|| key.to_string()),
        }
    }

    fn plain(key: &str) -> Keystroke {
        ks(key, false, false, false)
    }

    #[test]
    fn printable_and_named_keys() {
        let mode = TermMode::empty();
        assert_eq!(encode_key(&plain("a"), &mode), Some(b"a".to_vec()));
        assert_eq!(encode_key(&plain("enter"), &mode), Some(b"\r".to_vec()));
        assert_eq!(
            encode_key(&plain("backspace"), &mode),
            Some(b"\x7f".to_vec())
        );
        assert_eq!(encode_key(&plain("tab"), &mode), Some(b"\t".to_vec()));
        assert_eq!(
            encode_key(&ks("tab", false, false, true), &mode),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(encode_key(&plain("escape"), &mode), Some(b"\x1b".to_vec()));
        // 中文字符经 key_char 直出 UTF-8
        let mut cjk = plain("体");
        cjk.key_char = Some("体".to_string());
        assert_eq!(encode_key(&cjk, &mode), Some("体".as_bytes().to_vec()));
    }

    #[test]
    fn arrows_respect_app_cursor_and_modifiers() {
        let normal = TermMode::empty();
        let app = TermMode::APP_CURSOR;
        assert_eq!(encode_key(&plain("up"), &normal), Some(b"\x1b[A".to_vec()));
        assert_eq!(encode_key(&plain("up"), &app), Some(b"\x1bOA".to_vec()));
        assert_eq!(
            encode_key(&plain("left"), &normal),
            Some(b"\x1b[D".to_vec())
        );
        // Ctrl+Right = \x1b[1;5C
        assert_eq!(
            encode_key(&ks("right", true, false, false), &normal),
            Some(b"\x1b[1;5C".to_vec())
        );
        // Shift+Down = \x1b[1;2B;Home 无修饰 CSI H,APP_CURSOR 下 SS3 H
        assert_eq!(
            encode_key(&ks("down", false, false, true), &normal),
            Some(b"\x1b[1;2B".to_vec())
        );
        assert_eq!(
            encode_key(&plain("home"), &normal),
            Some(b"\x1b[H".to_vec())
        );
        assert_eq!(encode_key(&plain("home"), &app), Some(b"\x1bOH".to_vec()));
        assert_eq!(encode_key(&plain("end"), &normal), Some(b"\x1b[F".to_vec()));
    }

    #[test]
    fn tilde_and_function_keys() {
        let mode = TermMode::empty();
        assert_eq!(
            encode_key(&plain("delete"), &mode),
            Some(b"\x1b[3~".to_vec())
        );
        assert_eq!(
            encode_key(&plain("pageup"), &mode),
            Some(b"\x1b[5~".to_vec())
        );
        assert_eq!(
            encode_key(&plain("pagedown"), &mode),
            Some(b"\x1b[6~".to_vec())
        );
        assert_eq!(
            encode_key(&ks("delete", true, false, false), &mode),
            Some(b"\x1b[3;5~".to_vec())
        );
        assert_eq!(encode_key(&plain("f1"), &mode), Some(b"\x1bOP".to_vec()));
        assert_eq!(encode_key(&plain("f5"), &mode), Some(b"\x1b[15~".to_vec()));
        assert_eq!(encode_key(&plain("f12"), &mode), Some(b"\x1b[24~".to_vec()));
    }

    #[test]
    fn ctrl_table_alt_prefix_and_cmd_passthrough() {
        let mode = TermMode::empty();
        assert_eq!(
            encode_key(&ks("c", true, false, false), &mode),
            Some(vec![0x03])
        );
        assert_eq!(
            encode_key(&ks("u", true, false, false), &mode),
            Some(vec![0x15])
        );
        assert_eq!(
            encode_key(&ks("h", true, false, false), &mode),
            Some(vec![0x08])
        );
        assert_eq!(
            encode_key(&ks("space", true, false, false), &mode),
            Some(vec![0x00])
        );
        // Alt 前缀 ESC + 基键(不用 key_char)
        assert_eq!(
            encode_key(&ks("b", false, true, false), &mode),
            Some(b"\x1bb".to_vec())
        );
        // ⌘ 永不编码(交还 UI 快捷键)
        let mut cmd = plain("c");
        cmd.modifiers.platform = true;
        assert_eq!(encode_key(&cmd, &mode), None);
        // 多字符 key(非单键)不编码
        assert_eq!(encode_key(&plain("foo"), &mode), None);
    }

    #[test]
    fn paste_bracketed_and_line_normalization() {
        let plain_mode = TermMode::empty();
        let bracketed = TermMode::BRACKETED_PASTE;
        assert_eq!(encode_paste("ls\npwd", &plain_mode), b"ls\rpwd".to_vec());
        assert_eq!(
            encode_paste("a\r\nb", &bracketed),
            b"\x1b[200~a\rb\x1b[201~".to_vec()
        );
    }
}
