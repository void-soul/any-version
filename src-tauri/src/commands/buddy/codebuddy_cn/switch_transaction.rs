//! CN 登录态的两资源提交：SQLite 持有写事务，身份文件失败时不提交 secret。
use std::path::{Path, PathBuf};

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("读取 CodeBuddy CN storage.json 失败: {e}")),
    }
}

pub(super) struct IdentityUpdate {
    path: PathBuf,
    original: Option<Vec<u8>>,
    target: String,
}

impl IdentityUpdate {
    pub(super) fn prepare(data_root: &Path, uid: &str) -> Result<Self, String> {
        if uid.trim().is_empty() {
            return Err("账号缺少 uid，无法同步 CodeBuddy CN 身份键".into());
        }
        let path = data_root.join("User/globalStorage/storage.json");
        let original = read_optional(&path)?;
        let mut value = match &original {
            Some(bytes) => serde_json::from_slice::<serde_json::Value>(bytes)
                .map_err(|e| format!("CodeBuddy CN storage.json 解析失败，已停止切号: {e}"))?,
            None => serde_json::json!({}),
        };
        let object = value
            .as_object_mut()
            .ok_or("CodeBuddy CN storage.json 必须是对象，已停止切号")?;
        object.insert("genie.userId".into(), uid.trim().into());
        let target = serde_json::to_string(&value).map_err(|e| e.to_string())?;
        Ok(Self {
            path,
            original,
            target,
        })
    }

    fn write(&self) -> Result<(), String> {
        if read_optional(&self.path)? != self.original {
            return Err("CodeBuddy CN 身份文件已被其他程序修改，已停止切号".into());
        }
        super::store::write_atomic(&self.path, &self.target)?;
        self.verify_target()
    }

    fn verify_target(&self) -> Result<(), String> {
        if read_optional(&self.path)?.as_deref() != Some(self.target.as_bytes()) {
            return Err("CodeBuddy CN 身份文件写后校验失败".into());
        }
        Ok(())
    }

    fn restore(&self) -> Result<(), String> {
        let current = read_optional(&self.path)?;
        if current == self.original {
            return Ok(());
        }
        if current.as_deref() != Some(self.target.as_bytes()) {
            return Err("身份文件已被其他程序修改，未覆盖外部更新".into());
        }
        match &self.original {
            Some(bytes) => {
                let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
                super::store::write_atomic(&self.path, text)?;
            }
            None => std::fs::remove_file(&self.path).map_err(|e| e.to_string())?,
        }
        if read_optional(&self.path)? != self.original {
            return Err("原身份文件恢复校验失败".into());
        }
        Ok(())
    }
}

/// `verify` 在两份目标数据可读、SQLite 尚未提交时执行，便于故障注入。
pub(super) fn commit(
    db: &Path,
    key: &str,
    encrypted: &str,
    identity: IdentityUpdate,
    verify: impl FnOnce(&rusqlite::Connection) -> Result<(), String>,
) -> Result<CommittedSwitch, String> {
    let mut conn = rusqlite::Connection::open(db).map_err(|e| e.to_string())?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| format!("无法开始 CodeBuddy CN 切号事务: {e}"))?;
    tx.execute(
        "CREATE TABLE IF NOT EXISTS ItemTable (key TEXT PRIMARY KEY, value TEXT)",
        [],
    )
    .map_err(|e| e.to_string())?;
    use rusqlite::OptionalExtension;
    let previous: Option<String> = tx
        .query_row("SELECT value FROM ItemTable WHERE key=?1", [key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT OR REPLACE INTO ItemTable (key,value) VALUES (?1,?2)",
        rusqlite::params![key, encrypted],
    )
    .map_err(|e| e.to_string())?;
    let result = identity
        .write()
        .and_then(|_| verify(&tx))
        .and_then(|_| identity.verify_target());
    if let Err(error) = result {
        let db_restore = tx.rollback().map_err(|e| e.to_string());
        return Err(recover_failure(error, db_restore, &identity));
    }
    if let Err(error) = tx.commit() {
        // Confirm the rollback instead of treating a failed COMMIT as a confirmed recovery.
        let restored = conn
            .query_row("SELECT value FROM ItemTable WHERE key=?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .map_err(|e| e.to_string())
            .and_then(|value| {
                if value == previous {
                    Ok(())
                } else {
                    Err("原 secret 恢复校验失败".into())
                }
            });
        return Err(recover_failure(
            format!("提交登录态失败: {error}"),
            restored,
            &identity,
        ));
    }
    Ok(CommittedSwitch {
        db: db.to_path_buf(),
        key: key.to_string(),
        previous,
        target: encrypted.to_string(),
        identity,
    })
}

pub(super) struct CommittedSwitch {
    db: PathBuf,
    key: String,
    previous: Option<String>,
    target: String,
    identity: IdentityUpdate,
}

impl std::fmt::Debug for CommittedSwitch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommittedSwitch").finish_non_exhaustive()
    }
}

