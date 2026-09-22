//! Job Object:进程树的唯一容器。
//!
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 让「最后一个句柄关闭」等价于
//! 「整棵树被内核终止」——runner 因此是这份句柄的唯一持有者,**杀掉 runner
//! 就是杀掉载荷树**,不需要额外协议。

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};

use super::error::{WinError, check};

/// 进程容器句柄(析构即关闭;关闭即终止树)
#[derive(Debug)]
pub struct Job(HANDLE);

impl Job {
    /// 建一个「句柄关闭即杀死成员」的 Job
    pub fn kill_on_close() -> Result<Self, WinError> {
        // SAFETY: 匿名 Job,无安全属性
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(WinError::last("CreateJobObjectW"));
        }
        let job = Self(handle);

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: info 与所声明的类型长度一致
        check("SetInformationJobObject", unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        })?;
        Ok(job)
    }

    /// 加入一个进程(载荷在挂入 Job 之前不得运行,见 [`super::process`])
    pub fn assign(&self, process: HANDLE) -> Result<(), WinError> {
        // SAFETY: 两个句柄都由调用方持有且有效
        check("AssignProcessToJobObject", unsafe {
            AssignProcessToJobObject(self.0, process)
        })
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: 句柄由本类型独占。关闭会连带终止仍在运行的成员
            unsafe { CloseHandle(self.0) };
        }
    }
}
