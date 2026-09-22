//! Windows 沙箱 runner:`liuma-sandbox-run --mode … [--writable <dir> <sid>]… -- <程序> [参数]…`
//!
//! 职责(顺序即 fail-closed 时序,任何一步失败都不 spawn 不受限的载荷):
//! 1. 参数一致性(模式与可写根数量匹配);
//! 2. 每个可写根 canonical 归一 + 重算能力 SID 与调用方对账;
//! 3. 建受限令牌(保活组恒在,写 SID 按模式加入);
//! 4. 修令牌默认 DACL(否则受限孙进程建管道必失败);
//! 5. 逐根授予写权限(精确 ACE 已在则跳过);
//! 6. 建 Job 并受限创建载荷,退出码原样镜像(全 32 位)。
//!
//! 失败一律:`liuma-sandbox-run: <细节>` 到 stderr + 退出码 127。

fn main() {
    #[cfg(windows)]
    {
        std::process::exit(liuma_sandbox_winacl::run(std::env::args_os().skip(1)));
    }
    #[cfg(not(windows))]
    {
        // 非 Windows 上不存在受限令牌与 Job Object:明确报错而不是假装成功
        eprintln!(
            "{}this runner only runs on Windows",
            liuma_sandbox_winacl::FAIL_PREFIX
        );
        std::process::exit(liuma_sandbox_winacl::EXIT_RUNNER_FAILURE);
    }
}
