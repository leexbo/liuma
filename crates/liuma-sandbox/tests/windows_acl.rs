//! Windows rung 的宿主侧出口门:经 [`spawn`] 真跑沙箱命令。
//!
//! 与 `liuma-sandbox-winacl` 自己的用例相比,这里多走一层:`probe` → 选 rung
//! → `wrap_argv` → 命令串经平台 shell → `wait_classified` 的**拒绝分类**。
//! 分类是通往模型的那条链(拒绝标记 + 升级提示),所以断言落在 `ExitClass`
//! 上,而不只是退出码。

#![cfg(windows)]

use std::path::{Path, PathBuf};

use liuma_sandbox::sandbox::SandboxPolicy;
use liuma_sandbox::shell;
use liuma_sandbox::{ExitClass, SpawnOptions, spawn};

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("liuma-wacl-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建夹具目录");
    dir
}

/// 经平台 shell 跑一条命令,返回分类与 stdout
async fn run(ws: &Path, command: &str) -> (ExitClass, String) {
    let (program, args) = shell::shell_argv(command).expect("本机应有可用 shell");
    let opts = SpawnOptions {
        cwd: Some(ws.to_path_buf()),
        env: Default::default(),
        sandbox: Some(SandboxPolicy::workspace_write(ws)),
        stdin: None,
    };
    let mut child = spawn(&program, &args, &opts).await.expect("沙箱 spawn");
    let out = child.stdout().await.unwrap_or_default();
    let class = child.wait_classified().await;
    (class, String::from_utf8_lossy(&out).into_owned())
}

/// 可写根内的写执行成功,输出回到宿主
#[tokio::test]
async fn sandboxed_command_runs_and_returns_output() {
    let ws = workspace("run");
    let (class, out) = run(&ws, "echo sandbox-ok").await;
    assert!(
        matches!(class, ExitClass::Ran(status) if status.success()),
        "命令应在沙箱内正常执行;got: {class:?}"
    );
    assert!(out.contains("sandbox-ok"), "输出应回到宿主;got: {out:?}");
    let _ = std::fs::remove_dir_all(&ws);
}

/// 非 ASCII 输出往返:受限令牌下 shell 进 ConstrainedLanguage,把输出编码
/// 改成 UTF-8 的那句 .NET 属性设置被语言模式挡掉,字节落在系统代码页里
/// (实测 zh-CN 为 936)——宿主这一侧必须按平台代码页兜底解码,不能糊成乱码
#[tokio::test]
async fn non_ascii_output_round_trips() {
    let ws = workspace("cjk");
    let (class, out) = run(&ws, "Write-Output '中文测试-OK'").await;
    assert!(
        matches!(class, ExitClass::Ran(status) if status.success()),
        "命令应正常执行;got: {class:?}"
    );
    assert!(
        out.contains("中文测试-OK"),
        "中文输出应原样回传(宿主按系统代码页解码);got: {out:?}"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

/// 工作区内写成功(证明包 shell + 授权链路是通的,不是「什么都没跑」)
#[tokio::test]
async fn write_inside_workspace_succeeds() {
    let ws = workspace("inside");
    let (_, _) = run(&ws, "Set-Content -Path inside.txt -Value ok").await;
    assert!(ws.join("inside.txt").is_file(), "工作区内应写出文件");
    let _ = std::fs::remove_dir_all(&ws);
}

/// 工作区外写被内核拦,且**分类为拒绝**(不是「常规失败」)
#[tokio::test]
async fn write_outside_workspace_is_classified_as_denied() {
    let ws = workspace("outside");
    // 「区外」要挑一个**既不在工作区、也不在暂存区**的位置(可写根 =
    // 工作区 + %TEMP%),所以不能放在 temp_dir 下 —— 拿用户主目录
    // (用户可写,但不是任何可写根)
    let outside_dir = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| ws.parent().expect("工作区必有父目录").to_path_buf())
        .join(format!("liuma-wacl-elsewhere-{}", std::process::id()));
    std::fs::create_dir_all(&outside_dir).expect("建区外夹具目录");
    let outside = outside_dir.join("denied.txt");
    let _ = std::fs::remove_file(&outside);

    let (class, _) = run(
        &ws,
        &format!("Set-Content -Path '{}' -Value nope", outside.display()),
    )
    .await;
    assert!(!outside.exists(), "工作区外写必须被拒;分类: {class:?}");
    assert!(
        matches!(class, ExitClass::Denied { .. }),
        "拒绝应被识别为 Denied(方言表匹配),而不是常规退出;got: {class:?}"
    );
    let _ = std::fs::remove_dir_all(&ws);
    let _ = std::fs::remove_dir_all(&outside_dir);
}
