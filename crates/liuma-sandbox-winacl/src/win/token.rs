//! 受限令牌:构造 restricting 列表,并修补令牌的默认 DACL。
//!
//! 两处「缺了就跑不起来」的坑都在这个文件里,注释写明原因。

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GRANT_ACCESS, SetEntriesInAclW, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN,
    TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    DISABLE_MAX_PRIVILEGE, GetTokenInformation, LUA_TOKEN, SID_AND_ATTRIBUTES, SetTokenInformation,
    TOKEN_ADJUST_DEFAULT, TOKEN_ASSIGN_PRIMARY, TOKEN_DEFAULT_DACL, TOKEN_DUPLICATE,
    TOKEN_INFORMATION_CLASS, TOKEN_QUERY, TokenDefaultDacl, WRITE_RESTRICTED,
};
use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::error::{WinError, check};
use super::sid::LocalSid;

/// 进程令牌句柄(析构即关闭)
#[derive(Debug)]
pub struct Token(HANDLE);

impl Token {
    /// 打开当前进程的令牌
    pub fn open_current() -> Result<Self, WinError> {
        let mut handle: HANDLE = std::ptr::null_mut();
        // SAFETY: 伪句柄由系统提供,始终有效
        check("OpenProcessToken", unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ADJUST_DEFAULT | TOKEN_ASSIGN_PRIMARY,
                &mut handle,
            )
        })?;
        Ok(Self(handle))
    }

    /// 裸句柄
    pub fn raw(&self) -> HANDLE {
        self.0
    }

    /// 建受限令牌。
    ///
    /// `restricting` 是 restricting SID 列表。**`WRITE_RESTRICTED` 恒设**:
    /// 它让 restricting SID 只参与**写**类的交叉检查(读与执行仍按正常
    /// 令牌判定)。不设它,restricting 列表会对每次访问都求交,于是连读
    /// `C:\Windows\System32` 里的系统 DLL 都会被拒 —— 表现是载荷「能启动
    /// 但什么都做不了」,极难从上层定位。只读模式因此也恒设它:只读的语义
    /// 由「restricting 列表里没有任何写 SID」表达,不由标志位表达。
    ///
    /// **调用方必须把「登录 SID + Everyone」放进 restricting 列表**——它们
    /// 是保活组,缺了会让子进程的 DLL 初始化失败。
    pub fn create_restricted(&self, restricting: &[&LocalSid]) -> Result<Self, WinError> {
        let entries: Vec<SID_AND_ATTRIBUTES> = restricting
            .iter()
            .map(|sid| SID_AND_ATTRIBUTES {
                Sid: sid.as_ptr(),
                Attributes: 0,
            })
            .collect();
        let flags = DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED;
        let mut new_token: HANDLE = std::ptr::null_mut();
        // SAFETY: entries 的指针在调用期间有效;不删特权、不禁用 SID(传 0/NULL)
        check("CreateRestrictedToken", unsafe {
            windows_sys::Win32::Security::CreateRestrictedToken(
                self.0,
                flags,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                entries.len() as u32,
                entries.as_ptr(),
                &mut new_token,
            )
        })?;
        Ok(Self(new_token))
    }

    /// 往令牌的**默认 DACL** 里补一条 `sid` 的完全访问。
    ///
    /// 不做这一步,受限子进程新建匿名管道(stdio)、事件、互斥体等对象时,
    /// 新对象自带的默认 DACL 过不了 restricting 的交叉检查 —— 表现为「带
    /// 管道的孙进程一律 spawn 失败」。新对象仍受创建工作区的路径约束,
    /// 这条 ACE 只解决「新建对象自身能否被自己访问」。
    pub fn grant_default_dacl(&self, sid: &LocalSid) -> Result<(), WinError> {
        const CLASS: TOKEN_INFORMATION_CLASS = TokenDefaultDacl;
        let mut needed: u32 = 0;
        // SAFETY: 先问长度
        unsafe {
            GetTokenInformation(self.0, CLASS, std::ptr::null_mut(), 0, &mut needed);
        }
        if needed == 0 {
            return Err(WinError::last("GetTokenInformation(TokenDefaultDacl)"));
        }
        let mut buf = vec![0u8; needed as usize];
        // SAFETY: buf 长度即问到的字节数
        check("GetTokenInformation(TokenDefaultDacl)", unsafe {
            GetTokenInformation(
                self.0,
                CLASS,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                needed,
                &mut needed,
            )
        })?;

        // 缓冲只保证字节对齐,按未对齐读取(强转成引用是未对齐解引用 UB)
        // SAFETY: 调用成功后 buf 至少容纳一个 TOKEN_DEFAULT_DACL
        let current: TOKEN_DEFAULT_DACL =
            unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const TOKEN_DEFAULT_DACL) };
        if current.DefaultDacl.is_null() {
            return Err(WinError {
                api: "token default DACL",
                code: 0,
                context: Some("token has a NULL default DACL".into()),
            });
        }

        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_ALL_ACCESS,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: 0,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: std::ptr::null_mut(),
                MultipleTrusteeOperation: 0,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN,
                ptstrName: sid.as_ptr() as *mut u16,
            },
        };
        let mut merged: *mut windows_sys::Win32::Security::ACL = std::ptr::null_mut();
        // SAFETY: 单条 entry;old ACL 在 buf 内,合并结果 merged 单独分配
        let rc = unsafe { SetEntriesInAclW(1, &entry, current.DefaultDacl, &mut merged) };
        if rc != 0 {
            return Err(WinError {
                api: "SetEntriesInAclW(default DACL)",
                code: rc,
                context: None,
            });
        }

        let info = TOKEN_DEFAULT_DACL {
            DefaultDacl: merged,
        };
        // SAFETY: info 是 TOKEN_DEFAULT_DACL,长度与之相符
        let set = unsafe {
            SetTokenInformation(
                self.0,
                CLASS,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<TOKEN_DEFAULT_DACL>() as u32,
            )
        };
        // SAFETY: merged 由 SetEntriesInAclW 分配,须 LocalFree 释放。
        // `buf` 是 Rust 的 Vec(API 只是往里写),不能交给 LocalFree
        unsafe { LocalFree(merged as *mut core::ffi::c_void) };
        check("SetTokenInformation(TokenDefaultDacl)", set)
    }
}

impl Drop for Token {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: 句柄由本类型独占
            unsafe { CloseHandle(self.0) };
        }
    }
}
