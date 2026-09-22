//! shell 缝的出口门:本平台解释器能被解析出来,并且真的执行一条命令。
//!
//! 这条链断在哪都不好查——解析失败会让每条命令都报「shell unavailable」,
//! 而 argv 形态错了只在特定方言下才现形,故用一条两套方言都合法的命令
//! (`echo`)做端到端断言。载荷不经沙箱(沙箱链由 process.rs 的用例覆盖),
//! 这里只验缝本身。

/// 解析 → 组 argv → 执行 → 读到输出
#[test]
fn platform_shell_runs_a_command() {
    let dialect = liuma_sandbox::shell::dialect();
    let program = liuma_sandbox::shell::shell_program()
        .unwrap_or_else(|| panic!("本机应能解析到 shell({:?})", dialect));
    let (program, args) = liuma_sandbox::shell::command_argv(dialect, program, "echo seam-ok");
    let out = std::process::Command::new(&program)
        .args(&args)
        .output()
        .unwrap_or_else(|e| panic!("{program} 应可启动: {e}"));
    assert!(out.status.success(), "退出应成功: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("seam-ok"),
        "shell 应执行命令串并回显;got: {stdout:?}"
    );
}

/// 工具名与方言一致(模型面名字是模型要写的语法)
#[test]
fn tool_name_matches_dialect() {
    let expected = match liuma_sandbox::shell::dialect() {
        liuma_sandbox::shell::Dialect::Posix => "bash",
        liuma_sandbox::shell::Dialect::PowerShell => "pwsh",
    };
    assert_eq!(liuma_sandbox::shell::tool_name(), expected);
}
