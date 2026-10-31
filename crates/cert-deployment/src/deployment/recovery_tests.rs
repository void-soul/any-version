use super::*;
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("receiver-recovery-{}", crate::random_hex(12)));
        std::fs::create_dir(&p).unwrap();
        std::fs::write(p.join("identity.crt"), "old").unwrap();
        Self(p)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn interrupted(root: &Root, phase: Phase, committed: bool) -> PathBuf {
    let mut d = Deployment::new(&root.0).unwrap();
    d.set_recovery_plan(vec![]);
    d.file("identity.crt", "new").unwrap();
    d.install().unwrap();
    if phase != Phase::Prepared {
        d.begin_reload().unwrap();
    }
    if phase == Phase::FilesRestored {
        d.restore_for_reload().unwrap();
    }
    if committed {
        d.commit_journal().unwrap();
    }
    let path = d.staging.clone();
    d.preserve = true;
    drop(d);
    path
}
#[test]
fn crash_at_each_durable_phase_restores_files_and_confirms_runtime() {
    for (phase, committed, reloads) in [
        (Phase::Prepared, false, 0),
        (Phase::Reloading, false, 1),
        (Phase::FilesRestored, false, 1),
        (Phase::Reloading, true, 0),
    ] {
        let root = Root::new();
        let stage = interrupted(&root, phase, committed);
        let mut calls = 0;
        recover_pending(&root.0, Some(&[]), &mut |_| {
            calls += 1;
            assert_eq!(
                std::fs::read_to_string(root.0.join("identity.crt")).unwrap(),
                "old"
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(calls, reloads);
        assert_eq!(
            std::fs::read_to_string(root.0.join("identity.crt")).unwrap(),
            if committed { "new" } else { "old" }
        );
        assert!(!stage.exists());
    }
}
#[test]
fn failed_recovery_keeps_journal_and_retry_does_not_remove_restored_files() {
    let root = Root::new();
    let stage = interrupted(&root, Phase::Reloading, false);
    assert!(recover_pending(&root.0, Some(&[]), &mut |_| Err(
        "fake service unavailable".into()
    ))
    .is_err());
    assert!(stage.join("journal.json").is_file());
    let journal: Journal =
        serde_json::from_slice(&std::fs::read(stage.join("journal.json")).unwrap()).unwrap();
    assert!(journal.phase == Phase::FilesRestored);
    assert!(!journal.committed);
    let mut calls = 0;
    recover_pending(&root.0, Some(&[]), &mut |_| {
        calls += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(
        std::fs::read_to_string(root.0.join("identity.crt")).unwrap(),
        "old"
    );
    assert!(!stage.exists());
}
#[test]
fn legacy_journal_requires_explicit_recovery_plan_and_remembers_it() {
    let root = Root::new();
    let stage = interrupted(&root, Phase::Prepared, false);
    std::fs::write(
        stage.join("journal.json"),
        r#"{"committed":false,"targets":[["identity.crt",true]]}"#,
    )
    .unwrap();
    assert!(recover_pending(&root.0, None, &mut |_| panic!("no plan")).is_err());
    assert!(stage.exists());
    assert!(recover_pending(&root.0, Some(&[]), &mut |_| Err("retry".into())).is_err());
    recover_pending(&root.0, None, &mut |plan| {
        assert!(plan.is_empty());
        Ok(())
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.0.join("identity.crt")).unwrap(),
        "old"
    );
}
#[test]
fn restored_new_target_is_not_resurrected_or_removed_twice() {
    let root = Root::new();
    let mut d = Deployment::new(&root.0).unwrap();
    d.set_recovery_plan(vec![]);
    d.file("first.crt", "new").unwrap();
    d.install().unwrap();
    d.begin_reload().unwrap();
    let stage = d.staging.clone();
    d.preserve = true;
    drop(d);
    assert!(recover_pending(&root.0, Some(&[]), &mut |_| Err("retry".into())).is_err());
    assert!(!root.0.join("first.crt").exists());
    assert!(stage.exists());
    recover_pending(&root.0, None, &mut |_| Ok(())).unwrap();
    assert!(!root.0.join("first.crt").exists());
}

#[test]
fn unconfirmed_reload_drop_keeps_recovery_evidence() {
    let root = Root::new();
    let stage;
    {
        let mut d = Deployment::new(&root.0).unwrap();
        d.set_recovery_plan(vec![]);
        d.file("identity.crt", "new").unwrap();
        d.install().unwrap();
        d.begin_reload().unwrap();
        stage = d.staging.clone();
    }
    assert!(stage.join("journal.json").exists());
    assert_eq!(
        std::fs::read_to_string(root.0.join("identity.crt")).unwrap(),
        "new"
    );
    recover_pending(&root.0, None, &mut |_| Ok(())).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.0.join("identity.crt")).unwrap(),
        "old"
    );
}
#[test]
fn crash_mid_install_does_not_reload_or_lose_untouched_target() {
    let root = Root::new();
    std::fs::write(root.0.join("second.crt"), "second-old").unwrap();
    let mut d = Deployment::new(&root.0).unwrap();
    d.set_recovery_plan(vec![]);
    d.file("identity.crt", "new").unwrap();
    d.file("second.crt", "second-new").unwrap();
    let journal = Journal {
        committed: false,
        phase: Phase::Prepared,
        reload_plan: Some(vec![]),
        targets: vec![("identity.crt".into(), true), ("second.crt".into(), true)],
    };
    write_journal(&d.staging, &journal).unwrap();
    std::fs::rename(&d.operations[0].target, &d.operations[0].backup).unwrap();
    std::fs::rename(&d.operations[0].staged, &d.operations[0].target).unwrap();
    d.preserve = true;
    drop(d);
    recover_pending(&root.0, None, &mut |_| panic!("no external reload began")).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.0.join("identity.crt")).unwrap(),
        "old"
    );
    assert_eq!(
        std::fs::read_to_string(root.0.join("second.crt")).unwrap(),
        "second-old"
    );
}

#[test]
fn windows_start_failure_keeps_recovery_pending_until_start_succeeds() {
    let root = Root::new();
    let options = crate::protocol::ReloadOptions {
        mode: "service",
        command: "",
        nginx_prefix: "",
        nginx_conf: "",
        service_name: "fake-service",
        post_script: None,
    };
    let plan = crate::reload_plan::plan(&options, crate::reload_plan::Platform::Windows).unwrap();
    let mut d = Deployment::new(&root.0).unwrap();
    d.set_recovery_plan(plan);
    d.file("identity.crt", "new").unwrap();
    d.install().unwrap();
    d.begin_reload().unwrap();
    let stage = d.staging.clone();
    d.preserve = true;
    drop(d);
    assert!(recover_pending(&root.0, None, &mut |plan| {
        crate::reload_plan::execute_with(plan, &|_| {}, |_| Err("fake stop/start failure".into()))
            .map(|_| ())
    })
    .is_err());
    assert!(stage.join("journal.json").is_file());
    recover_pending(&root.0, None, &mut |plan| {
        crate::reload_plan::execute_with(plan, &|_| {}, |cmd| {
            if cmd.args[0] == "stop" {
                Err("already stopped".into())
            } else {
                Ok("started".into())
            }
        })
        .map(|_| ())
    })
    .unwrap();
    assert!(!stage.exists());
    assert_eq!(
        std::fs::read_to_string(root.0.join("identity.crt")).unwrap(),
        "old"
    );
}
