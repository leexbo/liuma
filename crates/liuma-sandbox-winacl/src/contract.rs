//! Windows 沙箱后端与调用方之间的契约(纯逻辑,全平台编译与单测)。
//!
//! 两件事在这里定死,因为它们在两侧都要用同一份实现:
//! - **能力 SID 的派生**:调用方按 canonical 路径算,runner 按自己
//!   canonicalize 的结果再算一遍并对账——两边不一致就拒绝执行。这是
//!   「同一目录因 `\\?\` 前缀 / 大小写 / 8.3 短名铸出两个身份、ACE 打在
//!   一套身份而令牌带另一套」这类漂移的唯一防线。
//! - **runner 的 argv 形态**:可重复的 `--writable <目录> <期望 SID>` 而非
//!   固定的工作区/暂存区两槽——可写根多于两个时不该退化成拒绝执行。
//!
//! 这里**不做**「暂存区与可写根互不包含」判定:当前形态下两个根都由调用方
//! 显式给出(工作区 + 系统暂存区),暂存区不构成独立身份,谁包含谁都不产生
//! 越权,该判定只会误拒「工作区恰好位于 %TEMP% 下」的正当用法。等到引入
//! 「每会话私有暂存区」(独立能力身份)时才需要它。

use std::ffi::OsString;
use std::path::PathBuf;

/// runner 失败时的退出码(命令从未执行的信号)
pub const EXIT_RUNNER_FAILURE: i32 = 127;

/// runner 失败时 stderr 的前缀。**必须与退出码成对判定**——只认前缀的话,
/// 一条恰好打印该前缀的受限命令会被误判成「没跑」
pub const FAIL_PREFIX: &str = "liuma-sandbox-run: ";

/// 能力 SID 的目录授权掩码:`FILE_GENERIC_WRITE | DELETE | FILE_DELETE_CHILD`
/// 去掉 `STANDARD_RIGHTS_WRITE`(0x00020000),即
/// `0x00120116 | 0x00010000 | 0x00000040) & !0x00020000 = 0x000110156`。
///
/// 故意不含 `WRITE_DAC` / `WRITE_OWNER`:受限子进程无法改写目录的 DACL 或
/// 夺取所有权,授权边界不被自己撬开。
pub const GRANT_MASK: u32 = 0x0011_0156;

/// 沙箱模式(runner 侧词汇;与 `liuma_sandbox::SandboxMode` 一一对应)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 只读:不启用写限制的授予,历史 ACE 保持惰性
    ReadOnly,
    /// 工作区可写:至少一个可写根
    WorkspaceWrite,
    /// 全盘放行:不下发 `WRITE_RESTRICTED`,但进程仍入 Job
    FullAccess,
}

impl Mode {
    /// argv 词汇
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::ReadOnly => "read-only",
            Mode::WorkspaceWrite => "workspace-write",
            Mode::FullAccess => "full-access",
        }
    }

    /// 解析 argv 词汇
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "read-only" => Some(Mode::ReadOnly),
            "workspace-write" => Some(Mode::WorkspaceWrite),
            "full-access" => Some(Mode::FullAccess),
            _ => None,
        }
    }

    /// 本模式是否要求可写根(制定 argv 一致性校验的规则)
    pub fn requires_writable_roots(self) -> bool {
        matches!(self, Mode::WorkspaceWrite)
    }
}

/// 一个可写根:目录 + 调用方按该目录算出的能力 SID
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritableRoot {
    /// 目录(canonical 形式由消费者归一)
    pub dir: PathBuf,
    /// 调用方算出的能力 SID(runner 会重算对账)
    pub sid: String,
}

/// runner 的完整调用规格
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerSpec {
    /// 模式
    pub mode: Mode,
    /// 可写根(顺序即授权顺序)
    pub writable: Vec<WritableRoot>,
    /// 载荷程序
    pub program: String,
    /// 载荷参数
    pub args: Vec<String>,
}

