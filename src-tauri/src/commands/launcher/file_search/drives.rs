use super::*;

pub(super) fn normalize_drive(drive: &str) -> Result<String, String> {
    let d = drive.trim().trim_end_matches('\\').to_uppercase();
    let bytes = d.as_bytes();
    if bytes.len() == 2 && bytes[0].is_ascii_uppercase() && bytes[1] == b':' {
        Ok(d)
    } else {
        Err(format!("无效盘符: {}", drive))
    }
}

#[cfg(windows)]
pub(super) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
pub(super) fn drive_type_key(t: u32) -> &'static str {
    // GetDriveTypeW 返回值（winbase.h），windows-sys 未导出对应常量。
    match t {
        2 => "removable",
        3 => "fixed",
        4 => "network",
        5 => "cdrom",
        6 => "ramdisk",
        _ => "unknown",
    }
}

#[cfg(windows)]
pub(super) fn windows_drives() -> Vec<(String, String, u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives,
    };

    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = format!("{}:", (b'A' + i as u8) as char);
        let root = format!("{}\\", letter);
        let w = wide(&root);
        let dtype = drive_type_key(unsafe { GetDriveTypeW(w.as_ptr()) });
        let mut free_to_caller = 0u64;
        let mut total = 0u64;
        let mut free = 0u64;
        let ok =
            unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut free_to_caller, &mut total, &mut free) };
        if ok == 0 {
            total = 0;
            free = 0;
        }
        out.push((letter, dtype.to_string(), total, free));
    }
    out
}

#[cfg(not(windows))]
pub(super) fn windows_drives() -> Vec<(String, String, u64, u64)> {
    Vec::new()
}
