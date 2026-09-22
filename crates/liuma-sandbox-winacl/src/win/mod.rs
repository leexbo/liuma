//! Win32 实现与 runner 编排。仅 Windows 编译。
//!
//! 顺序即 fail-closed 时序,任何一步失败都在**创建进程之前**返回:
//! 参数一致性 → 可写根 canonical 与 SID 对账 → 建令牌 → 修默认 DACL →
//! 逐根授权 → 建 Job → 受限创建载荷 → 镜像退出码。
//!
//! 「代理」边界说明:runner 从**自身**令牌派生受限令牌,而自身令牌来自
//! 调用它的进程。受限令牌再受限是求交,故受限载荷即便自己调用 runner 也
//! 无法放宽权限;真正的信任边界是「谁能以不受限身份启动 runner」,与
//! 「谁能直接起进程」等价,没有新增面。可写根仍逐条对账,防止的是**误用**。

mod acl;
mod error;
mod job;
mod process;
mod sid;
mod token;

use std::ffi::OsString;
use std::path::PathBuf;

use crate::contract::{
    EXIT_RUNNER_FAILURE, GRANT_MASK, Mode, RunnerSpec, normalize_canonical, parse_runner_args,
    resolve_program, workspace_sid,
};
use job::Job;
use sid::LocalSid;
use token::Token;

/// runner 入口:返回进程退出码(失败恒为 [`EXIT_RUNNER_FAILURE`])
pub fn run(argv: impl Iterator<Item = OsString>) -> i32 {
    match run_inner(argv) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("{}{message}", crate::contract::FAIL_PREFIX);
            EXIT_RUNNER_FAILURE
        }
    }
}

fn run_inner(argv: impl Iterator<Item = OsString>) -> Result<i32, String> {
    let spec = parse_runner_args(argv)?;
    // 全盘放行不下发写限制,但仍入 Job(进程树终止不能丢)
    if spec.mode == Mode::FullAccess {
        return spawn_unrestricted(&spec);
    }
    let prepared = prepare(&spec)?;
    let program = resolve_program(&spec.program, &path_dirs(), &pathext_entries(), &|p| {
        p.exists()
    });
    let exit = process::spawn_in_job(&prepared.token, &prepared.job, &program, &spec.args, None)
        .map_err(|e| e.render())?;
    Ok(exit)
}

/// 校验并完成授权,产出一枚可用的受限令牌与 Job
struct Prepared {
    token: Token,
    job: Job,
}

fn prepare(spec: &RunnerSpec) -> Result<Prepared, String> {
    // ① 可写根:canonical 归一 + 与调用方算出的 SID 对账
    let mut roots: Vec<(PathBuf, LocalSid)> = Vec::new();
    for root in &spec.writable {
        let canonical = canonicalize(&root.dir)?;
        // 一个工作区一个身份:所有可写根同域派生(工作区根与暂存区根同为
        // 本会话的可写集,不构成两种身份)
        let expected = workspace_sid(&canonical);
        if root.sid != expected {
            return Err(format!(
                "capability SID mismatch for {}: caller sent {}, runner derived {} \
                 (path canonicalization differs)",
                root.dir.display(),
                root.sid,
                expected
            ));
        }
        let sid = LocalSid::from_string(&root.sid).map_err(|e| e.render())?;
        sid.validate().map_err(|e| e.render())?;
        roots.push((PathBuf::from(&canonical), sid));
    }
    // ② 令牌:登录 SID + Everyone 恒在(保活组,缺则 DLL 初始化失败)
    let current = Token::open_current().map_err(|e| e.render())?;
    let logon = sid::logon_sid(current.raw()).map_err(|e| e.render())?;
    let everyone = LocalSid::everyone().map_err(|e| e.render())?;
    let mut restricting: Vec<&LocalSid> = vec![&logon, &everyone];
    for (_, sid) in &roots {
        restricting.push(sid);
    }
    let token = current
        .create_restricted(&restricting)
        .map_err(|e| e.render())?;

    // ④ 默认 DACL:不补这条,受限孙进程建管道会一律失败
    //    trustee 取最窄的可用身份(优先暂存区 SID,其次任一写 SID)
    let dacl_sid = roots.last().map_or(&everyone, |(_, sid)| sid);
    token.grant_default_dacl(dacl_sid).map_err(|e| e.render())?;

    // ⑤ 逐根授权(精确 ACE 命中即跳过,避免重复的全树传播)
    for (dir, sid) in &roots {
        match acl::exact_ace_present(dir, sid, GRANT_MASK) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(e) => return Err(e.render()),
        }
        acl::grant_write(dir, sid, GRANT_MASK).map_err(|e| e.render())?;
    }

    // ⑥ Job 先于进程存在(创建后先挂载再恢复运行)
    let job = Job::kill_on_close().map_err(|e| e.render())?;
    Ok(Prepared { token, job })
}

/// 全盘放行:不建受限令牌、不给 ACE,但**仍然入 Job**
fn spawn_unrestricted(spec: &RunnerSpec) -> Result<i32, String> {
    let job = Job::kill_on_close().map_err(|e| e.render())?;
    // 用自身令牌(即以调用者身份)创建,只是挂在 Job 里
    let token = Token::open_current().map_err(|e| e.render())?;
    process::spawn_in_job(&token, &job, &spec.program, &spec.args, None).map_err(|e| e.render())
}

/// PATH 目录列表(Windows 分隔符)
fn path_dirs() -> Vec<PathBuf> {
    std::env::var("PATH")
        .map(|v| {
            v.split(';')
                .map(str::trim)
                .map(|s| s.trim_matches('"'))
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

/// PATHEXT 扩展名表(缺省按 Windows 约定)
fn pathext_entries() -> Vec<String> {
    const DEFAULT: &[&str] = &[".COM", ".EXE", ".BAT", ".CMD"];
    std::env::var("PATHEXT")
        .map(|v| {
            v.split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_ascii_uppercase)
                .collect::<Vec<_>>()
        })
        .ok()
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| DEFAULT.iter().map(|s| (*s).to_string()).collect())
}

/// canonical 归一:去 `\\?\` 前缀后再规范,保证 SID 派生的输入是同一个字符串
fn canonicalize(path: &PathBuf) -> Result<String, String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|e| format!("cannot resolve {}: {e}", path.display()))?;
    let normalized = normalize_canonical(&canonical.display().to_string());
    let as_path = PathBuf::from(&normalized);
    if !as_path.is_dir() {
        return Err(format!("{} is not a directory", as_path.display()));
    }
    Ok(normalized)
}
