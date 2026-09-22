//! 平台 shell:工具名、命令串的 argv 形态与解释器解析的单点收口。
//!
//! 为什么需要这层:命令串是**一条字符串**,交给 shell 做二次解析——而
//! `bash -c` 与 `pwsh -Command` 的引号、分隔符、变量展开规则完全不同。
//! 调用方(前台 / 后台 / PTY / hooks)不该各自知道这件事,模型面的工具名
//! 也要与实际方言一致,否则模型会按 `bash` 的语义写出 Windows 上跑不通的
//! 命令。
//!
//! fail-closed:解析不到解释器即拒绝执行,不退化为 `cmd /c`——那等于把
//! 命令丢进一个从未被验证过的方言里跑。

use std::path::{Path, PathBuf};

/// shell 解析失败
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ShellError {
    /// 本机找不到可用的 shell(命令不执行;不退化为 cmd)
    #[error("no usable shell found; refusing to run the command (fail-closed)")]
    NoShell,
}

/// shell 方言(命令行的解析规则;由平台决定,不由解释器路径推断)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// POSIX shell(`bash -c <command>`)
    Posix,
    /// PowerShell(`pwsh -Command <command>`)
    PowerShell,
}

/// 模型面的工具名(随方言走:模型看到的工具名与它要写的语法一致)
pub fn tool_name() -> &'static str {
    match dialect() {
        Dialect::Posix => "bash",
        Dialect::PowerShell => "pwsh",
    }
}

/// 工具描述(schema 文案;两套方言各一套)
pub fn tool_description() -> &'static str {
    match dialect() {
        Dialect::Posix => "Run a shell command in the sandboxed working directory.",
        Dialect::PowerShell => {
            "Run a PowerShell command in the sandboxed working directory. \
             The command string is passed to `pwsh -Command` verbatim."
        }
    }
}

/// 命令参数描述
pub fn command_param_description() -> &'static str {
    match dialect() {
        Dialect::Posix => "The bash command to execute.",
        Dialect::PowerShell => "The PowerShell command to execute.",
    }
}

/// 本平台方言
pub fn dialect() -> Dialect {
    #[cfg(windows)]
    {
        Dialect::PowerShell
    }
    #[cfg(not(windows))]
    {
        Dialect::Posix
    }
}

/// pwsh 输出编码前导。
///
/// Windows PowerShell 5.1 默认按 OEM 代码页写 stdout,宿主按 UTF-8 解码会
/// 糊掉非 ASCII;与命令**接在同一行**(`;` 连接)才不会打乱报错行号。
const PWSH_PREAMBLE: &str = "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);$OutputEncoding=[Console]::OutputEncoding";

/// 命令串 → (program, args)。命令串整体作为**单个** argv 元素交给解释器,
/// 二次解析只发生在解释器内部——调用方不做引号转义(两套方言的转义规则
/// 不同,任何"帮忙"转义都会在另一套方言上出错)。
pub fn command_argv(dialect: Dialect, program: &Path, command: &str) -> (String, Vec<String>) {
    let program = program.display().to_string();
    match dialect {
        Dialect::Posix => (program, vec!["-c".to_string(), command.to_string()]),
        Dialect::PowerShell => (
            program,
            vec![
                "-NoLogo".to_string(),
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                format!("{PWSH_PREAMBLE}; {command}"),
            ],
        ),
    }
}

/// 本平台命令串 → (program, args);解释器缺席即拒绝执行
pub fn shell_argv(command: &str) -> Result<(String, Vec<String>), ShellError> {
    let program = shell_program().ok_or(ShellError::NoShell)?;
    Ok(command_argv(dialect(), program, command))
}

/// 本平台解释器(解析一次即缓存;仅缓存成功,便于装好 shell 后不必重启)
pub fn shell_program() -> Option<&'static Path> {
    static RESOLVED: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    if let Some(p) = RESOLVED.get() {
        return Some(p);
    }
    let program = resolve_shell(&exists)?;
    let _ = RESOLVED.set(program);
    RESOLVED.get().map(PathBuf::as_path)
}

/// 按本平台候选链选解释器
pub fn resolve_shell(exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let lookup = |k: &str| std::env::var(k).ok();
        let dirs = path_dirs_from(&std::env::var("PATH").unwrap_or_default());
        first_existing(&windows_candidates(&lookup, &dirs), exists)
    }
    #[cfg(not(windows))]
    {
        first_existing(&posix_candidates(), exists)
    }
}

/// POSIX 解释器:固定绝对路径,不查 PATH
/// (shell 是所有命令的解释器,PATH 解析等于把整条链路交给可写目录)。
pub fn posix_candidates() -> Vec<PathBuf> {
    vec![PathBuf::from("/bin/bash")]
}

