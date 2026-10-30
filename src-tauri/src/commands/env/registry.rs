use super::*;

#[cfg(windows)]
pub fn broadcast_setting_change() {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    type SendMessageTimeoutW = unsafe extern "system" fn(
        hwnd: *mut std::ffi::c_void,
        msg: u32,
        wparam: usize,
        lparam: *const u16,
        flags: u32,
        timeout: u32,
        result: *mut usize,
    ) -> isize;

    unsafe {
        let module_name: Vec<u16> = OsStr::new("user32.dll")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let handle = LoadLibraryW(module_name.as_ptr());
        if !handle.is_null() {
            let proc_name = std::ffi::CString::new("SendMessageTimeoutW").unwrap();
            let proc_addr = GetProcAddress(handle, proc_name.as_ptr() as *const u8);
            if !proc_addr.is_null() {
                let send_msg_timeout: SendMessageTimeoutW = std::mem::transmute(proc_addr);
                let env_str: Vec<u16> = OsStr::new("Environment")
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                let mut result = 0;
                send_msg_timeout(
                    0xffff as *mut std::ffi::c_void, // HWND_BROADCAST
                    0x001a,                          // WM_SETTINGCHANGE
                    0,
                    env_str.as_ptr(),
                    0x0002, // SMTO_ABORTIFHUNG
                    5000,
                    &mut result,
                );
            }
            FreeLibrary(handle);
        }
    }
}

#[cfg(windows)]
extern "system" {
    fn LoadLibraryW(lpLibFileName: *const u16) -> *mut std::ffi::c_void;
    fn GetProcAddress(
        hModule: *mut std::ffi::c_void,
        lpProcName: *const u8,
    ) -> *mut std::ffi::c_void;
    fn FreeLibrary(hLibModule: *mut std::ffi::c_void) -> i32;
}

#[cfg(not(windows))]
pub fn broadcast_setting_change() {}

pub fn get_registry_env(name: &str) -> Option<String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(env_key) = hkcu.open_subkey("Environment") {
        if let Ok(val) = env_key.get_value::<String, _>(name) {
            return Some(val);
        }
    }
    None
}

/// 读取系统级(HKLM)环境变量
pub fn get_system_registry_env(name: &str) -> Option<String> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(env_key) =
        hklm.open_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment")
    {
        if let Ok(val) = env_key.get_value::<String, _>(name) {
            return Some(val);
        }
    }
    None
}

/// 同时检查用户级和系统级注册表，返回值及其来源
pub fn get_registry_env_any(name: &str) -> Option<(String, &'static str)> {
    // 用户级优先
    if let Some(val) = get_registry_env(name) {
        if !val.is_empty() {
            return Some((val, "HKCU"));
        }
    }
    // 系统级
    if let Some(val) = get_system_registry_env(name) {
        if !val.is_empty() {
            return Some((val, "HKLM"));
        }
    }
    None
}

/// 智能写入注册表环境变量：
/// - 值包含 % 时使用 REG_EXPAND_SZ（支持 %SystemRoot% 等展开）
/// - 普通字符串使用 REG_SZ
#[cfg(windows)]
pub(super) fn set_registry_value_smart(
    key: &RegKey,
    name: &str,
    value: &str,
) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let is_expandable = value.contains('%');
    let reg_type = if is_expandable { REG_EXPAND_SZ } else { REG_SZ };

    // 将字符串编码为 UTF-16LE + null terminator，再转为 &[u8]
    let wide: Vec<u16> = OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0)) // null terminator
        .collect();
    let bytes = unsafe {
        std::slice::from_raw_parts(
            wide.as_ptr() as *const u8,
            wide.len() * std::mem::size_of::<u16>(),
        )
    };
    let reg_value = winreg::RegValue {
        vtype: reg_type,
        bytes: bytes.to_vec(),
    };
    key.set_raw_value(name, &reg_value)
        .map_err(|e| e.to_string())
}

pub fn set_registry_env(name: &str, value: &str) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (env_key, _) = hkcu
        .create_subkey("Environment")
        .map_err(|e| e.to_string())?;
    if value.is_empty() {
        let _ = env_key.delete_value(name);
    } else {
        set_registry_value_smart(&env_key, name, value)?;
    }
    broadcast_setting_change();
    Ok(())
}

pub fn set_system_registry_env(name: &str, value: &str) -> Result<(), String> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let (env_key, _) = hklm
        .create_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment")
        .map_err(|e| e.to_string())?;
    if value.is_empty() {
        let _ = env_key.delete_value(name);
    } else {
        set_registry_value_smart(&env_key, name, value)?;
    }
    broadcast_setting_change();
    Ok(())
}
