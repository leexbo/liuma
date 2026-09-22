//! Windows 沙箱后端:受限令牌 + 能力 SID 授权 + Job Object。
//!
//! 与 bwrap / seatbelt 同构:沙箱是**包装**——argv 前插 runner,由 runner
//! 建受限令牌并 `CreateProcessAsUserW` 拉起载荷。选这个形态而非在 liuma
//! 进程内直接受限创建,是因为 tokio 的 `Child` 抽象载不动自建的
//! `PROCESS_INFORMATION` + Job 句柄,而 `Confined{program, argv}` 已经够用。
//!
//! 分工:
//! - [`contract`]:两侧共用的纯逻辑(SID 派生、argv 形态、路径边界),
//!   全平台编译与单测;
//! - `win`:Win32 实现(仅 Windows 编译);
//! - `liuma-sandbox-run`:runner 二进制——它持有 Job 句柄,因此**杀掉
//!   runner 就等于杀掉整棵载荷树**(句柄关闭即触发 Job 的内核终止)。
//!
//! 边界(如实声明,不夸大):写入与删除受 ACL 约束,**读、网络与进程可见性
//! 不受限**;enforcement 恒为 `partial`(硬链接是文件对象别名、被
//! AppContainer 标记过的树读不了)。因此消费方不得把它当作绝对边界。

#![deny(missing_docs)]

pub mod contract;

#[cfg(windows)]
mod win;

/// runner 入口(见 `liuma-sandbox-run` 的模块文档):返回进程退出码
#[cfg(windows)]
pub fn run(argv: impl Iterator<Item = std::ffi::OsString>) -> i32 {
    win::run(argv)
}

pub use contract::{
    EXIT_RUNNER_FAILURE, FAIL_PREFIX, GRANT_MASK, Mode, RunnerSpec, WritableRoot,
    normalize_canonical, parse_runner_args, runner_argv, temp_sid, workspace_sid,
};
