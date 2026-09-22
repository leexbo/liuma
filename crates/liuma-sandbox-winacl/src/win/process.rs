//! 受限进程的创建:先以挂起态创建、挂入 Job、再恢复。
//!
//! 顺序不能换:载荷代码必须在**已经属于 Job** 之后才开始跑,否则
//! create → assign 之间的窗口里它能在容器之外做事。

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, GetExitCodeProcess,
    INFINITE, PROCESS_INFORMATION, ResumeThread, STARTF_USESHOWWINDOW, STARTF_USESTDHANDLES,
    STARTUPINFOW, WaitForSingleObject,
};

use super::error::WinError;
use super::job::Job;
use super::token::Token;

/// 窗口不显示(载荷是命令行程序;`CREATE_NO_WINDOW` 在受限令牌下会让
/// DLL 初始化失败,不能用)
const SW_HIDE: u16 = 0;

/// 以受限令牌创建载荷,挂入 Job 后恢复运行,等待退出
pub fn spawn_in_job(
    token: &Token,
    job: &Job,
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
) -> Result<i32, WinError> {
    let command_line = build_command_line(program, args);
    let mut cmd_line: Vec<u16> = command_line
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let app: Vec<u16> = program.encode_utf16().chain(std::iter::once(0)).collect();
    let cwd_wide: Option<Vec<u16>> = cwd.map(|p| {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    });

    // stdio 由 runner 继承(liuma 已把管道接到 runner 上):把三个标准句柄
    // 显式传给载荷,并确保它们带继承位
    let std_in = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    let std_out = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    let std_err = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
    for handle in [std_in, std_out, std_err] {
        if !handle.is_null() {
            // SAFETY: 句柄来自本进程的标准流;失败不致命(某些句柄本就不该继承)
            unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
        }
    }

    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    si.dwFlags = STARTF_USESTDHANDLES | STARTF_USESHOWWINDOW;
    si.wShowWindow = SW_HIDE;
    si.hStdInput = std_in;
    si.hStdOutput = std_out;
    si.hStdError = std_err;

    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: 各缓冲区在调用期间有效;lpEnvironment = NULL 表示继承 runner 的
    // 环境——那正是 liuma 逐键设定过的环境,且天然保留 Windows 的 `=X:`
    // 驱动器伪条目(重建环境块容易丢掉它们)
    let created = unsafe {
        CreateProcessAsUserW(
            token.raw(),
            app.as_ptr(),
            cmd_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1, // bInheritHandles:stdio 要传下去
            CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
            std::ptr::null(),
            cwd_wide.as_ref().map_or(std::ptr::null(), |w| w.as_ptr()),
            &si,
            &mut pi,
        )
    };
    if created == 0 {
        return Err(WinError::last("CreateProcessAsUserW").with_context(program.to_string()));
    }

    // 从这里起无论成败都要回收句柄:进程与线程句柄由本函数独占
    let outcome = run_after_create(job, &pi);
    // SAFETY: 两个句柄由 CreateProcessAsUserW 产出,本函数独占
    unsafe {
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
    }
    outcome
}

/// 挂 Job → 恢复 → 等退出(与句柄回收分离,便于 `?` 直出)
fn run_after_create(job: &Job, pi: &PROCESS_INFORMATION) -> Result<i32, WinError> {
    if let Err(e) = job.assign(pi.hProcess) {
        // 进不了容器就绝不能让它跑起来
        // SAFETY: 句柄有效;这是失败路径上的兜底终止
        unsafe { windows_sys::Win32::System::Threading::TerminateProcess(pi.hProcess, 1) };
        return Err(e);
    }
    // SAFETY: 线程句柄由 CreateProcessAsUserW 产出且尚未关闭
    if unsafe { ResumeThread(pi.hThread) } == u32::MAX {
        return Err(WinError::last("ResumeThread"));
    }
    // SAFETY: 进程句柄有效
    let waited = unsafe { WaitForSingleObject(pi.hProcess, INFINITE) };
    if waited != WAIT_OBJECT_0 {
        return Err(WinError::last("WaitForSingleObject"));
    }
    let mut code: u32 = 0;
    // SAFETY: 等待已完成,退出码可读
    if unsafe { GetExitCodeProcess(pi.hProcess, &mut code) } == 0 {
        return Err(WinError::last("GetExitCodeProcess"));
    }
    // 原样镜像:崩溃码(0xC0000005 一类)也是 u32,截成 i32 交给上层按负码分类
    Ok(code as i32)
}

/// 按 `CommandLineToArgvW` 的规则拼命令行(载荷是 shell,参数会再经它解析)
pub fn build_command_line(program: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(quote_arg(program));
    for arg in args {
        parts.push(quote_arg(arg));
    }
    parts.join(" ")
}

/// 单个参数的引号规则:仅含空白或引号时才加引号,反斜杠按 `2n+1` 规则翻倍
fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => {
                backslashes += 1;
                out.push('\\');
            }
            '"' => {
                // 引号前的 n 个反斜杠要翻倍,再加一个转义引号
                for _ in 0..=backslashes {
                    out.push('\\');
                }
                backslashes = 0;
                out.push('"');
            }
            _ => {
                backslashes = 0;
                out.push(ch);
            }
        }
    }
    // 收尾的反斜杠紧邻闭合引号,同样需要翻倍
    for _ in 0..backslashes {
        out.push('\\');
    }
    out.push('"');
    out
}
