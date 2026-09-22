//! SID 的拥有形式与取用:自铸能力 SID、知名 SID、令牌里的登录 SID。

use windows_sys::Win32::Foundation::{HANDLE, LocalFree};
use windows_sys::Win32::Security::{
    CreateWellKnownSid, GetLengthSid, PSID, SECURITY_MAX_SID_SIZE, SID_AND_ATTRIBUTES,
    TOKEN_GROUPS, TOKEN_INFORMATION_CLASS, TokenGroups, WinWorldSid,
};
use windows_sys::Win32::Security::{GetTokenInformation, IsValidSid};
use windows_sys::Win32::System::SystemServices::SE_GROUP_LOGON_ID;

use super::error::{WinError, check};

/// 一个自有的 SID 缓冲区(`SECURITY_MAX_SID_SIZE` 足够容纳任何 SID)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSid(Vec<u8>);

impl LocalSid {
    /// 从 SDDL 字符串构造(`S-1-4-…` 一类的自铸身份)
    pub fn from_string(sddl: &str) -> Result<Self, WinError> {
        use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
        let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut raw: PSID = std::ptr::null_mut();
        // SAFETY: wide 以 NUL 结尾;成功时 raw 指向 LocalAlloc 的内存,下面复制后释放
        check("ConvertStringSidToSidW", unsafe {
            ConvertStringSidToSidW(wide.as_ptr(), &mut raw)
        })?;
        // SAFETY: raw 由上面的调用产出且校验通过
        let sid = unsafe { Self::copy_from(raw) };
        // SAFETY: ConvertStringSidToSidW 的内存归调用方,须 LocalFree 释放
        unsafe { LocalFree(raw) };
        sid
    }

    /// 知名 SID(如 Everyone)
    pub fn well_known(kind: i32) -> Result<Self, WinError> {
        let mut buf = vec![0u8; SECURITY_MAX_SID_SIZE as usize];
        let mut len = buf.len() as u32;
        // SAFETY: buf 按 SECURITY_MAX_SID_SIZE 分配,len 同步给出容量
        check("CreateWellKnownSid", unsafe {
            CreateWellKnownSid(
                kind,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as PSID,
                &mut len,
            )
        })?;
        buf.truncate(len as usize);
        Ok(Self(buf))
    }

    /// Everyone(`S-1-1-0`)
    pub fn everyone() -> Result<Self, WinError> {
        Self::well_known(WinWorldSid)
    }

    /// 裸指针(仅在持有 `self` 的语句内使用)
    pub fn as_ptr(&self) -> PSID {
        self.0.as_ptr() as PSID
    }

    /// 从裸 SID 复制一份自有副本
    ///
    /// # Safety
    /// `raw` 必须指向一个合法的 SID(长度由 `GetLengthSid` 给出)
    unsafe fn copy_from(raw: PSID) -> Result<Self, WinError> {
        // SAFETY: 由调用方保证 raw 合法
        let len = unsafe { GetLengthSid(raw) };
        if len == 0 {
            return Err(WinError::last("GetLengthSid"));
        }
        // SAFETY: raw 至少有 len 字节
        let slice = unsafe { std::slice::from_raw_parts(raw as *const u8, len as usize) };
        Ok(Self(slice.to_vec()))
    }

    /// 校验缓冲区确实是个合法 SID
    pub fn validate(&self) -> Result<(), WinError> {
        // SAFETY: self.0 是已分配的缓冲区
        check("IsValidSid", unsafe { IsValidSid(self.as_ptr()) })
    }
}

/// 取令牌里的**登录 SID**。
///
/// 受限令牌的 restricting 列表必须保留它:缺了它,子进程的早期 DLL 初始化
/// 会以 `STATUS_DLL_INIT_FAILED`(0xC0000142)死掉——这是「沙箱里什么都跑
/// 不起来」最常见的原因。
pub fn logon_sid(token: HANDLE) -> Result<LocalSid, WinError> {
    const CLASS: TOKEN_INFORMATION_CLASS = TokenGroups;
    let mut needed: u32 = 0;
    // SAFETY: 先问长度,缓冲区传 NULL 是文档规定的用法
    unsafe {
        GetTokenInformation(token, CLASS, std::ptr::null_mut(), 0, &mut needed);
    }
    if needed == 0 {
        return Err(WinError::last("GetTokenInformation(TokenGroups)"));
    }
    let mut buf = vec![0u8; needed as usize];
    // SAFETY: buf 长度即上面问到的字节数
    check("GetTokenInformation(TokenGroups)", unsafe {
        GetTokenInformation(
            token,
            CLASS,
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            needed,
            &mut needed,
        )
    })?;

    // 缓冲来自 API,只保证字节对齐,故一律按未对齐读取——强转成
    // `*const TOKEN_GROUPS` 再解引用是未对齐解引用 UB(debug 下会直接 abort)
    let base = buf.as_ptr();
    let count = unsafe { std::ptr::read_unaligned(base as *const u32) };
    // Groups 是柔性数组:偏移取字段偏移,步长取元素大小(不能拿结构体大小减)
    let start = std::mem::offset_of!(TOKEN_GROUPS, Groups);
    let stride = std::mem::size_of::<SID_AND_ATTRIBUTES>();
    let attrs_offset = std::mem::offset_of!(SID_AND_ATTRIBUTES, Attributes);
    for i in 0..count as usize {
        let offset = start + i * stride;
        if offset + stride > buf.len() {
            break;
        }
        // SAFETY: offset 在 buf 内;两个字段各自按未对齐读
        let (sid_ptr, attrs) = unsafe {
            (
                std::ptr::read_unaligned(base.add(offset) as *const PSID),
                std::ptr::read_unaligned(base.add(offset + attrs_offset) as *const u32),
            )
        };
        // SE_GROUP_LOGON_ID 是属性位里的**掩码**(0xC0000000),不是全值:
        // 登录组实际带着 SE_GROUP_ENABLED 一类的伴生位(如 0xC0000004),
        // 按相等判会一个都匹配不上
        let logon_mask = SE_GROUP_LOGON_ID as u32;
        if attrs & logon_mask == logon_mask {
            // SAFETY: sid_ptr 由 GetTokenInformation 填充,指向令牌组里的合法 SID
            return unsafe { LocalSid::copy_from(sid_ptr) };
        }
    }
    Err(WinError {
        api: "logon SID lookup",
        code: 0,
        context: Some(format!("no logon SID among {count} token groups")),
    })
}
