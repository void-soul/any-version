//! refresh responsibility.
use super::*;

/// 账号资料 + 用量刷新结果（合并进账号）
#[derive(Debug, Clone)]
pub struct RefreshPayload {
    pub email: String,
    pub uid: Option<String>,
    pub nickname: Option<String>,
    pub enterprise_id: Option<String>,
    pub enterprise_name: Option<String>,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub expires_at: Option<i64>,
    pub domain: Option<String>,
    pub plan_type: Option<String>,
    pub dosage_notify_code: Option<String>,
    pub dosage_notify_zh: Option<String>,
    pub dosage_notify_en: Option<String>,
    pub payment_type: Option<String>,
    pub quota_raw: Option<Value>,
    pub profile_raw: Option<Value>,
    pub usage_raw: Option<Value>,
    pub status: Option<String>,
    pub status_reason: Option<String>,
}

pub(super) fn payload_to_account(
    platform: BuddyPlatform,
    p: RefreshPayload,
    base: Option<&BuddyAccount>,
) -> BuddyAccount {
    let now = chrono::Utc::now().timestamp();
    BuddyAccount {
        id: base.map(|b| b.id.clone()).unwrap_or_default(),
        platform: platform.as_str().to_string(),
        email: p.email,
        uid: p.uid,
        nickname: p.nickname,
        enterprise_id: p.enterprise_id,
        enterprise_name: p.enterprise_name,
        tags: base.and_then(|b| b.tags.clone()),
        access_token: p.access_token,
        refresh_token: p.refresh_token,
        token_type: p.token_type,
        expires_at: p.expires_at,
        domain: p.domain,
        plan_type: p.plan_type,
        dosage_notify_code: p.dosage_notify_code,
        dosage_notify_zh: p.dosage_notify_zh,
        dosage_notify_en: p.dosage_notify_en,
        payment_type: p.payment_type,
        quota_raw: p.quota_raw,
        usage_raw: p.usage_raw,
        profile_raw: p.profile_raw,
        status: p.status,
        status_reason: p.status_reason,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: Some(now),
        last_checkin_time: base.and_then(|b| b.last_checkin_time),
        checkin_streak: base.map(|b| b.checkin_streak).unwrap_or(0),
        checkin_rewards: base.and_then(|b| b.checkin_rewards.clone()),
        auth_raw: base.and_then(|b| b.auth_raw.clone()),
        expiry_times: base.map(|b| b.expiry_times.clone()).unwrap_or_default(),
        created_at: base.map(|b| b.created_at).unwrap_or(now),
        last_used: base.map(|b| b.last_used).unwrap_or(now),
    }
}

/// 刷新节奏：**临期（<1 天）或「今天还没成功刷过」**才刷，其余情况复用现有 token。
///
/// 这是两个参考实现交汇出来的语义：
/// - WorkDaddy `scripts/token-refresh.js::shouldRefreshAccessToken`（阈值 `DAY_MS`）+ `daemon.js::refreshAccountBackupToken`
///   的 `dailyKeepalive`（当天没成功刷过就保活一次）；
/// - workbuddy2api-hub `wb_accounts.py::_refresh_locked` 给每个账号一把 `_refresh_lock`，注释明说
///   「上游会轮换 refresh token，并发刷新会让第二个用掉第一个已消费的串」。
///
/// 为什么不是"每轮全量刷"（我们原先的做法）：refresh token 的寿命只有 ~30 天
/// （`refreshExpiresIn≈2591662s`），access token 是 28 天 —— 不刷会把号"放死"，
/// 每轮全量刷又制造大量并发/重复刷新。每天一次保活 + 临期刷新是最小充分频率。
///
/// 为什么不是"只在临期刷"：那就把「refresh 已经坏了」藏起来（2026-10-17 起 refresh 就在失败，
/// 账号靠未过期 access token 撑到 10-23 才整片 401）。每日保活保证坏号 24h 内必然暴露。
///
/// 实测补充（2026-10-23，单账号）：先刷一次拿到新 `refreshToken`，再拿**旧串**连刷两次都是 200
/// ⇒ 至少对这个会话没有可观测的 reuse detection。但单点观测不足以推翻参考实现的并发告警，
/// 所以仍按保守策略走，并由 [`refresh_warning`] 把失败亮出来。
pub(super) const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// 上一次刷新成功的时间（毫秒），存在 `auth_raw.lastRefreshTime` 里（与客户端同名字段）。
pub(super) fn last_refresh_ms(account: &BuddyAccount) -> Option<i64> {
    account
        .auth_raw
        .as_ref()
        .and_then(|v| v.get("lastRefreshTime"))
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
        })
        .map(|v| if v < 100_000_000_000 { v * 1000 } else { v })
}

