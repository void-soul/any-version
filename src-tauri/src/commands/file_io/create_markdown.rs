use std::io::Write;
use std::path::{Path, PathBuf};

fn markdown_name(input: &str) -> Result<String, String> {
    let name = input.trim();
    if name.is_empty() || name.len() > 240 || name == "." || name == ".."
        || name.ends_with(['.', ' '])
        || name.chars().any(|c| c.is_control() || "<>:\"/\\|?*".contains(c)) {
        return Err("请输入有效的文件名，不能包含目录或特殊字符".into());
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && matches!(stem.as_bytes()[3], b'1'..=b'9')) {
        return Err("此文件名为系统保留名称".into());
    }
    let ext = Path::new(name).extension().and_then(|ext| ext.to_str()).unwrap_or_default().to_ascii_lowercase();
    Ok(if ["md", "markdown", "mdx", "mdown"].contains(&ext.as_str()) { name.to_string() } else { format!("{name}.md") })
}

pub(super) fn create(directory: &Path, name: &str) -> Result<PathBuf, String> {
    let file_name = markdown_name(name)?;
    let root = directory.canonicalize().map_err(|e| format!("读取目录失败: {e}"))?;
    if !root.is_dir() { return Err("请选择文件夹".into()); }
    let path = root.join(&file_name);
    // create_new is atomic: an existing document can never be truncated by New File.
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)
        .map_err(|e| if e.kind() == std::io::ErrorKind::AlreadyExists { "同名文件已存在，请更换文件名".into() } else { format!("创建文件失败: {e}") })?;
    let title = Path::new(&file_name).file_stem().and_then(|s| s.to_str()).unwrap_or(&file_name);
    if let Err(error) = file.write_all(format!("# {title}\n\n").as_bytes()).and_then(|_| file.sync_all()) {
        drop(file);
        let cleanup = std::fs::remove_file(&path);
        return Err(format!("写入新文件失败: {error}{}", cleanup.err().map(|e| format!("；清理失败: {e}")).unwrap_or_default()));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_single_markdown_files() {
        for name in ["", "..", "../other", "a/b", "a\\b", "C:other", "bad\nname", "CON.md", "LPT1"] { assert!(markdown_name(name).is_err(), "{name}"); }
        assert_eq!(markdown_name("notes").unwrap(), "notes.md");
        assert_eq!(markdown_name("notes.MD").unwrap(), "notes.MD");
    }
    #[test]
    fn same_name_never_overwrites_existing_content() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("kira-md-create-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let file = create(&root, "notes").unwrap();
        std::fs::write(&file, "keep original").unwrap();
        assert!(create(&root, "notes").is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "keep original");
        assert!(create(&root, "../escape").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
