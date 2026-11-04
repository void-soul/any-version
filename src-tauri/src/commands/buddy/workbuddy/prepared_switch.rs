use super::super::twoapi::atrest;
use super::*;

/// All fallible crypto/format work precedes session migration. No credential
/// reload or key subprocess is needed when this prepared generation commits.
pub(crate) struct PreparedSwitch {
    path: PathBuf,
    original: Option<Vec<u8>>,
    content: String,
    secret: String,
    token: String,
    marker: PathBuf,
    marker_existed: bool,
}

impl PreparedSwitch {
    pub(crate) fn prepare(platform: BuddyPlatform, account: &BuddyAccount) -> Result<Self, String> {
        let path = default_auth_file_path(platform).ok_or("无法定位 WorkBuddy 登录信息路径")?;
        let secret = atrest::fetch_secret()
            .map_err(|error| format!("无法取得 WorkBuddy 官方密钥，已停止切号: {error}"))?;
        Self::prepare_at(path, platform, account, secret)
    }

    fn prepare_at(
        path: PathBuf,
        platform: BuddyPlatform,
        account: &BuddyAccount,
        secret: String,
    ) -> Result<Self, String> {
        if account.access_token.trim().is_empty() {
            return Err("账号缺少 accessToken，已停止切号".into());
        }
        let original = match fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("读取 WorkBuddy 登录信息失败: {error}")),
        };
        let encrypted = if let Some(bytes) = &original {
            let existing: serde_json::Value = serde_json::from_slice(bytes)
                .map_err(|error| format!("现有 WorkBuddy 登录信息无效，已停止切号: {error}"))?;
            if !existing.is_object() {
                return Err("WorkBuddy 登录信息必须为对象".into());
            }
            encrypted_session::prepare_switched_session(account, platform, &existing, &secret)?
        } else {
            atrest::encrypt_session_fields(&build_auth_session(account, platform, None), &secret)?
        };
        let marker = logout_marker_path(&path);
        let marker_existed = marker.exists();
        Ok(Self {
            path,
            original,
            content: serde_json::to_string_pretty(&encrypted).map_err(|e| e.to_string())?,
            secret,
            token: account.access_token.clone(),
            marker,
            marker_existed,
        })
    }

    pub(crate) fn commit(self) -> Result<String, String> {
        match &self.original {
            Some(bytes) => {
                backup_auth_file(&self.path);
                let hash: [u8; 32] = Sha256::digest(bytes).into();
                if !store::write_atomic_if_hash_matches(&self.path, &hash, &self.content)? {
                    return Err("WorkBuddy 登录信息在预检后发生变化，已停止覆盖，请重试".into());
                }
            }
            None => {
                if self.path.exists() {
                    return Err("WorkBuddy 登录文件在预检后出现，已停止覆盖，请重试".into());
                }
                store::write_atomic(&self.path, &self.content)?;
            }
        }
        let bytes = fs::read(&self.path).map_err(|e| e.to_string())?;
        if bytes != self.content.as_bytes() {
            return Err("WorkBuddy 登录信息写后发生变化，未覆盖外部更新".into());
        }
        let written: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let token = written
            .get("auth")
            .and_then(|v| v.get("accessToken"))
            .ok_or("登录态缺少 accessToken")?;
        if !atrest::is_encrypted(token) || atrest::decrypt_field(token, &self.secret)? != self.token
        {
            return Err("WorkBuddy 写后校验未确认目标账号".into());
        }
        retire_logout_marker(&self.marker, self.marker_existed)?;
        Ok(format!(
            "已写入 WorkBuddy 目标账号（{}）",
            self.path.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_preflight_never_changes_original_and_commit_rejects_new_generation() {
        let dir = std::env::temp_dir().join(format!(
            "buddy-preflight-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("auth.json");
        fs::write(&path, "{bad").unwrap();
        let account = super::super::tests::sample_account();
        assert!(PreparedSwitch::prepare_at(
            path.clone(),
            BuddyPlatform::Workbuddy,
            &account,
            "fixture".into()
        )
        .is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{bad");
        fs::write(&path, "{}").unwrap();
        let plan = PreparedSwitch::prepare_at(
            path.clone(),
            BuddyPlatform::Workbuddy,
            &account,
            "fixture".into(),
        )
        .unwrap();
        fs::write(&path, "{\"external\":true}").unwrap();
        assert!(plan.commit().is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"external\":true}");
        let plan = PreparedSwitch::prepare_at(
            path.clone(),
            BuddyPlatform::Workbuddy,
            &account,
            "fixture".into(),
        )
        .unwrap();
        plan.commit().unwrap();
        let written: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            atrest::decrypt_field(&written["auth"]["accessToken"], "fixture").unwrap(),
            account.access_token
        );
        let missing = dir.join("new-auth.json");
        let plan = PreparedSwitch::prepare_at(
            missing.clone(),
            BuddyPlatform::Workbuddy,
            &account,
            "fixture".into(),
        )
        .unwrap();
        assert!(!missing.exists(), "preflight must not create login files");
        plan.commit().unwrap();
        assert!(missing.exists());
        for file in fs::read_dir(&dir).unwrap() {
            fs::remove_file(file.unwrap().path()).unwrap();
        }
        fs::remove_dir(dir).unwrap();
    }
}
