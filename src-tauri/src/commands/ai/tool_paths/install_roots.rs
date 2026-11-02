//! Resolve declared executable tails under registry install roots without running commands.
use std::path::{Path, PathBuf};

pub(super) fn roots(location: &str, icon: &str, uninstall: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let location = PathBuf::from(location.trim().trim_matches('"'));
    if location.is_absolute() && location.is_dir() {
        out.push(location);
    }
    for text in [icon, uninstall] {
        let text = text.trim();
        let executable = if let Some(quoted) = text.strip_prefix('"') {
            quoted.split('"').next().unwrap_or("")
        } else {
            let lower = text.to_ascii_lowercase();
            match lower.find(".exe") {
                Some(end) => &text[..end + 4],
                None => text.split(',').next().unwrap_or("").trim(),
            }
        };
        let path = Path::new(executable);
        if path.is_absolute() {
            if let Some(parent) = path.parent().filter(|p| p.is_dir()) {
                if !out.iter().any(|root| root == parent) {
                    out.push(parent.to_path_buf());
                }
            }
        }
    }
    out
}

pub(super) fn resolve(roots: &[PathBuf], declared: &[String]) -> Option<PathBuf> {
    // All declared runtime tails precede all top-level launcher names.
    let mut tails = Vec::new();
    let mut names = Vec::new();
    for raw in declared {
        let normalized = raw.replace('\\', "/");
        let parts: Vec<_> = normalized.split('/').collect();
        let Some(name) = parts
            .last()
            .filter(|name| name.to_ascii_lowercase().ends_with(".exe"))
        else {
            continue;
        };
        if parts.iter().any(|part| *part == "..") {
            continue;
        }
        if parts.len() >= 2 {
            let parent = parts[parts.len() - 2];
            if !parent.is_empty() && !parent.contains(['%', '*', ':']) && parent != "." {
                tails.push(PathBuf::from(parent).join(name));
            }
        }
        names.push(PathBuf::from(name));
    }
    for candidate in tails.iter().chain(names.iter()) {
        for root in roots {
            let path = root.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_install_roots_prefer_declared_runtime_and_fallback_to_uninstaller() {
        let root = std::env::temp_dir().join(format!(
            "kira-custom-install-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("app")).unwrap();
        let launcher = root.join("Client.exe");
        let runtime = root.join("app/Client.exe");
        for path in [
            &launcher,
            &runtime,
            &root.join("uninstall.exe"),
            &root.join("icon.ico"),
        ] {
            std::fs::write(path, b"fixture").unwrap();
        }
        let uninstall = format!("\"{}\" /S", root.join("uninstall.exe").display());
        let icon = format!("\"{}\",0", root.join("icon.ico").display());
        assert_eq!(roots("", "", &uninstall), vec![root.clone()]);
        assert_eq!(roots("", &icon, ""), vec![root.clone()]);
        assert_eq!(roots(&root.to_string_lossy(), "", ""), vec![root.clone()]);
        let found_roots = roots("", &icon, &uninstall);
        assert!(found_roots.contains(&root));
        let declared = vec![
            "C:/default/Client/app/Client.exe".to_string(),
            "C:/default/Client/Client.exe".to_string(),
        ];
        assert_eq!(resolve(&found_roots, &declared), Some(runtime.clone()));
        std::fs::remove_file(runtime).unwrap();
        assert_eq!(resolve(&found_roots, &declared), Some(launcher));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn install_roots_do_not_execute_or_guess_unrelated_binaries() {
        assert!(roots("", "", "msiexec.exe /x {id}").is_empty());
        assert!(resolve(&[std::env::temp_dir()], &["../uninstall.exe".into()]).is_none());
    }
}