pub(super) fn local_date_of(ms: i64) -> Option<chrono::NaiveDate> {
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.date_naive())
}

/// 纯判定（便于单测）：`expires_at` / 上次刷新时间 / 当前时间。
pub(super) fn refresh_due_at(
    expires_at: Option<i64>,
    last_refresh_ms: Option<i64>,
    now_ms: i64,
) -> bool {
    let near_expiry = match expires_at.filter(|ms| *ms > 0) {
        Some(ms) => ms - now_ms < DAY_MS,
        // 没有过期时间（老账号 / 第三方导入）→ 只能刷一次换到完整信息
        None => true,
    };
    if near_expiry {
        return true;
    }
    match last_refresh_ms {
        Some(ms) => local_date_of(ms) != local_date_of(now_ms),
        None => true,
    }
}

pub(super) fn token_refresh_due(account: &BuddyAccount, now_ms: i64) -> bool {
    refresh_due_at(account.expires_at, last_refresh_ms(account), now_ms)
}

/// 每个账号同一时刻只允许一个刷新在飞（对齐 hub `wb_accounts.py::_refresh_lock`）。
///
/// 理由见 hub 的注释：上游会轮换 refresh token，两个并发刷新里第二个用的是"已被消费"的串。
/// 我们这边并发来源很实在：面板「刷新全部」+ 托盘定时查询 + 用量页手动查询可以同时打到同一个账号。
pub(super) fn refresh_inflight() -> &'static Mutex<HashSet<String>> {
    static INFLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    INFLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 拿到租约才能刷；拿不到说明同一账号已有刷新在飞，本轮直接用现有 token（另一路会写回新 token）。
pub(super) struct RefreshLease(String);

impl Drop for RefreshLease {
    fn drop(&mut self) {
        if let Ok(mut set) = refresh_inflight().lock() {
            set.remove(&self.0);
        }
    }
}

pub(super) fn try_refresh_lease(key: &str) -> Option<RefreshLease> {
    let mut set = refresh_inflight().lock().ok()?;
    if !set.insert(key.to_string()) {
        return None;
    }
    Some(RefreshLease(key.to_string()))
}

/// 用现有账号刷新 token + 用量（参考 `refresh_payload_for_account_inner`）
/// 刷新只会改这四个字段，抽成小结构让「按节奏刷」和「401 后强制刷一次」共用同一段合并逻辑。
#[derive(Clone)]
pub(super) struct TokenState {
    pub(super) access: String,
    pub(super) refresh: Option<String>,
    pub(super) expires_at: Option<i64>,
    pub(super) domain: Option<String>,
}

impl TokenState {
    pub(super) fn from_account(account: &BuddyAccount) -> Self {
        Self {
            access: account.access_token.clone(),
            refresh: account.refresh_token.clone(),
            expires_at: account.expires_at,
            domain: account.domain.clone(),
        }
    }

