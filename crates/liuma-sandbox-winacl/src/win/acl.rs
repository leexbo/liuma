//! 对可写根授予能力 SID(精确幂等)与授权存在性检查。

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, SE_FILE_OBJECT, SetEntriesInAclW,
    SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, EqualSid, GetAce,
    OBJECT_INHERIT_ACE, PSECURITY_DESCRIPTOR, UNPROTECTED_DACL_SECURITY_INFORMATION,
};
use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;

use super::error::WinError;
use super::sid::LocalSid;

/// 目录授权用的继承标志:对象与容器都继承
const INHERIT_BOTH: u32 = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE;

/// 在目录上授予 `sid` 写权限。
///
/// 提交的是 `DACL | UNPROTECTED`:`UNPROTECTED` 让新 ACE 沿既有子项传播
/// ——少了它,遇父目录带保护位 DACL 的工作区会「新建文件成功、改写/删除
/// 既有文件失败」这种极难定位的半残状态。
pub fn grant_write(dir: &Path, sid: &LocalSid, mask: u32) -> Result<(), WinError> {
    let wide = wide_path(dir);
    let mut old_dacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: wide 以 NUL 结尾;只取 DACL,其余出参传 NULL
    let rc = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut old_dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if rc != 0 {
        return Err(WinError {
            api: "GetNamedSecurityInfoW",
            code: rc,
            context: Some(dir.display().to_string()),
        });
    }

    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: mask,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: INHERIT_BOTH,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid.as_ptr() as *mut u16,
        },
    };
    let mut merged: *mut ACL = std::ptr::null_mut();
    // SAFETY: 单条 entry;old_dacl 属于 descriptor,合并结果单独分配
    let rc = unsafe { SetEntriesInAclW(1, &entry, old_dacl, &mut merged) };
    if rc != 0 {
        // SAFETY: descriptor 由 GetNamedSecurityInfoW 分配
        unsafe { LocalFree(descriptor) };
        return Err(WinError {
            api: "SetEntriesInAclW",
            code: rc,
            context: Some(dir.display().to_string()),
        });
    }
    // 描述符必须在提交前释放:ACL 指针指向描述符块内,不能单独释放,
    // 也不能在描述符已释放后再读它
    // SAFETY: descriptor 由上面的调用分配且尚未释放
    unsafe { LocalFree(descriptor) };

    // SAFETY: wide 与 merged 在调用期间有效
    let rc = unsafe {
        SetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            merged,
            std::ptr::null_mut(),
        )
    };
    // SAFETY: merged 由 SetEntriesInAclW 分配
    unsafe { LocalFree(merged as *mut core::ffi::c_void) };
    if rc != 0 {
        return Err(WinError {
            api: "SetNamedSecurityInfoW",
            code: rc,
            context: Some(dir.display().to_string()),
        });
    }
    Ok(())
}

/// 目录上是否已有**精确**的那条授权 ACE。
///
/// 精确 = 类型为允许、掩码与继承标志全等、SID 相等。它是「同一工作区只在
/// 首次授予时付一次全树传播代价」的依据(授权是急切传播的,大仓库首次能
/// 到数十秒)。
pub fn exact_ace_present(dir: &Path, sid: &LocalSid, mask: u32) -> Result<bool, WinError> {
    let wide = wide_path(dir);
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: wide 以 NUL 结尾;只取 DACL
    let rc = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if rc != 0 {
        return Err(WinError {
            api: "GetNamedSecurityInfoW",
            code: rc,
            context: Some(dir.display().to_string()),
        });
    }

    let found = if dacl.is_null() {
        false
    } else {
        // SAFETY: dacl 由上面的调用产出
        is_exact_ace(unsafe { &*dacl }, sid, mask)
    };
    // SAFETY: descriptor 由 GetNamedSecurityInfoW 分配
    unsafe { LocalFree(descriptor) };
    Ok(found)
}

/// 在 ACL 里找精确匹配的允许 ACE
fn is_exact_ace(dacl: &ACL, sid: &LocalSid, mask: u32) -> bool {
    let count = dacl.AceCount;
    for i in 0..count {
        let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: i < AceCount,dacl 有效
        if unsafe { GetAce(dacl as *const ACL as *const _, i as u32, &mut ace) } == 0 {
            continue;
        }
        // SAFETY: ace 指向 ACL 内的一个 ACE;先读头部判类型
        let header = unsafe { &*(ace as *const windows_sys::Win32::Security::ACE_HEADER) };
        if header.AceType != ACCESS_ALLOWED_ACE_TYPE as u8 {
            continue;
        }
        // SAFETY: 类型已确认,可按 ACCESS_ALLOWED_ACE 解读
        let allowed = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
        if allowed.Mask != mask {
            continue;
        }
        if u32::from(header.AceFlags) & INHERIT_BOTH != INHERIT_BOTH {
            continue;
        }
        // SidStart 是 ACE 里内联 SID 的起点(不是指针)
        let ace_sid = unsafe {
            (ace as *const u8).add(std::mem::offset_of!(ACCESS_ALLOWED_ACE, SidStart)) as *mut _
        };
        // SAFETY: 内联 SID 随 ACE 一起驻留在 ACL 内
        if unsafe { EqualSid(ace_sid, sid.as_ptr()) } != 0 {
            return true;
        }
    }
    false
}

/// 路径 → 以 NUL 结尾的宽串
fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