/// Windows 解释器候选(优先级序):PowerShell 7 安装位 → PATH → Windows
/// PowerShell 5.1。不依赖宿主平台,便于在 Linux/macOS 上验证这条链。
///
/// 每个 PATH 目录试两个名字:`pwsh.exe`(常规安装)与 `pwsh`(Microsoft
/// Store 安装只提供不带扩展名的应用执行别名,`pwsh.exe` 在那里不存在)。
pub fn windows_candidates(
    lookup: &dyn Fn(&str) -> Option<String>,
    path_dirs: &[PathBuf],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(pf) = lookup("ProgramFiles") {
        out.push(Path::new(&pf).join("PowerShell").join("7").join("pwsh.exe"));
    }
    for dir in path_dirs {
        out.push(dir.join("pwsh.exe"));
        out.push(dir.join("pwsh"));
    }
    if let Some(sr) = lookup("SystemRoot") {
        out.push(
            Path::new(&sr)
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe"),
        );
    }
    out
}

/// PATH 字符串 → 目录列表(平台分隔符)
pub fn path_dirs_from(path_var: &str) -> Vec<PathBuf> {
    #[cfg(windows)]
    let sep = ';';
    #[cfg(not(windows))]
    let sep = ':';
    split_path_dirs(path_var, sep)
}

/// 指定分隔符的 PATH 拆分(纯函数:两种分隔符在任何宿主上都可测)
pub fn split_path_dirs(path_var: &str, sep: char) -> Vec<PathBuf> {
    path_var
        .split(sep)
        .map(str::trim)
        // 条目可能带引号(`setx` 风格写入的 PATH 常见)
        .map(|s| s.trim_matches('"'))
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn first_existing(candidates: &[PathBuf], exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    candidates.iter().find(|c| exists(c)).cloned()
}

/// 候选是否存在。
///
/// 用 `symlink_metadata`(lstat 语义)而非 `is_file`:Microsoft Store 的
/// 应用执行别名是 reparse point,`is_file` 会因目标不可达而误判缺席,而
/// `CreateProcess` 对别名本身是可行的。
fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_command_argv_is_bash_c() {
        let (program, args) = command_argv(Dialect::Posix, Path::new("/bin/bash"), "ls -l");
        assert_eq!(program, "/bin/bash");
        assert_eq!(args, ["-c", "ls -l"]);
    }

    /// 命令串整体落在单个 argv 元素里(不拆词、不转义),且编码前导与命令
    /// 同一行——换行会让 pwsh 的报错行号与命令自身行号错位
    #[test]
    fn powershell_command_argv_is_single_command_arg() {
        let (program, args) = command_argv(
            Dialect::PowerShell,
            Path::new(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            "Get-ChildItem | Where-Object { $_.Length -gt 1 }",
        );
        assert_eq!(program, r"C:\Program Files\PowerShell\7\pwsh.exe");
        assert_eq!(
            &args[..4],
            ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"]
        );
        assert_eq!(args.len(), 5);
        let command = &args[4];
        assert!(command.ends_with("; Get-ChildItem | Where-Object { $_.Length -gt 1 }"));
        assert!(command.starts_with("[Console]::OutputEncoding="));
        assert_eq!(command.lines().count(), 1, "前导必须与命令同行");
    }

    /// Windows 候选链的优先级:PowerShell 7 安装位 → PATH → 5.1 兜底
    #[test]
    fn windows_candidates_order() {
        let lookup = |k: &str| match k {
            "ProgramFiles" => Some(r"C:\Program Files".to_string()),
            "SystemRoot" => Some(r"C:\Windows".to_string()),
            _ => None,
        };
        let dirs = vec![PathBuf::from(r"C:\tools")];
        let candidates = windows_candidates(&lookup, &dirs);
        assert_eq!(
            candidates,
            vec![
                PathBuf::from(r"C:\Program Files\PowerShell\7\pwsh.exe"),
                PathBuf::from(r"C:\tools\pwsh.exe"),
                PathBuf::from(r"C:\tools\pwsh"),
                PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
            ]
        );
    }

    #[test]
    fn resolve_shell_takes_first_existing_candidate() {
        let candidates = vec![
            PathBuf::from("/a/pwsh"),
            PathBuf::from("/b/pwsh"),
            PathBuf::from("/c/pwsh"),
        ];
        let exists = |p: &Path| p == Path::new("/b/pwsh") || p == Path::new("/c/pwsh");
        assert_eq!(
            first_existing(&candidates, &exists),
            Some(PathBuf::from("/b/pwsh"))
        );
        assert_eq!(first_existing(&candidates, &|_: &Path| false), None);
    }

    /// PATH 拆分的两个现实细节:条目带引号(`setx` 写入风格)、空条目
    #[test]
    fn split_path_dirs_handles_quotes_and_empties() {
        assert_eq!(
            split_path_dirs(r#""C:\Program Files";C:\tools;;"C:\bin\""#, ';'),
            vec![
                PathBuf::from(r"C:\Program Files"),
                PathBuf::from(r"C:\tools"),
                PathBuf::from(r"C:\bin"),
            ]
        );
        assert_eq!(
            split_path_dirs("/usr/bin:/bin:", ':'),
            vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")]
        );
    }

    #[test]
    fn dialect_matches_host() {
        #[cfg(windows)]
        assert_eq!(dialect(), Dialect::PowerShell);
        #[cfg(not(windows))]
        assert_eq!(dialect(), Dialect::Posix);
    }
}