    /// 把 refresh 响应合并进来（camelCase / snake_case 都认，缺字段就沿用原值）。
    pub(super) fn apply(&mut self, account: &BuddyAccount, data: &Value) {
        self.access = data
            .get("accessToken")
            .or_else(|| data.get("access_token"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| account.access_token.clone());
        self.refresh = data
            .get("refreshToken")
            .or_else(|| data.get("refresh_token"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| account.refresh_token.clone());
        self.expires_at = token_expiry_at(data).or(account.expires_at);
        self.domain = data
            .get("domain")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| account.domain.clone());
    }
}

pub(super) enum RefreshOutcome {
    Ok,
    /// 会话已被服务端吊销，刷新救不回来
    Revoked(String),
    /// 网络/其他原因，账号未必坏
    Failed(String),
}

/// 真正打 refresh 并把结果合进 `tokens`（调用方负责节奏与租约）。
pub(super) async fn do_refresh(
    platform: BuddyPlatform,
    account: &BuddyAccount,
    tokens: &mut TokenState,
) -> RefreshOutcome {
    let Some(refresh_tk) = account.refresh_token.as_deref() else {
        return RefreshOutcome::Failed("缺少 refresh token".to_string());
    };
    match refresh_token(
        platform,
        &tokens.access,
        refresh_tk,
        tokens.domain.as_deref(),
    )
    .await
    {
        Ok(data) => {
            tokens.apply(account, &data);
            RefreshOutcome::Ok
        }
        Err(e) if e.starts_with(AUTH_REVOKED_PREFIX) => RefreshOutcome::Revoked(e),
        Err(e) => RefreshOutcome::Failed(e),
    }
}

pub async fn refresh_payload_for_account(
    platform: BuddyPlatform,
    account: &BuddyAccount,
) -> Result<(BuddyAccount, Option<String>), String> {
    let mut tokens = TokenState::from_account(account);
    // 服务端已吊销登录态（刷新也救不回来）时的原因，用于置账号状态与界面提示
    let mut auth_revoked: Option<String> = None;

    // 刷新失败但不属于"被吊销"时留个记录，配额也失败就一起报出来 ——
    // 否则 refresh 长期失败会被 eprintln 吞掉（2026-10-17~10-23 就是这么瞎了 6 天）。
    let mut refresh_warning: Option<String> = None;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut refreshed_at: Option<i64> = None;
    // 同一账号已有刷新在飞时不重复刷（并发用同一个 refreshToken 是参考实现明确警告过的坑）
    let lease_key = account.uid.clone().unwrap_or_else(|| account.id.clone());

    // 第一跳：按节奏刷（临期 / 今天还没刷过）。手动点"刷新"也走这里 ——
    // 面板上的"刷新"刷新的是**用量数据**，令牌刷新与否由节奏决定，避免用户狂点把节奏打乱。
    if token_refresh_due(account, now_ms) {
        if let Some(_lease) = try_refresh_lease(&lease_key) {
            match do_refresh(platform, account, &mut tokens).await {
                RefreshOutcome::Ok => {
                    // 刷新成功 ⇒ 之前的「已吊销」结论作废，账号恢复可用
                    auth_revoked = None;
                    refreshed_at = Some(now_ms);
                }
                RefreshOutcome::Revoked(e) => {
                    eprintln!("[Buddy] 账号 {} 登录态已被服务端吊销: {}", account.id, e);
                    auth_revoked = Some(e);
                }
                RefreshOutcome::Failed(e) => {
                    eprintln!("[Buddy] Token 刷新失败，将使用现有 token 查询配额: {}", e);
                    refresh_warning = Some(e);
                }
            }
        }
    }

    let dosage = fetch_dosage_notify(
        platform,
        &tokens.access,
        account.uid.as_deref(),
        account.enterprise_id.as_deref(),
        tokens.domain.as_deref(),
    )
    .await
    .ok();

    let payment = fetch_payment_type(
        platform,
        &tokens.access,
        account.uid.as_deref(),
        account.enterprise_id.as_deref(),
        tokens.domain.as_deref(),
    )
    .await
    .ok();

    let mut quota_refresh_error: Option<String> = None;
    let fetch_quota = |tokens: &TokenState| {
        let access = tokens.access.clone();
        let domain = tokens.domain.clone();
        async move {
            fetch_quota_resource_for_account(
                platform,
                &access,
                account.uid.as_deref(),
                account.enterprise_id.as_deref(),
                domain.as_deref(),
            )
            .await
        }
    };
    let mut user_resource = match fetch_quota(&tokens).await {
        Ok(payload) => Some(payload),
        Err(err) => {
            eprintln!("[Buddy] 刷新 user_resource 失败: {}", err);
            if err.starts_with(AUTH_REVOKED_PREFIX) && auth_revoked.is_none() {
                auth_revoked = Some(err.clone());
            }
            quota_refresh_error = Some(err);
            None
        }
    };

    // 第二跳：用量接口判 401 而本轮**还没刷过**令牌 → 强制刷一次再重试一次。
    // 这是"手动刷新"真正的意义所在：能救的号当场救回来，救不回来才判吊销。
    // 第一跳已经刷过还 401 ⇒ 刷新过的令牌照样被拒，直接认定吊销，不再浪费请求。
    if user_resource.is_none()
        && auth_revoked.is_some()
        && refreshed_at.is_none()
        && account.refresh_token.is_some()
    {
        if let Some(_lease) = try_refresh_lease(&lease_key) {
            match do_refresh(platform, account, &mut tokens).await {
                RefreshOutcome::Ok => match fetch_quota(&tokens).await {
                    Ok(payload) => {
                        user_resource = Some(payload);
                        quota_refresh_error = None;
                        auth_revoked = None;
                        refreshed_at = Some(now_ms);
                    }
                    Err(err) => {
                        eprintln!("[Buddy] 强制刷新后重试 user_resource 仍失败: {}", err);
                        quota_refresh_error = Some(err);
                    }
                },
                RefreshOutcome::Revoked(e) => {
                    eprintln!("[Buddy] 账号 {} 登录态已被服务端吊销: {}", account.id, e);
                    auth_revoked = Some(e);
                }
                RefreshOutcome::Failed(e) => {
                    eprintln!("[Buddy] 强制刷新失败: {}", e);
                    refresh_warning = Some(e);
                }
            }
        }
    }

    // 失败原因收口：配额失败 + 刷新也失败 = 账号真用不了，两个原因一起给，别只报"解析失败"
    if let Some(err) = quota_refresh_error.take() {
        let mut msg = err
            .strip_prefix(AUTH_REVOKED_PREFIX)
            .unwrap_or(&err)
            .to_string();
        if !err.starts_with(AUTH_REVOKED_PREFIX) {
            if let Some(warn) = &refresh_warning {
                msg = format!("{}（令牌刷新也失败：{}）", msg, warn);
            }
        }
        quota_refresh_error = Some(msg);
    }

    let dosage_data = dosage.as_ref().and_then(|v| v.get("data"));
    let dosage_notify_code = dosage_data
        .and_then(|d| d.get("dosageNotifyCode"))
        .map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => v.to_string(),
        });
    let dosage_notify_zh = dosage_data
        .and_then(|d| d.get("dosageNotifyZh"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let dosage_notify_en = dosage_data
        .and_then(|d| d.get("dosageNotifyEn"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let payment_data = payment.as_ref().and_then(|v| v.get("data"));
    let payment_type = payment_data
        .and_then(|d| {
            d.as_str().map(|s| s.to_string()).or_else(|| {
                d.get("paymentType")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
        })
        .or_else(|| account.payment_type.clone());

    let mut combined_quota = account
        .quota_raw
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(d) = &dosage {
        combined_quota.insert("dosage".to_string(), d.clone());
    }
    if let Some(p) = &payment {
        combined_quota.insert("payment".to_string(), p.clone());
    }
    if let Some(r) = &user_resource {
        combined_quota.insert("userResource".to_string(), r.clone());
    }

    let quota_raw = if combined_quota.is_empty() {
        account.quota_raw.clone()
    } else {
        Some(Value::Object(combined_quota))
    };

    let final_email =
        normalize_non_empty(Some(account.email.as_str())).unwrap_or_else(|| account.email.clone());

    // 登录态被吊销 → 落成账号状态，界面据此显示"需重新登录"、调度器据此跳过，
    // 而不是每次刷新都甩一个裸 401 让人以为是网络问题。
    let (status, status_reason) = match (&auth_revoked, account.status.as_deref()) {
        (Some(msg), _) => (
            Some(STATUS_AUTH_REVOKED.to_string()),
            Some(
                msg.strip_prefix(AUTH_REVOKED_PREFIX)
                    .unwrap_or(msg)
                    .to_string(),
            ),
        ),
        (None, Some(prev)) if prev == STATUS_AUTH_REVOKED => (Some("normal".to_string()), None),
        (None, _) => (account.status.clone(), account.status_reason.clone()),
    };

    let payload = RefreshPayload {
        email: final_email,
        uid: account.uid.clone(),
        nickname: account.nickname.clone(),
        enterprise_id: account.enterprise_id.clone(),
        enterprise_name: account.enterprise_name.clone(),
        access_token: tokens.access,
        refresh_token: tokens.refresh,
        token_type: account.token_type.clone(),
        expires_at: tokens.expires_at,
        domain: tokens.domain,
        plan_type: account.plan_type.clone(),
        dosage_notify_code,
        dosage_notify_zh,
        dosage_notify_en,
        payment_type,
        quota_raw,
        profile_raw: account.profile_raw.clone(),
        usage_raw: user_resource.or_else(|| account.usage_raw.clone()),
        status,
        status_reason,
    };

    let mut next = payload_to_account(platform, payload, Some(account));
    if let Some(ms) = refreshed_at {
        // 记下刷新成功时间：既是「每日保活」的判据，也让诊断能看出上次刷新是什么时候
        let mut raw = next.auth_raw.take().unwrap_or_else(|| json!({}));
        if let Some(obj) = raw.as_object_mut() {
            obj.insert("lastRefreshTime".to_string(), json!(ms));
        }
        next.auth_raw = Some(raw);
    }
    Ok((next, quota_refresh_error))
}

/// 用裸 token 构建账号（拉取资料 + 用量），参考 `build_payload_from_token`
pub async fn build_payload_from_token(
    platform: BuddyPlatform,
    access_token: &str,
) -> Result<BuddyAccount, String> {
    let client = build_client()?;
    let url = format!("{}{}/accounts", api_endpoint(platform), API_PREFIX);

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| format!("请求 accounts 失败: {}", e))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 accounts 响应失败: {}", e))?;

    let accounts = body
        .get("data")
        .and_then(|d| d.get("accounts"))
        .and_then(|a| a.as_array());

    let account_data = accounts
        .and_then(|arr| {
            arr.iter().find(|a| {
                a.get("lastLogin")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            })
        })
        .or_else(|| accounts.and_then(|arr| arr.first()))
        .cloned()
        .unwrap_or(json!({}));

    let uid = account_data
        .get("uid")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let nickname = account_data
        .get("nickname")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let email = account_data
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let enterprise_id = account_data
        .get("enterpriseId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let enterprise_name = account_data
        .get("enterpriseName")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let dosage = fetch_dosage_notify(
        platform,
        access_token,
        uid.as_deref(),
        enterprise_id.as_deref(),
        None,
    )
    .await
    .ok();
    let payment = fetch_payment_type(
        platform,
        access_token,
        uid.as_deref(),
        enterprise_id.as_deref(),
        None,
    )
    .await
    .ok();
    let user_resource = fetch_user_resource_with_access_token_default(
        platform,
        access_token,
        uid.as_deref(),
        enterprise_id.as_deref(),
        None,
    )
    .await
    .ok();

    let dosage_data = dosage.as_ref().and_then(|v| v.get("data"));
    let dosage_notify_code = dosage_data
        .and_then(|d| d.get("dosageNotifyCode"))
        .map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => v.to_string(),
        });
    let dosage_notify_zh = dosage_data
        .and_then(|d| d.get("dosageNotifyZh"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let dosage_notify_en = dosage_data
        .and_then(|d| d.get("dosageNotifyEn"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let payment_data = payment.as_ref().and_then(|v| v.get("data"));
    let payment_type = payment_data.and_then(|d| {
        d.as_str().map(|s| s.to_string()).or_else(|| {
            d.get("paymentType")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
    });

    let mut combined_quota = serde_json::Map::new();
    if let Some(payload) = dosage.as_ref() {
        combined_quota.insert("dosage".to_string(), payload.clone());
    }
    if let Some(payload) = payment.as_ref() {
        combined_quota.insert("payment".to_string(), payload.clone());
    }
    if let Some(payload) = user_resource.as_ref() {
        combined_quota.insert("userResource".to_string(), payload.clone());
    }
    let quota_raw = if combined_quota.is_empty() {
        None
    } else {
        Some(Value::Object(combined_quota))
    };

    // uid / email 全空时不要落库：派生 id 只能退化成 `{platform}_user`，
    // 多个这种账号会互相覆盖，而且缺 uid 的账号上游根本用不了（X-User-Id）。
    if uid.is_none() && email.trim().is_empty() {
        return Err(
            "接口未返回账号信息（uid / email 均为空）：token 可能无效，或 /v2/plugin/accounts 契约变了"
                .to_string(),
        );
    }

    let email_final = if email.is_empty() {
        nickname
            .clone()
            .or_else(|| uid.clone())
            .unwrap_or_else(|| "unknown".to_string())
    } else {
        email
    };

    let now = chrono::Utc::now().timestamp();
    Ok(BuddyAccount {
        // id 由 store::upsert_account 按 uid/email 派生（{platform}_{md5}）
        id: String::new(),
        platform: platform.as_str().to_string(),
        email: email_final,
        uid,
        nickname,
        enterprise_id,
        enterprise_name,
        tags: None,
        access_token: access_token.to_string(),
        refresh_token: None,
        token_type: Some("Bearer".to_string()),
        expires_at: None,
        domain: None,
        plan_type: None,
        dosage_notify_code,
        dosage_notify_zh,
        dosage_notify_en,
        payment_type,
        quota_raw,
        usage_raw: user_resource,
        profile_raw: Some(account_data),
        status: Some("normal".to_string()),
        status_reason: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: Some(now),
        last_checkin_time: None,
        checkin_streak: 0,
        checkin_rewards: None,
        auth_raw: None,
        expiry_times: Default::default(),
        created_at: 0,
        last_used: 0,
    })
}