/// 能力 SID(工作区身份):`S-1-4-<a>-<b>`。
///
/// 确定性派生 —— 同一工作区在每台机器上是同一个身份,ACE 因此可以常驻并
/// 按精确命中跳过重复授予。`canonical` 必须是 canonical 路径
/// (见 [`normalize_canonical`]),否则同一目录的两种拼写会派生两个身份。
pub fn workspace_sid(canonical: &str) -> String {
    let (a, b) = derive(canonical, "liuma-workspace");
    format!("S-1-4-{a}-{b}")
}

/// 域分离的确定性派生:两个 30 位子授权域。
///
/// 30 位(而非 32)是留白:S-1-4-… 是自铸身份区,值域收窄可避免与将来
/// 系统定义撞上。
fn derive(canonical: &str, domain: &str) -> (u32, u32) {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0u8]); // 域与路径之间不可歧义拼接
    hasher.update(canonical.as_bytes());
    let digest = hasher.finalize();
    let take = |i: usize| {
        let raw = u32::from_le_bytes([digest[i], digest[i + 1], digest[i + 2], digest[i + 3]]);
        raw % ((1 << 30) - 1) + 1
    };
    (take(0), take(4))
}

/// canonical 路径归一:去掉 Windows 的 `\\?\` 与 `\\?\UNC\` 前缀。
///
/// `std::fs::canonicalize` 在 Windows 上产出带前缀的路径,而 SID 派生与
/// 用户可见的路径都不该带它 —— 同一目录只有在归一之后才是「同一个字符串」。
#[cfg(windows)]
pub fn normalize_canonical(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = raw.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    raw.to_string()
}

/// 非 Windows:路径本就无前缀概念,原样返回(便于契约层在任意宿主机上测)
#[cfg(not(windows))]
pub fn normalize_canonical(raw: &str) -> String {
    raw.to_string()
}

/// 构造 runner 的 argv(不含程序名本身)
pub fn runner_argv(spec: &RunnerSpec) -> Vec<String> {
    let mut argv = vec!["--mode".to_string(), spec.mode.as_str().to_string()];
    for root in &spec.writable {
        argv.push("--writable".to_string());
        argv.push(root.dir.display().to_string());
        argv.push(root.sid.clone());
    }
    argv.push("--".to_string());
    argv.push(spec.program.clone());
    argv.extend(spec.args.iter().cloned());
    argv
}

