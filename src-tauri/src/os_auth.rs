//! 用系统的身份验证确认是电脑的主人在操作，用于查看保存的密码之前。
//!
//! 弹出的是系统对话框，调用会一直阻塞到用户完成或取消验证，须放在阻塞线程里执行。

use std::fmt;

use crate::i18n::Msg;

/// 弹出系统验证对话框。通过返回 `Ok(true)`，用户取消返回 `Ok(false)`。
/// `parent` 是对话框所属窗口的句柄，只在 Windows 上使用。
pub fn verify(parent: Option<isize>) -> Result<bool, Msg> {
    imp::verify(parent)
}

fn unavailable(cause: impl fmt::Display) -> Msg {
    msg!("err.osAuth.unavailable", "无法验证电脑密码").with_cause(cause)
}

#[cfg_attr(windows, allow(dead_code))]
fn denied() -> Msg {
    msg!("err.osAuth.denied", "电脑密码验证未通过")
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCOUNT_RESTRICTION, ERROR_CANCELLED, ERROR_LOGON_FAILURE, ERROR_SUCCESS,
        GetLastError, HANDLE,
    };
    use windows_sys::Win32::Security::Credentials::{
        CRED_PACK_PROTECTED_CREDENTIALS, CREDUI_INFOW, CREDUI_MAX_DOMAIN_TARGET_LENGTH,
        CREDUI_MAX_USERNAME_LENGTH, CREDUIWIN_ENUMERATE_CURRENT_USER, CredUIParseUserNameW,
        CredUIPromptForWindowsCredentialsW, CredUnPackAuthenticationBufferW,
    };
    use windows_sys::Win32::Security::{LOGON32_LOGON_NETWORK, LOGON32_PROVIDER_DEFAULT, LogonUserW};
    use windows_sys::Win32::System::Com::CoTaskMemFree;

    use super::{Msg, unavailable};
    use crate::i18n::pick;

    const USER_LEN: usize = CREDUI_MAX_USERNAME_LENGTH as usize + 1;
    const DOMAIN_LEN: usize = CREDUI_MAX_DOMAIN_TARGET_LENGTH as usize + 1;
    const PASSWORD_LEN: usize = 1024;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn verify(parent: Option<isize>) -> Result<bool, Msg> {
        let caption = wide("Rain Term");
        let message = wide(pick([
            "请输入这台电脑的登录密码，验证通过后才能查看保存的密码。",
            "請輸入這台電腦的登入密碼，驗證通過後才能查看儲存的密碼。",
            "Enter this computer's sign-in password to view the saved password.",
        ]));
        let info = CREDUI_INFOW {
            cbSize: size_of::<CREDUI_INFOW>() as u32,
            hwndParent: parent.map_or(null_mut(), |h| h as *mut c_void),
            pszMessageText: message.as_ptr(),
            pszCaptionText: caption.as_ptr(),
            hbmBanner: null_mut(),
        };
        let mut auth_error = 0;
        loop {
            let mut package = 0u32;
            let mut buf: *mut c_void = null_mut();
            let mut size = 0u32;
            let mut save = 0;
            let r = unsafe {
                CredUIPromptForWindowsCredentialsW(
                    &info,
                    auth_error,
                    &mut package,
                    null(),
                    0,
                    &mut buf,
                    &mut size,
                    &mut save,
                    CREDUIWIN_ENUMERATE_CURRENT_USER,
                )
            };
            match r {
                ERROR_SUCCESS => {}
                ERROR_CANCELLED => return Ok(false),
                e => return Err(unavailable(std::io::Error::from_raw_os_error(e as i32))),
            }
            let passed = unsafe {
                let passed = logon(buf, size);
                std::ptr::write_bytes(buf as *mut u8, 0, size as usize);
                CoTaskMemFree(buf);
                passed
            };
            if passed {
                return Ok(true);
            }
            // 再次弹出对话框，并提示密码不正确
            auth_error = ERROR_LOGON_FAILURE;
        }
    }

    /// 解出对话框返回的账号和密码，交给系统校验。
    unsafe fn logon(buf: *const c_void, size: u32) -> bool {
        let mut user = [0u16; USER_LEN];
        let mut domain = [0u16; DOMAIN_LEN];
        let mut password = [0u16; PASSWORD_LEN];
        let (mut user_len, mut domain_len, mut password_len) =
            (USER_LEN as u32, DOMAIN_LEN as u32, PASSWORD_LEN as u32);
        let unpacked = unsafe {
            CredUnPackAuthenticationBufferW(
                CRED_PACK_PROTECTED_CREDENTIALS,
                buf,
                size,
                user.as_mut_ptr(),
                &mut user_len,
                domain.as_mut_ptr(),
                &mut domain_len,
                password.as_mut_ptr(),
                &mut password_len,
            )
        } != 0;
        let passed = unpacked && unsafe { check(&user, &password) };
        password.fill(0);
        passed
    }

    unsafe fn check(full_name: &[u16], password: &[u16]) -> bool {
        // 对话框返回的账号形如“域\用户名”或“user@domain”，LogonUserW 要求分开传
        let mut user = [0u16; USER_LEN];
        let mut domain = [0u16; DOMAIN_LEN];
        let parsed = unsafe {
            CredUIParseUserNameW(
                full_name.as_ptr(),
                user.as_mut_ptr(),
                USER_LEN as u32,
                domain.as_mut_ptr(),
                DOMAIN_LEN as u32,
            )
        } == ERROR_SUCCESS;
        let (user, domain) = if parsed {
            (user.as_ptr(), domain.as_ptr())
        } else {
            (full_name.as_ptr(), null())
        };
        let mut token: HANDLE = null_mut();
        let ok = unsafe {
            LogonUserW(
                user,
                domain,
                password.as_ptr(),
                LOGON32_LOGON_NETWORK,
                LOGON32_PROVIDER_DEFAULT,
                &mut token,
            )
        } != 0;
        if ok {
            unsafe { CloseHandle(token) };
            return true;
        }
        // 没有设置密码的账号不允许网络登录，系统会报账号受限而不是密码错误
        password[0] == 0 && unsafe { GetLastError() } == ERROR_ACCOUNT_RESTRICTION
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::sync::mpsc;

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAError, LAPolicy};

    use super::{Msg, denied, unavailable};
    use crate::i18n::pick;

    pub fn verify(_parent: Option<isize>) -> Result<bool, Msg> {
        let (tx, rx) = mpsc::channel();
        let reply = RcBlock::new(move |ok: Bool, err: *mut NSError| {
            let failure = (!ok.as_bool()).then(|| {
                unsafe { err.as_ref() }
                    .map(|e| (e.code(), e.localizedDescription().to_string()))
                    .unwrap_or((0, String::new()))
            });
            let _ = tx.send(failure);
        });
        // 系统会显示为“Rain Term 正在尝试<reason>”
        let reason = NSString::from_str(pick(["查看保存的密码", "查看儲存的密碼", "view the saved password"]));
        let context = unsafe { LAContext::new() };
        unsafe {
            context.evaluatePolicy_localizedReason_reply(
                LAPolicy::DeviceOwnerAuthentication,
                &reason,
                &reply,
            )
        };
        match rx.recv() {
            Ok(None) => Ok(true),
            Ok(Some((code, _)))
                if [LAError::UserCancel, LAError::SystemCancel, LAError::AppCancel]
                    .iter()
                    .any(|e| e.0 == code) =>
            {
                Ok(false)
            }
            Ok(Some((code, _))) if code == LAError::AuthenticationFailed.0 => Err(denied()),
            Ok(Some((code, desc))) => Err(unavailable(format!("{desc} ({code})"))),
            Err(e) => Err(unavailable(e)),
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use std::process::Command;

    use super::{Msg, denied, unavailable};

    pub fn verify(_parent: Option<isize>) -> Result<bool, Msg> {
        let out = Command::new("pkcheck")
            .args(["--action-id", "org.freedesktop.policykit.exec", "--allow-user-interaction", "--process"])
            .arg(std::process::id().to_string())
            .output()
            .map_err(unavailable)?;
        // pkcheck 的退出码：0 通过，1 未通过，2 无法弹出验证（没有运行 polkit 验证代理），3 用户取消
        match out.status.code() {
            Some(0) => Ok(true),
            Some(1) => Err(denied()),
            Some(3) => Ok(false),
            _ => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let cause = stderr.trim();
                Err(unavailable(if cause.is_empty() { out.status.to_string() } else { cause.to_string() }))
            }
        }
    }
}