impl CommittedSwitch {
    pub(super) fn verify_target(&self) -> Result<(), String> {
        let conn = rusqlite::Connection::open(&self.db).map_err(|e| e.to_string())?;
        let value: String = conn
            .query_row(
                "SELECT value FROM ItemTable WHERE key=?1",
                [&self.key],
                |row| row.get(0),
            )
            .map_err(|e| format!("CN 登录态延时校验失败: {e}"))?;
        if value != self.target {
            return Err("CN 登录态在切号期间被其他程序修改".into());
        }
        self.identity.verify_target()
    }

    /// Delayed confirmation may observe a client's newer generation. Never restore over it.
    pub(super) fn rollback(self, error: String) -> String {
        let restore = (|| {
            use rusqlite::OptionalExtension;
            let mut conn = rusqlite::Connection::open(&self.db).map_err(|e| e.to_string())?;
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
            let current: Option<String> = tx
                .query_row(
                    "SELECT value FROM ItemTable WHERE key=?1",
                    [&self.key],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if current.as_deref() != Some(self.target.as_str()) {
                return Err("secret 已被其他程序修改，未覆盖外部更新".into());
            }
            self.identity.verify_target()?;
            match &self.previous {
                Some(value) => tx.execute(
                    "UPDATE ItemTable SET value=?2 WHERE key=?1",
                    rusqlite::params![self.key, value],
                ),
                None => tx.execute("DELETE FROM ItemTable WHERE key=?1", [&self.key]),
            }
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            self.identity.restore()
        })();
        match restore {
            Ok(()) => format!("{error}；已恢复原登录态"),
            Err(reason) => format!("{error}；原登录态恢复未确认（{reason}）"),
        }
    }
}

fn recover_failure(error: String, db: Result<(), String>, identity: &IdentityUpdate) -> String {
    // Do not actively create an old-identity/new-secret pair if SQLite recovery is uncertain.
    let file = if db.is_ok() {
        identity.restore()
    } else {
        Err("数据库恢复未确认，未单独恢复身份文件".into())
    };
    failure(error, db, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);
    impl TestDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> (TestDir, PathBuf, PathBuf) {
        let mut nonce = [0u8; 8];
        getrandom::getrandom(&mut nonce).unwrap();
        let dir = TestDir(std::env::temp_dir().join(format!(
            "buddy-cn-txn-{}-{}",
            std::process::id(),
            u64::from_ne_bytes(nonce)
        )));
        std::fs::create_dir_all(dir.path()).unwrap();
        let db = dir.path().join("state.vscdb");
        let file = dir.path().join("User/globalStorage/storage.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, br#"{"genie.userId":"old","setting":{"keep":true}}"#).unwrap();
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE ItemTable (key TEXT PRIMARY KEY,value TEXT); INSERT INTO ItemTable VALUES ('secret','old-token');").unwrap();
        (dir, db, file)
    }

    fn secret(db: &Path) -> String {
        rusqlite::Connection::open(db)
            .unwrap()
            .query_row(
                "SELECT value FROM ItemTable WHERE key='secret'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn malformed_identity_fails_before_any_write() {
        let (dir, db, file) = fixture();
        for bytes in [b"{broken".as_slice(), b"[]", b"null"] {
            std::fs::write(&file, bytes).unwrap();
            assert!(IdentityUpdate::prepare(dir.path(), "new").is_err());
            assert_eq!(std::fs::read(&file).unwrap(), bytes);
            assert_eq!(secret(&db), "old-token");
        }
    }

    #[test]
    fn unreadable_identity_fails_closed() {
        let (dir, db, file) = fixture();
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(IdentityUpdate::prepare(dir.path(), "new").is_err());
        assert_eq!(secret(&db), "old-token");
    }

    #[test]
    fn successful_commit_preserves_other_settings() {
        let (dir, db, file) = fixture();
        let identity = IdentityUpdate::prepare(dir.path(), "new").unwrap();
        commit(&db, "secret", "new-token", identity, |_| Ok(())).unwrap();
        assert_eq!(secret(&db), "new-token");
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        assert_eq!(value["genie.userId"], "new");
        assert_eq!(value["setting"]["keep"], true);
    }

    #[test]
    fn failed_verification_restores_both_resources_exactly() {
        let (dir, db, file) = fixture();
        let before = std::fs::read(&file).unwrap();
        let identity = IdentityUpdate::prepare(dir.path(), "new").unwrap();
        let error = commit(&db, "secret", "new-token", identity, |_| {
            Err("injected failure".into())
        })
        .unwrap_err();
        assert!(error.contains("已恢复原登录态"));
        assert_eq!(secret(&db), "old-token");
        assert_eq!(std::fs::read(file).unwrap(), before);
    }

    #[test]
    fn external_update_before_write_is_not_overwritten() {
        let (dir, db, file) = fixture();
        let identity = IdentityUpdate::prepare(dir.path(), "new").unwrap();
        std::fs::write(&file, b"external update").unwrap();
        let error = commit(&db, "secret", "new-token", identity, |_| Ok(())).unwrap_err();
        assert!(error.contains("恢复未确认"));
        assert_eq!(secret(&db), "old-token");
        assert_eq!(std::fs::read(file).unwrap(), b"external update");
    }

    #[test]
    fn external_update_during_verification_is_not_overwritten() {
        let (dir, db, file) = fixture();
        let identity = IdentityUpdate::prepare(dir.path(), "new").unwrap();
        let error = commit(&db, "secret", "new-token", identity, |_| {
            std::fs::write(&file, b"external update").unwrap();
            Err("injected failure".into())
        })
        .unwrap_err();
        assert!(error.contains("恢复未确认"));
        assert_eq!(secret(&db), "old-token");
        assert_eq!(std::fs::read(file).unwrap(), b"external update");
    }

    #[test]
    fn missing_identity_is_removed_on_rollback() {
        let (dir, db, file) = fixture();
        std::fs::remove_file(&file).unwrap();
        let identity = IdentityUpdate::prepare(dir.path(), "new").unwrap();
        assert!(commit(&db, "secret", "new-token", identity, |_| Err(
            "failure".into()
        ))
        .is_err());
        assert!(!file.exists());
        assert_eq!(secret(&db), "old-token");
    }

    #[test]
    fn missing_identity_can_be_created() {
        let (dir, db, file) = fixture();
        std::fs::remove_file(&file).unwrap();
        let identity = IdentityUpdate::prepare(dir.path(), "new").unwrap();
        commit(&db, "secret", "new-token", identity, |_| Ok(())).unwrap();
        assert_eq!(secret(&db), "new-token");
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        assert_eq!(value["genie.userId"], "new");
    }

    #[test]
    fn delayed_failure_recovers_both_resources() {
        let (dir, db, file) = fixture();
        let before = std::fs::read(&file).unwrap();
        let identity = IdentityUpdate::prepare(dir.path(), "new").unwrap();
        let committed = commit(&db, "secret", "new-token", identity, |_| Ok(())).unwrap();
        assert!(committed
            .rollback("late failure".into())
            .contains("已恢复原登录态"));
        assert_eq!(secret(&db), "old-token");
        assert_eq!(std::fs::read(file).unwrap(), before);
    }

    #[test]
    fn delayed_failure_does_not_restore_over_external_secret() {
        let (dir, db, file) = fixture();
        let identity = IdentityUpdate::prepare(dir.path(), "new").unwrap();
        let committed = commit(&db, "secret", "new-token", identity, |_| Ok(())).unwrap();
        let target_identity = std::fs::read(&file).unwrap();
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute(
                "UPDATE ItemTable SET value='external-token' WHERE key='secret'",
                [],
            )
            .unwrap();
        assert!(committed
            .rollback("late failure".into())
            .contains("恢复未确认"));
        assert_eq!(secret(&db), "external-token");
        assert_eq!(std::fs::read(file).unwrap(), target_identity);
    }

    #[test]
    fn uncertain_database_recovery_does_not_restore_identity_alone() {
        let (dir, _, file) = fixture();
        let identity = IdentityUpdate::prepare(dir.path(), "new").unwrap();
        identity.write().unwrap();
        let before = std::fs::read(&file).unwrap();
        let error = recover_failure(
            "commit failure".into(),
            Err("rollback failed".into()),
            &identity,
        );
        assert!(error.contains("恢复未确认"));
        assert_eq!(std::fs::read(file).unwrap(), before);
    }

    #[test]
    fn commit_error_with_confirmed_old_database_restores_identity() {
        let (dir, db, file) = fixture();
        let before = std::fs::read(&file).unwrap();
        let identity = IdentityUpdate::prepare(dir.path(), "new").unwrap();
        let error = commit(&db, "secret", "new-token", identity, |conn| {
            // Fault injection: end the transaction before COMMIT, which must then fail.
            conn.execute_batch("ROLLBACK").map_err(|e| e.to_string())
        })
        .unwrap_err();
        assert!(error.contains("已恢复原登录态"));
        assert_eq!(secret(&db), "old-token");
        assert_eq!(std::fs::read(file).unwrap(), before);
    }

    #[test]
    fn rollback_error_does_not_create_old_identity_new_secret_pair() {
        let (dir, db, file) = fixture();
        let identity = IdentityUpdate::prepare(dir.path(), "new").unwrap();
        let error = commit(&db, "secret", "new-token", identity, |conn| {
            // Fault injection: ROLLBACK cannot recover a transaction committed externally.
            conn.execute_batch("COMMIT").unwrap();
            Err("injected verification failure".into())
        })
        .unwrap_err();
        assert!(error.contains("恢复未确认"));
        assert_eq!(secret(&db), "new-token");
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        assert_eq!(value["genie.userId"], "new");
    }

    #[test]
    fn delayed_identity_only_overwrite_is_not_a_success() {
        let (dir, db, file) = fixture();
        let identity = IdentityUpdate::prepare(dir.path(), "new").unwrap();
        let committed = commit(&db, "secret", "new-token", identity, |_| Ok(())).unwrap();
        assert!(committed.verify_target().is_ok());
        std::fs::write(&file, br#"{"genie.userId":"external"}"#).unwrap();
        let error = committed.verify_target().unwrap_err();
        assert!(committed.rollback(error).contains("恢复未确认"));
        assert_eq!(secret(&db), "new-token");
        assert_eq!(
            std::fs::read(file).unwrap(),
            br#"{"genie.userId":"external"}"#
        );
    }
}

fn failure(error: String, db: Result<(), String>, file: Result<(), String>) -> String {
    match (db, file) {
        (Ok(()), Ok(())) => format!("{error}；已恢复原登录态"),
        (db, file) => format!("{error}；原登录态恢复未确认（数据库：{db:?}；身份文件：{file:?}）"),
    }
}
