use super::*;

/// 扫描数据目录状态
pub(super) fn build_data_dirs_status(
    def: &ProjectDef,
    active_install_root: Option<&str>,
) -> Vec<crate::commands::project::types::DataDirStatus> {
    use crate::commands::cache::{format_bytes, get_dir_size};
    use crate::commands::project::types::DataDirStatus;

    let config = load_config();
    let install_root = active_install_root.map(PathBuf::from);
    let mut statuses = Vec::new();

    for dir_def in &def.data_dirs {
        let resolved = crate::commands::service::resolve_data_dir(
            def,
            dir_def,
            &config,
            install_root.as_deref(),
        );
        let path = Path::new(&resolved.path);
        let exists = path.exists();
        let mut is_link = false;
        let mut real_target = String::new();

        if exists {
            if let Ok(metadata) = fs::symlink_metadata(path) {
                if metadata.file_type().is_symlink() {
                    if let Ok(eval_path) = fs::read_link(path) {
                        is_link = true;
                        real_target = eval_path.to_string_lossy().to_string();
                    } else if let Ok(eval_path) = fs::canonicalize(path) {
                        let canonical = eval_path.to_string_lossy().to_string();
                        let canonical_clean = canonical.trim_start_matches(r"\\?\").to_string();
                        if canonical_clean != path.to_string_lossy().to_string() {
                            is_link = true;
                            real_target = canonical_clean;
                        }
                    }
                }
            }
        }

        let size_path = if exists && is_link && !real_target.is_empty() {
            Path::new(&real_target)
        } else {
            path
        };
        let size = if exists {
            format_bytes(get_dir_size(size_path))
        } else {
            "0 B".to_string()
        };

        statuses.push(DataDirStatus {
            id: resolved.id,
            display_name: resolved.display_name,
            path: resolved.path,
            size,
            is_link,
            real_target,
            exists,
            kind: resolved.kind,
            source: Some(resolved.source),
        });
    }

    statuses
}
