use super::*;

pub(super) fn same_settings(a: &BuddyAutoCheckinConfig, b: &BuddyAutoCheckinConfig) -> bool {
    a.enabled == b.enabled && a.start_time == b.start_time && a.end_time == b.end_time
}

/// UI owns settings; its stale schedule snapshot must never replace runtime progress.
pub(super) fn save_settings(path: &Path, incoming: &BuddyAutoCheckinConfig) -> Result<(), String> {
    let _guard = lock_storage()?;
    let mut latest = read_config_from_path(path)?.unwrap_or_default();
    latest.enabled = incoming.enabled;
    latest.start_time = incoming.start_time.clone();
    latest.end_time = incoming.end_time.clone();
    write_config_to_path(path, &latest)
}

pub(super) fn save_cycle_config(base: &BuddyAutoCheckinConfig, next: &BuddyAutoCheckinConfig) -> Result<BuddyAutoCheckinConfig, String> {
    save_cycle_at(&get_config_file_path(), base, next)
}

fn save_cycle_at(path: &Path, base: &BuddyAutoCheckinConfig, next: &BuddyAutoCheckinConfig) -> Result<BuddyAutoCheckinConfig, String> {
    let _guard = lock_storage()?;
    let mut latest = read_config_from_path(path)?.unwrap_or_default();
    let settings_match = same_settings(base, &latest);
    let before = base.account_schedules.clone().unwrap_or_default();
    let mut schedules = latest.account_schedules.clone().unwrap_or_default();
    for (id, result) in next.account_schedules.as_ref().into_iter().flatten() {
        if before.get(id) == Some(result) { continue; }
        if settings_match && schedules.get(id) == before.get(id) {
            schedules.insert(id.clone(), result.clone());
        } else if result.last_checked_date.as_deref() == Some(result.scheduled_date.as_str()) {
            // A completed in-flight request stays completed even if the user
            // disables scheduling or changes its window during the request.
            let current = schedules.entry(id.clone()).or_insert_with(|| result.clone());
            if current.scheduled_date == result.scheduled_date && current.last_checked_date != result.last_checked_date {
                current.last_checked_date = result.last_checked_date.clone();
                current.last_checked_time = result.last_checked_time.clone();
                current.retry_at_epoch = None;
                current.retry_count = 0;
                current.exhausted = false;
            }
        }
    }
    latest.account_schedules = (!schedules.is_empty()).then_some(schedules);
    write_config_to_path(path, &latest)?;
    Ok(latest)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, BuddyAutoCheckinConfig) {
        let root = crate::commands::foundation::get_base_dir().join(format!("checkin-merge-{}", random_u32_below(u32::MAX)));
        let config = BuddyAutoCheckinConfig { enabled:true, ..Default::default() };
        write_config_to_path(&root, &config).unwrap();
        (root, config)
    }
    fn completed(config: &BuddyAutoCheckinConfig) -> BuddyAutoCheckinConfig {
        let mut next = config.clone();
        let mut schedules = HashMap::new();
        mark_schedule_checked(&mut schedules, "fixture", "2026-11-03", 600);
        next.account_schedules = Some(schedules);
        next
    }
    #[test]
    fn old_ui_settings_preserve_new_runtime_completion() {
        let (path, mut ui) = fixture();
        save_cycle_at(&path, &ui, &completed(&ui)).unwrap();
        ui.start_time = "09:00".into();
        save_settings(&path, &ui).unwrap();
        let result = read_config_from_path(&path).unwrap().unwrap();
        assert_eq!(result.start_time, "09:00");
        assert_eq!(result.account_schedules.unwrap()["fixture"].last_checked_date.as_deref(), Some("2026-11-03"));
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn cycle_preserves_user_disable_and_new_window_but_records_inflight_success() {
        let (path, base) = fixture();
        let mut ui = base.clone(); ui.enabled = false; ui.end_time = "09:00".into();
        save_settings(&path, &ui).unwrap();
        let result = save_cycle_at(&path, &base, &completed(&base)).unwrap();
        assert!(!result.enabled);
        assert_eq!(result.end_time, "09:00");
        assert!(result.account_schedules.unwrap()["fixture"].last_checked_date.is_some());
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn stale_cycle_cannot_overwrite_newer_plan_or_corrupt_file() {
        let (path, base) = fixture();
        let mut newer = completed(&base);
        newer.account_schedules.as_mut().unwrap().get_mut("fixture").unwrap().scheduled_date = "2026-11-04".into();
        write_config_to_path(&path, &newer).unwrap();
        let result = save_cycle_at(&path, &base, &completed(&base)).unwrap();
        assert_eq!(result.account_schedules.unwrap()["fixture"].scheduled_date, "2026-11-04");
        fs::write(&path, "corrupt").unwrap();
        assert!(save_settings(&path, &base).is_err());
        assert!(save_cycle_at(&path, &base, &newer).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "corrupt");
        fs::remove_file(path).unwrap();
    }
}
