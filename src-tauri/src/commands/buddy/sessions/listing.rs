use super::*;

pub(super) fn aggregate_sessions(
    raw_sessions: Vec<RawSession>,
    filter: &BuddySessionFilter,
    instance_id: &str,
    instance_name: &str,
) -> Vec<BuddySessionRecord> {
    let mut aggregated: HashMap<String, BuddySessionRecord> = HashMap::new();

    let keyword = filter
        .keyword
        .as_deref()
        .map(|k| k.to_lowercase())
        .unwrap_or_default();
    let status_filter = filter.status.as_deref().unwrap_or("");

    for raw in raw_sessions {
        if raw.is_deleted {
            continue;
        }
        if !status_filter.is_empty() && raw.status != status_filter {
            continue;
        }
        if !keyword.is_empty() {
            let title_lower = raw.title.to_lowercase();
            let cwd_lower = raw.cwd.to_lowercase();
            if !title_lower.contains(&keyword) && !cwd_lower.contains(&keyword) {
                continue;
            }
        }

        let conversation_id = raw.conversation_id.clone();
        let location = BuddySessionLocation {
            instance_id: instance_id.to_string(),
            instance_name: instance_name.to_string(),
        };

        aggregated
            .entry(conversation_id.clone())
            .and_modify(|existing| {
                if let Some(new_updated) = raw.updated_at {
                    if existing.updated_at.map_or(true, |old| new_updated > old) {
                        existing.updated_at = Some(new_updated);
                    }
                }
                if let Some(new_activity) = activity_at(&raw) {
                    if existing
                        .last_activity_at
                        .map_or(true, |old| new_activity > old)
                    {
                        existing.last_activity_at = Some(new_activity);
                    }
                }
                if !existing
                    .locations
                    .iter()
                    .any(|l| l.instance_id == instance_id)
                {
                    existing.locations.push(location.clone());
                }
            })
            .or_insert_with(|| {
                // 先算出活动时间：下面的结构体字面量会移动 raw 的字段
                let raw_activity = activity_at(&raw);
                BuddySessionRecord {
                    conversation_id: raw.conversation_id,
                    title: raw.title,
                    cwd: raw.cwd,
                    user_id: raw.user_id,
                    status: raw.status,
                    created_at: raw.created_at,
                    updated_at: raw.updated_at,
                    last_activity_at: raw_activity,
                    is_playground: raw.is_playground,
                    content_missing: false,
                    locations: vec![location],
                }
            });
    }

    let mut records: Vec<BuddySessionRecord> = aggregated.into_values().collect();
    // 排序：活动时间倒序，其次 created_at 倒序（对齐参考实现的 COALESCE 排序）
    records.sort_by(|a, b| {
        b.last_activity_at
            .unwrap_or(0)
            .cmp(&a.last_activity_at.unwrap_or(0))
            .then_with(|| b.created_at.unwrap_or(0).cmp(&a.created_at.unwrap_or(0)))
    });
    records
}

/// 列出会话（platform: "workbuddy" | "codebuddy-cn"）
pub fn list_sessions(
    platform: &str,
    filter: &BuddySessionFilter,
) -> Result<Vec<BuddySessionRecord>, String> {
    match platform {
        "workbuddy" => {
            let db_path = workbuddy_sessions_db_path()?;
            let mut records = aggregate_sessions(
                read_sessions_from_workbuddy_db(&db_path),
                filter,
                "workbuddy",
                "WorkBuddy",
            );
            // 正文存在性探测：仅在能定位到旧版扩展目录布局时生效（5.x 只留 DB 时跳过）
            let local = current_platform_uid(BuddyPlatform::Workbuddy).and_then(|uid| {
                let roots: Vec<PathBuf> =
                    crate::commands::buddy::session_transfer::workbuddy::select_extension_data_roots(&uid)
                        .ok()?
                        .into_iter()
                        .map(|(_, dir)| dir)
                        .collect();
                collect_local_conversation_ids_from_roots(&roots, &uid)
            });
            apply_content_probe(&mut records, local.as_ref());
            Ok(records)
        }
        "codebuddy-cn" | "codebuddy_cn" => {
            let db_path = codebuddy_cn_sessions_db_path()?;
            let mut records = aggregate_sessions(
                read_sessions_from_db(&db_path),
                filter,
                "default",
                "CodeBuddy CN",
            );
            let local = current_platform_uid(BuddyPlatform::CodebuddyCn).and_then(|uid| {
                let root = crate::commands::buddy::session_transfer::codebuddy::codebuddy_extension_data_dir().ok()?;
                collect_local_conversation_ids_from_roots(&[root], &uid)
            });
            apply_content_probe(&mut records, local.as_ref());
            Ok(records)
        }
        other => Err(format!("未知平台: {}", other)),
    }
}
