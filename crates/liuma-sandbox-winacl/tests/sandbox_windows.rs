//! Windows 沙箱真机出口门:受限令牌 + 能力 SID 授权 + Job 的实际效果。
//!
//! 直接调库里的 [`liuma_sandbox_winacl::run`](它就是 runner 的入口,返回
//! 退出码),不起子进程,断言落在**文件系统副作用**上而不只是退出码。
//!
//! 载荷统一用 Windows PowerShell 5.1 的**真实路径**:Store 安装的 pwsh
//! 只提供应用执行别名,而别名是 reparse point,`CreateProcessAsUser` 打不开
//! (`ERROR_CANT_ACCESS_FILE`),其真实目标又落在 ACL 收紧的 WindowsApps 下。

#![cfg(windows)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use liuma_sandbox_winacl::{EXIT_RUNNER_FAILURE, Mode, workspace_sid};

/// Windows PowerShell 5.1(系统自带、普通 ACL,受限令牌可用)
const POWERSHELL: &str = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";

fn temp_workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("liuma-sbx-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建夹具目录");
    dir
}

fn canonical(path: &Path) -> String {
    let raw = std::fs::canonicalize(path).expect("夹具路径可解析");
    liuma_sandbox_winacl::normalize_canonical(&raw.display().to_string())
}

/// 组 runner 的 argv(不带程序名)
fn argv(mode: &str, writable: &[(String, String)], script: &str) -> Vec<OsString> {
    let mut words = vec!["--mode".to_string(), mode.to_string()];
    for (dir, sid) in writable {
        words.push("--writable".to_string());
        words.push(dir.clone());
        words.push(sid.clone());
    }
    words.push("--".to_string());
    words.push(POWERSHELL.to_string());
    words.push("-NoProfile".to_string());
    words.push("-NonInteractive".to_string());
    words.push("-Command".to_string());
    words.push(script.to_string());
    words.into_iter().map(OsString::from).collect()
}

/// 写文件的 PowerShell 片段
fn write_script(path: &Path) -> String {
    format!("Set-Content -Path '{}' -Value ok", path.display())
}

/// 可写根内写成功、根外写被拒**且事后文件不存在**
#[test]
fn workspace_write_allows_inside_and_denies_outside() {
    let ws = temp_workspace("ws");
    let inside = ws.join("inside.txt");
    let outside =
        std::env::temp_dir().join(format!("liuma-sbx-outside-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&outside);
    let sid = workspace_sid(&canonical(&ws));
    let root = canonical(&ws);

    let code = liuma_sandbox_winacl::run(
        argv(
            "workspace-write",
            &[(root.clone(), sid.clone())],
            &write_script(&inside),
        )
        .into_iter(),
    );
    assert_eq!(code, 0, "区内写应成功");
    assert!(inside.exists(), "区内的文件应被创建");

    let code = liuma_sandbox_winacl::run(
        argv("workspace-write", &[(root, sid)], &write_script(&outside)).into_iter(),
    );
    // PowerShell 的 cmdlet 报错不必然改退出码,故断言落在副作用上
    assert!(
        !outside.exists(),
        "区外写必须被拒(退出码 {code});文件不得被创建"
    );

    let _ = std::fs::remove_dir_all(&ws);
    let _ = std::fs::remove_file(&outside);
}

/// 只读:连可写根也不给,一切写被拒
#[test]
fn read_only_denies_writes() {
    let ws = temp_workspace("ro");
    let target = ws.join("nope.txt");
    let code =
        liuma_sandbox_winacl::run(argv("read-only", &[], &write_script(&target)).into_iter());
    assert!(!target.exists(), "只读下不得写出文件(退出码 {code})");
    let _ = std::fs::remove_dir_all(&ws);
}

/// 可写根与调用方算出的 SID 不一致即拒绝执行(路径 canonical 漂移的防线)
#[test]
fn capability_sid_mismatch_refuses_to_run() {
    let ws = temp_workspace("sid");
    let target = ws.join("must-not-exist.txt");
    let code = liuma_sandbox_winacl::run(
        argv(
            "workspace-write",
            &[(canonical(&ws), "S-1-4-1-1".to_string())],
            &write_script(&target),
        )
        .into_iter(),
    );
    assert_eq!(code, EXIT_RUNNER_FAILURE, "SID 不一致必须 fail-closed");
    assert!(!target.exists(), "拒绝执行时不得留下任何副作用");
    let _ = std::fs::remove_dir_all(&ws);
}

/// 载荷能读系统目录(受限令牌若对读也求交,连系统 DLL 都加载不了)
#[test]
fn sandboxed_payload_can_read_system_files() {
    let ws = temp_workspace("read");
    let code = liuma_sandbox_winacl::run(
        argv(
            "read-only",
            &[],
            "if (Test-Path $env:SystemRoot) { exit 0 } else { exit 3 }",
        )
        .into_iter(),
    );
    assert_eq!(code, 0, "受限载荷应能读系统环境与目录");
    let _ = std::fs::remove_dir_all(&ws);
}

/// 模式与可写根的数量必须匹配(全盘放行不带根)
#[test]
fn full_access_rejects_writable_roots() {
    let ws = temp_workspace("fa");
    let code = liuma_sandbox_winacl::run(
        argv(
            "full-access",
            &[(canonical(&ws), workspace_sid(&canonical(&ws)))],
            "exit 0",
        )
        .into_iter(),
    );
    assert_eq!(code, EXIT_RUNNER_FAILURE);
    let _ = std::fs::remove_dir_all(&ws);
}

/// `Mode` 的 argv 词汇与解析互为逆(opaque 契约的锚)
#[test]
fn mode_words_round_trip() {
    for mode in [Mode::ReadOnly, Mode::WorkspaceWrite, Mode::FullAccess] {
        assert_eq!(Mode::parse(mode.as_str()), Some(mode));
    }
    assert_eq!(Mode::parse("danger"), None);
}