/// 解析 runner 的 argv(不含程序名本身)
pub fn parse_runner_args<I>(argv: I) -> Result<RunnerSpec, String>
where
    I: IntoIterator<Item = OsString>,
{
    let words: Vec<String> = argv
        .into_iter()
        .map(|w| w.to_string_lossy().into_owned())
        .collect();
    let Some(sep) = words.iter().position(|w| w == "--") else {
        return Err("missing `--` before the payload command".into());
    };
    let (flags, payload) = words.split_at(sep);
    let payload = &payload[1..];
    let Some(program) = payload.first() else {
        return Err("missing payload command after `--`".into());
    };

    let mut mode: Option<Mode> = None;
    let mut writable: Vec<WritableRoot> = Vec::new();
    let mut i = 0;
    while i < flags.len() {
        match flags[i].as_str() {
            "--mode" => {
                let word = flags
                    .get(i + 1)
                    .ok_or_else(|| "--mode requires a value".to_string())?;
                mode = Some(Mode::parse(word).ok_or_else(|| {
                    format!("unknown mode `{word}` (read-only | workspace-write | full-access)")
                })?);
                i += 2;
            }
            "--writable" => {
                let dir = flags
                    .get(i + 1)
                    .ok_or_else(|| "--writable requires <dir> <sid>".to_string())?;
                let sid = flags
                    .get(i + 2)
                    .ok_or_else(|| "--writable requires <dir> <sid>".to_string())?;
                writable.push(WritableRoot {
                    dir: PathBuf::from(dir),
                    sid: sid.clone(),
                });
                i += 3;
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }

    let mode = mode.ok_or_else(|| "missing --mode".to_string())?;
    if mode.requires_writable_roots() && writable.is_empty() {
        return Err(format!(
            "mode `{}` requires at least one --writable root",
            mode.as_str()
        ));
    }
    if !mode.requires_writable_roots() && !writable.is_empty() {
        return Err(format!(
            "mode `{}` must not carry --writable roots",
            mode.as_str()
        ));
    }

    Ok(RunnerSpec {
        mode,
        writable,
        program: program.clone(),
        args: payload[1..].to_vec(),
    })
}

/// 解析载荷程序为绝对路径。
///
/// `CreateProcess` 的 `lpApplicationName` **不走 PATH 搜索**(显式给的名字
/// 就是显式路径),而裸名交给命令行则只补 `.exe`、不认 PATHEXT。调用方通常
/// 已给出解析好的绝对路径(见 `liuma_sandbox::shell`),这里是兜底:裸名按
/// PATH × PATHEXT 展开,首个存在者胜出;解析不到就原样返回,让创建进程
/// 报出准确的失败码。
pub fn resolve_program(
    program: &str,
    path_dirs: &[PathBuf],
    pathext: &[String],
    exists: &dyn Fn(&PathBuf) -> bool,
) -> String {
    let as_path = PathBuf::from(program);
    if as_path.is_absolute() || program.contains('\\') || program.contains('/') {
        return program.to_string();
    }
    for dir in path_dirs {
        for ext in pathext {
            let candidate = dir.join(format!("{program}{ext}"));
            if exists(&candidate) {
                return candidate.display().to_string();
            }
        }
    }
    program.to_string()
}

/// `candidate` 是否位于 `root` 之内(含等于)。
///
/// 比较按「组件边界 + 大小写不敏感」:Windows 路径大小写不敏感,且
/// `/ws` 不该被认为是 `/ws2` 的前缀。
pub fn is_within(root: &str, candidate: &str) -> bool {
    let root = trimmed(root);
    let candidate = trimmed(candidate);
    if candidate == root {
        return true;
    }
    candidate
        .strip_prefix(&root)
        .is_some_and(|rest| rest.starts_with('\\') || rest.starts_with('/'))
}

/// 去掉尾部分隔符(保留「盘根」形态:`C:\` 不变成 `C:`)
fn trimmed(path: &str) -> String {
    let lowered = path.replace('/', "\\").to_ascii_lowercase();
    let trimmed = lowered.trim_end_matches('\\');
    if trimmed.ends_with(':') {
        // `C:` -> `C:\`(驱动器相对路径与盘根本身不是一回事,这里统一按盘根)
        format!("{trimmed}\\")
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    /// SID 派生:确定性、域分离、输入字节敏感
    #[test]
    fn sid_derivation_is_deterministic_and_byte_sensitive() {
        let ws = workspace_sid(r"D:\proj\liuma");
        assert_eq!(ws, workspace_sid(r"D:\proj\liuma"), "同一输入必须同值");
        assert!(ws.starts_with("S-1-4-"), "{ws}");

        // 大小写与分隔符是**不同的字符串**(归一在调用方,见 normalize_canonical)
        assert_ne!(ws, workspace_sid(r"d:\proj\liuma"));
        // 不同目录必得不同身份(否则一个工作区的 ACE 会放行另一个)
        assert_ne!(ws, workspace_sid(r"D:\proj\other"));
    }

    #[test]
    fn normalize_canonical_strips_prefixes() {
        #[cfg(windows)]
        {
            assert_eq!(normalize_canonical(r"\\?\D:\proj"), r"D:\proj");
            assert_eq!(
                normalize_canonical(r"\\?\UNC\server\share\dir"),
                r"\\server\share\dir"
            );
            assert_eq!(normalize_canonical(r"D:\proj"), r"D:\proj");
        }
        #[cfg(not(windows))]
        assert_eq!(normalize_canonical("/tmp/x"), "/tmp/x");
    }

    /// argv 往返:多可写根、含空格的路径、载荷参数原样
    #[test]
    fn runner_argv_round_trip() {
        let spec = RunnerSpec {
            mode: Mode::WorkspaceWrite,
            writable: vec![
                WritableRoot {
                    dir: PathBuf::from(r"C:\work dir"),
                    sid: "S-1-4-1-2".into(),
                },
                WritableRoot {
                    dir: PathBuf::from(r"C:\tmp"),
                    sid: "S-1-4-3-4-1".into(),
                },
            ],
            program: "pwsh".into(),
            args: vec!["-Command".into(), "echo hi".into()],
        };
        let argv = runner_argv(&spec);
        assert_eq!(
            parse_runner_args(argv.iter().map(OsString::from).collect::<Vec<_>>()).unwrap(),
            spec
        );
    }

    /// 载荷里的 `--` 不参与分隔(分隔取第一个,之后的都归载荷)
    #[test]
    fn only_the_first_separator_splits() {
        let spec = parse_runner_args(args(&[
            "--mode",
            "read-only",
            "--",
            "git",
            "log",
            "--",
            "oneline",
        ]))
        .unwrap();
        assert_eq!(spec.program, "git");
        assert_eq!(spec.args, ["log", "--", "oneline"]);
    }

    #[test]
    fn mode_consistency_is_enforced() {
        // 可写模式必须带根
        let err = parse_runner_args(args(&["--mode", "workspace-write", "--", "ls"])).unwrap_err();
        assert!(err.contains("at least one --writable"), "{err}");
        // 不可写模式不许带根
        let err = parse_runner_args(args(&[
            "--mode",
            "read-only",
            "--writable",
            r"C:\ws",
            "S-1-4-1-2",
            "--",
            "ls",
        ]))
        .unwrap_err();
        assert!(err.contains("must not carry"), "{err}");
        // 全盘放行同样不许带根(放行由 mode 表达)
        assert!(
            parse_runner_args(args(&[
                "--mode",
                "full-access",
                "--writable",
                r"C:\ws",
                "S-1-4-1-2",
                "--",
                "ls"
            ]))
            .is_err()
        );
    }

    #[test]
    fn malformed_argv_is_rejected() {
        assert!(
            parse_runner_args(args(&["--mode", "read-only"])).is_err(),
            "缺 --"
        );
        assert!(
            parse_runner_args(args(&["--mode", "read-only", "--"])).is_err(),
            "缺载荷"
        );
        assert!(parse_runner_args(args(&["--", "ls"])).is_err(), "缺 mode");
        assert!(
            parse_runner_args(args(&["--mode", "danger", "--", "ls"])).is_err(),
            "未知 mode"
        );
        assert!(
            parse_runner_args(args(&[
                "--workspace",
                "x",
                "--mode",
                "read-only",
                "--",
                "ls"
            ]))
            .is_err(),
            "未知参数"
        );
        assert!(
            parse_runner_args(args(&[
                "--mode",
                "workspace-write",
                "--writable",
                r"C:\ws",
                "--",
                "ls"
            ]))
            .is_err(),
            "writable 缺 SID"
        );
    }

    /// 程序解析:显式路径原样、裸名按 PATH × PATHEXT、找不到就原样交回
    #[test]
    fn resolve_program_covers_path_forms() {
        let dirs = vec![PathBuf::from(r"C:\tools")];
        let exts = vec![".EXE".to_string(), ".CMD".to_string()];
        let exists = |p: &PathBuf| {
            p.to_string_lossy()
                .eq_ignore_ascii_case(r"C:\tools\thing.CMD")
        };
        assert_eq!(
            resolve_program("thing", &dirs, &exts, &exists),
            r"C:\tools\thing.CMD"
        );
        // 显式路径与带分隔符的路径不查 PATH
        assert_eq!(
            resolve_program(r"D:\opt\thing.exe", &dirs, &exts, &exists),
            r"D:\opt\thing.exe"
        );
        assert_eq!(resolve_program("./thing", &dirs, &exts, &exists), "./thing");
        // 解析不到时原样交回(失败信息由创建进程给出)
        assert_eq!(resolve_program("missing", &dirs, &exts, &exists), "missing");
    }

    /// 边界判定:组件边界 + 大小写不敏感 + 尾分隔符无关
    #[test]
    fn is_within_respects_component_boundaries() {
        assert!(is_within(r"C:\ws", r"C:\ws"));
        assert!(is_within(r"C:\ws", r"C:\ws\sub\file.txt"));
        assert!(is_within(r"C:\ws\", r"C:\ws\sub"), "尾分隔符无碍");
        assert!(is_within(r"C:\ws", r"c:\WS\sub"), "大小写不敏感");
        assert!(!is_within(r"C:\ws", r"C:\ws2\file"), "同前缀不同目录");
        assert!(!is_within(r"C:\ws", r"C:\other"));
    }
}
