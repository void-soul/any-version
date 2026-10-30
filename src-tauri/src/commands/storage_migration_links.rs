//! Preserve directory reparse points without following them or creating their targets.
use std::path::Path;

pub(super) fn directory_entry(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    { use std::os::windows::fs::MetadataExt; metadata.file_attributes() & 0x10 != 0 }
    #[cfg(not(windows))]
    { metadata.is_dir() }
}

#[cfg(windows)]
pub(super) fn create_directory_link(link: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn DeviceIoControl(handle: *mut c_void, control: u32, input: *mut c_void, input_len: u32,
            output: *mut c_void, output_len: u32, returned: *mut u32, overlapped: *mut c_void) -> i32;
    }
    if !target.is_absolute() { return Err("目录链接目标必须是绝对路径".into()); }
    let target = super::plain_path(target);
    let display = target.to_string_lossy();
    let substitute = if let Some(rest) = display.strip_prefix(r"\\") {
        format!(r"\??\UNC\{rest}")
    } else { format!(r"\??\{display}") };
    let substitute: Vec<u16> = std::ffi::OsStr::new(&substitute).encode_wide().collect();
    let print: Vec<u16> = target.as_os_str().encode_wide().collect();
    let substitute_len = substitute.len() * 2;
    let print_len = print.len() * 2;
    let data_len = 8 + substitute_len + 2 + print_len + 2;
    if data_len + 8 > 16 * 1024 { return Err("目录链接路径超过系统上限".into()); }
    let mut buffer = Vec::with_capacity(data_len + 8);
    buffer.extend_from_slice(&0xA0000003u32.to_le_bytes()); // IO_REPARSE_TAG_MOUNT_POINT
    buffer.extend_from_slice(&(data_len as u16).to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&(substitute_len as u16).to_le_bytes());
    buffer.extend_from_slice(&((substitute_len + 2) as u16).to_le_bytes());
    buffer.extend_from_slice(&(print_len as u16).to_le_bytes());
    for value in substitute { buffer.extend_from_slice(&value.to_le_bytes()); }
    buffer.extend_from_slice(&0u16.to_le_bytes());
    for value in print { buffer.extend_from_slice(&value.to_le_bytes()); }
    buffer.extend_from_slice(&0u16.to_le_bytes());
    std::fs::create_dir(link).map_err(|e| e.to_string())?;
    let result = (|| {
        let file = std::fs::OpenOptions::new().write(true).access_mode(0x40000000).share_mode(0)
            .custom_flags(0x00200000 | 0x02000000).open(link).map_err(|e| e.to_string())?;
        let mut returned = 0u32;
        let ok = unsafe { DeviceIoControl(file.as_raw_handle(), 0x000900A4, buffer.as_mut_ptr().cast(), buffer.len() as u32,
            std::ptr::null_mut(), 0, &mut returned, std::ptr::null_mut()) };
        if ok == 0 { Err(format!("创建目录链接失败: {}", std::io::Error::last_os_error())) } else { Ok(()) }
    })();
    if result.is_err() { let _ = std::fs::remove_dir(link); }
    result
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn preserves_sdk_directory_junction_and_retargets_inside_new_root() {
        let mut bytes = [0u8; 8]; getrandom::getrandom(&mut bytes).unwrap();
        let root = std::env::temp_dir().join(format!("kira-junction-tests-{:016x}", u64::from_ne_bytes(bytes)));
        let source = root.join("source"); let target = root.join("target");
        let version = source.join("sdk/_versions/node/22");
        std::fs::create_dir_all(&version).unwrap(); std::fs::create_dir_all(&target).unwrap();
        std::fs::write(version.join("node.exe"), b"sdk payload").unwrap();
        create_directory_link(&source.join("sdk/node"), &version).unwrap();
        let metadata = std::fs::symlink_metadata(source.join("sdk/node")).unwrap();
        assert!(directory_entry(&metadata));
        super::super::copy_tree(&source.join("sdk"), &target.join("sdk"), &source, &target).unwrap();
        assert_eq!(std::fs::read(target.join("sdk/node/node.exe")).unwrap(), b"sdk payload");
        assert_eq!(target.join("sdk/node").canonicalize().unwrap(), target.join("sdk/_versions/node/22").canonicalize().unwrap());
        assert_eq!(std::fs::read(version.join("node.exe")).unwrap(), b"sdk payload");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn creating_a_junction_does_not_create_its_target() {
        let mut bytes = [0u8; 8]; getrandom::getrandom(&mut bytes).unwrap();
        let root = std::env::temp_dir().join(format!("kira-junction-target-tests-{:016x}", u64::from_ne_bytes(bytes)));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("does-not-exist"); let link = root.join("link");
        create_directory_link(&link, &target).unwrap();
        assert!(!target.exists());
        std::fs::remove_dir(link).unwrap(); std::fs::remove_dir(root).unwrap();
    }
}
