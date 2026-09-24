//! Buddy 模块数据模型：账号与平台。

use serde::{Deserialize, Serialize};

/// 支持的平台
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuddyPlatform {
    Workbuddy,
    CodebuddyCn,
}

impl BuddyPlatform {
    pub fn as_str(&self) -> &'static str {
        match self {
            BuddyPlatform::Workbuddy => "workbuddy",
            BuddyPlatform::CodebuddyCn => "codebuddy-cn",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "workbuddy" => Some(BuddyPlatform::Workbuddy),
            "codebuddy-cn" | "codebuddy_cn" => Some(BuddyPlatform::CodebuddyCn),
            _ => None,
        }
    }

    /// 账号库子目录名
    pub fn accounts_dir_name(&self) -> &'static str {
        match self {
            BuddyPlatform::Workbuddy => "buddy_workbuddy_accounts",
            BuddyPlatform::CodebuddyCn => "buddy_codebuddy_cn_accounts",
        }
    }

    /// 生成账号 ID 时使用的平台前缀（与各平台 build_account_from_local 一致）
    pub fn id_prefix(&self) -> &'static str {
        match self {
            BuddyPlatform::Workbuddy => "workbuddy",
            BuddyPlatform::CodebuddyCn => "codebuddy_cn",
        }
    }

    /// 客户端本地登录态文件名。
    ///
    /// 两个 WorkBuddy 版本共用 `CodeBuddyExtension/Data/Public/auth` 目录，
    /// WorkBuddy 系的登录文件名（CodeBuddy CN 不走该文件布局）。
    pub fn auth_file_name(&self) -> &'static str {
        match self {
            BuddyPlatform::Workbuddy => "workbuddy-desktop.info",
            BuddyPlatform::CodebuddyCn => "",
        }
    }

    /// 是否属于 WorkBuddy 系（共用 CodeBuddyExtension 登录文件布局）。
    pub fn is_workbuddy_family(&self) -> bool {
        matches!(self, BuddyPlatform::Workbuddy)
    }
}

#[cfg(test)]
mod tests {
    use super::BuddyPlatform;

    /// 平台标识的往返与隔离：未知串不能被当合法平台，两个平台的账号库目录
    /// 与 id 前缀必须不同，否则会互相覆盖数据。
    #[test]
    fn platform_round_trips_and_stays_isolated() {
        let wb = BuddyPlatform::from_str("workbuddy").expect("应识别 workbuddy");
        assert_eq!(wb.as_str(), "workbuddy");
        let cn = BuddyPlatform::from_str("codebuddy_cn").expect("应识别下划线写法");
        assert_eq!(cn, BuddyPlatform::CodebuddyCn);
        // 未知平台返回 None（含曾存在过的 workbuddy-ai，已移除）
        assert_eq!(BuddyPlatform::from_str("workbuddy-ai"), None);
        assert_eq!(BuddyPlatform::from_str("workbuddy-ai-x"), None);

        assert_ne!(wb.accounts_dir_name(), cn.accounts_dir_name());
        assert_ne!(wb.id_prefix(), cn.id_prefix());
    }

    #[test]
    fn workbuddy_family_uses_distinct_auth_file() {
        assert_eq!(BuddyPlatform::Workbuddy.auth_file_name(), "workbuddy-desktop.info");
        assert!(BuddyPlatform::Workbuddy.is_workbuddy_family());
        // CodeBuddy CN 不走该文件布局
        assert!(!BuddyPlatform::CodebuddyCn.is_workbuddy_family());
    }
}

/// 跨平台互导时按目标平台重新生成账号 ID 的种子：uid > email > "{prefix}_user"
pub(crate) fn account_id_seed(account: &BuddyAccount, prefix: &str) -> String {
    account
        .uid
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .or_else(|| {
            let email = account.email.trim();
            if email.is_empty() || email.eq_ignore_ascii_case("unknown") {
                None
            } else {
                Some(email.to_lowercase())
            }
        })
        .unwrap_or_else(|| format!("{}_user", prefix))
}

/// 账号（统一模型：workbuddy / codebuddy-cn 共用）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyAccount {
    pub id: String,
    pub platform: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterprise_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterprise_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,

    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,

    // ─── 用量/套餐（官方接口返回） ───
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dosage_notify_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dosage_notify_zh: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dosage_notify_en: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_type: Option<String>,

    /// 配额原始数据（{ dosage, payment, userResource } 组合）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_raw: Option<serde_json::Value>,
    /// 用量原始数据（userResource 响应）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_raw: Option<serde_json::Value>,
    /// 账号资料原始数据（login/account 响应）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_raw: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_updated_at: Option<i64>,

    // ─── 签到 ───
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checkin_time: Option<i64>,
    #[serde(default)]
    pub checkin_streak: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkin_rewards: Option<serde_json::Value>,

    /// 原始登录 JSON（切换时用于还原官方字段，如 workbuddy 的 accounts/allAccounts）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_raw: Option<serde_json::Value>,

    /// 各"过期时间列"的时间值（列 id → epoch 毫秒）；同邮箱账号在两平台间共享
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub expiry_times: std::collections::HashMap<String, i64>,

    pub created_at: i64,
    pub last_used: i64,
}

impl BuddyAccount {
    /// 展示名：nickname > email > uid > unknown
    pub fn display_name(&self) -> String {
        self.nickname
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .or_else(|| {
                let email = self.email.trim();
                if email.is_empty() || email == "unknown" {
                    None
                } else {
                    Some(email.to_string())
                }
            })
            .or_else(|| self.uid.clone())
            .unwrap_or_else(|| "unknown".to_string())
    }
}

/// 账号索引（轻量摘要，供列表秒开）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyAccountSummary {
    pub id: String,
    pub platform: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
}

impl BuddyAccount {
    pub fn summary(&self) -> BuddyAccountSummary {
        BuddyAccountSummary {
            id: self.id.clone(),
            platform: self.platform.clone(),
            email: self.email.clone(),
            nickname: self.nickname.clone(),
            created_at: self.created_at,
            last_used: self.last_used,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyAccountIndex {
    pub version: String,
    pub accounts: Vec<BuddyAccountSummary>,
}

impl Default for BuddyAccountIndex {
    fn default() -> Self {
        Self {
            version: "1.0".to_string(),
            accounts: Vec::new(),
        }
    }
}