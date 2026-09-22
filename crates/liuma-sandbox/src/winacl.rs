//! Windows 沙箱 rung 的宿主侧:解析 runner、组装包装 argv、功能探测。
//!
//! 与 bwrap/seatbelt 同构——沙箱是 argv 包装(`runner --mode … -- <载荷>`),
//! 具体实现见 `liuma-sandbox-winacl`。这里只管「宿主这一侧」的三件事:
//! runner 在哪、本轮的可写根怎么变成能力 SID、这台机器到底能不能用。

use std::path::{Path, PathBuf};

use crate::sandbox::{
    Confined, ProbeResult, Rung, SandboxEnforcement, SandboxError, SandboxPolicy,
};

/// 显式指定 runner 路径的覆盖点(打包布局与测试用)
const RUNNER_ENV: &str = "LIUMA_SANDBOX_RUNNER";

/// runner 可执行名
#[cfg(windows)]
const RUNNER_BIN: &str = "liuma-sandbox-run.exe";
#[cfg(not(windows))]
const RUNNER_BIN: &str = "liuma-sandbox-run";

/// 解析 runner 可执行文件。
///
/// 位置按顺序:显式覆盖 → 与当前进程同目录(装机/同目录部署)→ 上一级
/// (cargo 把测试二进制放在 `target/debug/deps/`,runner 在 `target/debug/`)
/// → PATH。解析不到即 [`None`]:调用方据此 fail-closed,绝不降级为不受限。
pub fn resolve_runner() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(explicit) = std::env::var_os(RUNNER_ENV) {
        candidates.push(PathBuf::from(explicit));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join(RUNNER_BIN));
        if let Some(up) = dir.parent() {
            candidates.push(up.join(RUNNER_BIN));
        }
    }
    if let Some(found) = which_on_path(RUNNER_BIN) {
        candidates.push(found);
    }
    candidates.into_iter().find(|c| c.is_file())
}

/// 在 PATH 上找一个文件(裸名匹配,不做 PATHEXT 展开——runner 带全名)
#[cfg(windows)]
fn which_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").ok()?;
    path.split(';')
        .map(str::trim)
        .map(|s| s.trim_matches('"'))
        .filter(|s| !s.is_empty())
        .map(|dir| Path::new(dir).join(name))
        .find(|c| c.is_file())
}

/// 非 Windows:PATH 用冒号分隔(该 rung 在那里本就不可用,保持对称)
#[cfg(not(windows))]
fn which_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").ok()?;
    path.split(':')
        .filter(|s| !s.is_empty())
        .map(|dir| Path::new(dir).join(name))
        .find(|c| c.is_file())
}

/// 把策略 + 载荷命令包装成 runner 调用。
///
/// 可写根逐个 canonical 归一后派生能力 SID:runner 会用自己 canonicalize 的
/// 结果重算一遍并对账,两边不一致即拒绝执行——这是同一目录因 `\\?\` 前缀 /
/// 大小写 / 8.3 短名铸出两个身份的唯一防线。
pub fn wrap(policy: &SandboxPolicy, cmd: &str, args: &[String]) -> Result<Confined, SandboxError> {
    let runner = resolve_runner().ok_or_else(|| {
        SandboxError::Other(format!(
            "windows sandbox runner not found; build it with \
             `cargo build -p liuma-sandbox-winacl --bin {RUNNER_BIN}` \
             or point {RUNNER_ENV} at it"
        ))
    })?;

    let mode = match policy.mode {
        crate::sandbox::SandboxMode::ReadOnly => liuma_sandbox_winacl::Mode::ReadOnly,
        crate::sandbox::SandboxMode::WorkspaceWrite => liuma_sandbox_winacl::Mode::WorkspaceWrite,
        crate::sandbox::SandboxMode::FullAccess => liuma_sandbox_winacl::Mode::FullAccess,
    };

    let mut writable = Vec::new();
    if mode == liuma_sandbox_winacl::Mode::WorkspaceWrite {
        for root in policy.writable_roots() {
            let canonical = canonical_string(&root)?;
            writable.push(liuma_sandbox_winacl::WritableRoot {
                sid: liuma_sandbox_winacl::workspace_sid(&canonical),
                dir: PathBuf::from(canonical),
            });
        }
        if writable.is_empty() {
            // 可写模式却一个根都没有:与其让 runner 报参数不一致,不如在这里说清
            return Err(SandboxError::Other(
                "workspace-write 策略解析不出任何可写根(工作区与暂存区都不可用)".into(),
            ));
        }
    }

    let argv = liuma_sandbox_winacl::runner_argv(&liuma_sandbox_winacl::RunnerSpec {
        mode,
        writable,
        program: cmd.to_string(),
        args: args.to_vec(),
    });
    let rung = Rung::WindowsAcl(runner);
    Ok(Confined {
        program: match &rung {
            Rung::WindowsAcl(path) => path.to_string_lossy().into_owned(),
            _ => unreachable!("刚构造的 rung"),
        },
        argv,
        enforcement: SandboxEnforcement::Partial,
        denial_dialect: crate::sandbox::dialect_of(&rung),
        runner_failure_rules: crate::sandbox::runner_failure_of(&rung),
    })
}

/// canonical 归一(去 `\\?\` 前缀)——与 runner 侧同一个函数,保证对账成立
fn canonical_string(path: &Path) -> Result<String, SandboxError> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|e| SandboxError::Other(format!("cannot resolve {}: {e}", path.display())))?;
    Ok(liuma_sandbox_winacl::normalize_canonical(
        &canonical.display().to_string(),
    ))
}

/// 功能式探测:真跑一次只读沙箱。
///
/// 载荷取**平台 shell**而非 `cmd /c exit 0`:实测同一受限令牌下 `cmd` 可用
/// 而 PowerShell 可能整片不可用,用 cmd 探会给出假阳性。探测失败即无可用
/// rung,调用方按 fail-closed 拒绝执行。
pub fn probe() -> Option<ProbeResult> {
    let runner = resolve_runner()?;
    let program = crate::shell::shell_program()?;
    let (program, args) = crate::shell::command_argv(crate::shell::dialect(), program, "exit 0");
    let argv = liuma_sandbox_winacl::runner_argv(&liuma_sandbox_winacl::RunnerSpec {
        mode: liuma_sandbox_winacl::Mode::ReadOnly,
        writable: Vec::new(),
        program,
        args,
    });

    let mut command = std::process::Command::new(&runner);
    command
        .args(&argv)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let status = command.status().ok()?;
    status.success().then_some(ProbeResult {
        rung: Rung::WindowsAcl(runner),
        // 写与删除受 ACL 约束,**读、网络与进程可见性不受限** —— 如实声明
        enforcement: SandboxEnforcement::Partial,
    })
}
