//! OAuth results remain retryable until the required account write is confirmed.
use super::*;
use std::future::Future;
use std::sync::{Arc, Weak};

type Key = (String, String);
const COMPLETED_TTL: i64 = 60;
#[derive(Clone)]
pub(super) struct PendingOAuthState {
    pub(super) login_id: String,
    pub(super) expires_at: i64,
    pub(super) state: String,
    pub(super) token_ready: Option<Value>,
    pub(super) ready: Option<BuddyAccount>,
}
struct Completed {
    account: BuddyAccount,
    expires_at: i64,
    generation: u64,
}
#[derive(Default)]
struct Sessions {
    pending: HashMap<String, PendingOAuthState>,
    generation: HashMap<String, u64>,
    completed: HashMap<Key, Completed>,
}
#[derive(Default)]
pub(super) struct OAuthSessions {
    inner: Mutex<Sessions>,
    gates: Mutex<HashMap<Key, Weak<tokio::sync::Mutex<()>>>>,
}
fn key(platform: BuddyPlatform, login_id: &str) -> Key {
    (platform.as_str().to_string(), login_id.to_string())
}
fn prune(inner: &mut Sessions, now: i64) {
    inner.pending.retain(|_, state| state.expires_at > now);
    inner.completed.retain(|_, state| state.expires_at > now);
}
impl OAuthSessions {
    pub(super) fn begin(
        &self,
        platform: BuddyPlatform,
        state: PendingOAuthState,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|_| "OAuth状态锁失败")?;
        prune(&mut inner, now_timestamp());
        *inner
            .generation
            .entry(platform.as_str().to_string())
            .or_default() += 1;
        inner.pending.insert(platform.as_str().to_string(), state);
        Ok(())
    }
    pub(super) fn state(
        &self,
        platform: BuddyPlatform,
        id: &str,
        now: i64,
    ) -> Result<PendingOAuthState, String> {
        let mut inner = self.inner.lock().map_err(|_| "OAuth状态锁失败")?;
        prune(&mut inner, now);
        inner
            .pending
            .get(platform.as_str())
            .filter(|state| state.login_id == id)
            .cloned()
            .ok_or_else(|| "登录已取消、过期或被新的登录请求替代".into())
    }
    pub(super) fn completed(
        &self,
        platform: BuddyPlatform,
        id: &str,
        now: i64,
    ) -> Result<Option<BuddyAccount>, String> {
        let mut inner = self.inner.lock().map_err(|_| "OAuth状态锁失败")?;
        prune(&mut inner, now);
        Ok(inner
            .completed
            .get(&key(platform, id))
            .map(|entry| entry.account.clone()))
    }
    pub(super) fn remember_token(
        &self,
        platform: BuddyPlatform,
        id: &str,
        data: Value,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|_| "OAuth状态锁失败")?;
        prune(&mut inner, now_timestamp());
        let state = inner
            .pending
            .get_mut(platform.as_str())
            .filter(|state| state.login_id == id)
            .ok_or("登录已取消或过期")?;
        state.token_ready = Some(data);
        Ok(())
    }
    pub(super) fn remember_ready(
        &self,
        platform: BuddyPlatform,
        id: &str,
        account: BuddyAccount,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|_| "OAuth状态锁失败")?;
        prune(&mut inner, now_timestamp());
        let state = inner
            .pending
            .get_mut(platform.as_str())
            .filter(|state| state.login_id == id)
            .ok_or("登录已取消或过期")?;
        state.ready = Some(account);
        Ok(())
    }
    pub(super) fn cancel(&self, platform: BuddyPlatform, id: Option<&str>) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|_| "OAuth状态锁失败")?;
        let invalidate = id.is_none()
            || inner
                .pending
                .get(platform.as_str())
                .is_some_and(|state| id == Some(state.login_id.as_str()))
            || id.is_some_and(|id| {
                inner
                    .completed
                    .get(&key(platform, id))
                    .is_some_and(|entry| {
                        Some(&entry.generation) == inner.generation.get(platform.as_str())
                    })
            });
        if invalidate {
            *inner
                .generation
                .entry(platform.as_str().to_string())
                .or_default() += 1;
        }
        if inner
            .pending
            .get(platform.as_str())
            .is_some_and(|state| id.is_none() || id == Some(state.login_id.as_str()))
        {
            inner.pending.remove(platform.as_str());
        }
        inner
            .completed
            .retain(|(p, login), _| p != platform.as_str() || id.is_some_and(|id| id != login));
        Ok(())
    }
    fn gate(
        &self,
        platform: BuddyPlatform,
        id: &str,
    ) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
        let mut gates = self.gates.lock().map_err(|_| "OAuth完成锁失败")?;
        gates.retain(|_, gate| gate.strong_count() > 0);
        let slot = gates.entry(key(platform, id)).or_default();
        let gate = slot
            .upgrade()
            .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(())));
        *slot = Arc::downgrade(&gate);
        Ok(gate)
    }
    fn persist<S>(
        &self,
        platform: BuddyPlatform,
        id: &str,
        save: &mut S,
    ) -> Result<BuddyAccount, String>
    where
        S: FnMut(BuddyAccount) -> Result<BuddyAccount, String>,
    {
        // Cancellation/new-login and commit are serialized. No await under this lock.
        let mut inner = self.inner.lock().map_err(|_| "OAuth状态锁失败")?;
        prune(&mut inner, now_timestamp());
        let account = inner
            .pending
            .get(platform.as_str())
            .filter(|state| state.login_id == id)
            .and_then(|state| state.ready.clone())
            .ok_or("登录已取消、过期或尚未就绪")?;
        let saved = save(account)?; // Failure preserves ready and pending for a disk-only retry.
        inner.pending.remove(platform.as_str());
        if inner.completed.len() >= 128 {
            if let Some(old) = inner
                .completed
                .iter()
                .min_by_key(|(_, entry)| entry.expires_at)
                .map(|(key, _)| key.clone())
            {
                inner.completed.remove(&old);
            }
        }
        let generation = *inner.generation.get(platform.as_str()).unwrap_or(&0);
        inner.completed.insert(
            key(platform, id),
            Completed {
                account: saved.clone(),
                expires_at: now_timestamp() + OAUTH_TIMEOUT_SECONDS as i64,
                generation,
            },
        );
        Ok(saved)
    }
    pub(super) async fn finish<P, PF, S, E, EF, M>(
        &self,
        platform: BuddyPlatform,
        id: &str,
        poll: P,
        mut save: S,
        enrich: E,
        merge: M,
    ) -> Result<BuddyAccount, String>
    where
        P: FnOnce() -> PF,
        PF: Future<Output = Result<BuddyAccount, String>>,
        S: FnMut(BuddyAccount) -> Result<BuddyAccount, String>,
        E: FnOnce(BuddyAccount) -> EF,
        EF: Future<Output = Result<BuddyAccount, String>>,
        M: FnOnce(&BuddyAccount, BuddyAccount) -> Result<Option<BuddyAccount>, String>,
    {
        let gate = self.gate(platform, id)?;
        let _guard = gate.lock().await;
        if let Some(saved) = self.completed(platform, id, now_timestamp())? {
            return Ok(saved);
        }
        let state = self.state(platform, id, now_timestamp())?;
        if state.ready.is_none() {
            self.remember_ready(platform, id, poll().await?)?;
        }
        let saved = self.persist(platform, id, &mut save)?;
        // Optional work never extends the synchronous commit lock across an await.
        let enriched = enrich(saved.clone()).await;
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return Ok(saved),
        };
        let current = inner
            .generation
            .get(platform.as_str())
            .copied()
            .unwrap_or(0);
        let valid = inner
            .completed
            .get(&key(platform, id))
            .is_some_and(|entry| entry.generation == current);
        if !valid {
            return Ok(saved);
        }
        let result = match enriched {
            Ok(enriched) => match merge(&saved, enriched) {
                Ok(Some(result)) => result,
                Ok(None) => saved,
                Err(error) => {
                    eprintln!("[Buddy OAuth] 可选补全保存失败，保留已登录账号: {}", error);
                    saved
                }
            },
            Err(error) => {
                eprintln!("[Buddy OAuth] 可选补全失败，保留已登录账号: {}", error);
                saved
            }
        };
        if let Some(entry) = inner.completed.get_mut(&key(platform, id)) {
            entry.account = result.clone();
            entry.expires_at = now_timestamp() + COMPLETED_TTL;
        }
        Ok(result)
    }
}
pub(super) fn oauth_sessions() -> &'static OAuthSessions {
    static SESSIONS: OnceLock<OAuthSessions> = OnceLock::new();
    SESSIONS.get_or_init(OAuthSessions::default)
}

pub async fn finish_oauth_login(platform: BuddyPlatform, id: &str) -> Result<BuddyAccount, String> {
    oauth_sessions()
        .finish(
            platform,
            id,
            || oauth_complete(platform, id),
            |account| store::upsert_account(platform, account),
            |saved| async move { enrich_oauth_metadata(platform, saved).await },
            |expected, enriched| {
                store::merge_optional_oauth_enrichment(platform, expected, enriched)
            },
        )
        .await
}

async fn enrich_oauth_metadata(
    platform: BuddyPlatform,
    mut account: BuddyAccount,
) -> Result<BuddyAccount, String> {
    let access = &account.access_token;
    let uid = account.uid.as_deref();
    let enterprise = account.enterprise_id.as_deref();
    let domain = account.domain.as_deref();
    let dosage = fetch_dosage_notify(platform, access, uid, enterprise, domain)
        .await
        .ok();
    let payment = fetch_payment_type(platform, access, uid, enterprise, domain)
        .await
        .ok();
    let resource =
        fetch_quota_resource_for_account(platform, access, uid, enterprise, domain).await?;
    let mut quota = account
        .quota_raw
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(value) = dosage {
        if let Some(data) = value.get("data") {
            account.dosage_notify_code = data.get("dosageNotifyCode").map(|code| {
                code.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| code.to_string())
            });
            account.dosage_notify_zh = data
                .get("dosageNotifyZh")
                .and_then(Value::as_str)
                .map(str::to_string);
            account.dosage_notify_en = data
                .get("dosageNotifyEn")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        quota.insert("dosage".into(), value);
    }
    if let Some(value) = payment {
        account.payment_type = value
            .get("data")
            .and_then(|data| data.get("paymentType"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or(account.payment_type);
        quota.insert("payment".into(), value);
    }
    quota.insert("userResource".into(), resource.clone());
    account.quota_raw = Some(Value::Object(quota));
    account.usage_raw = Some(resource);
    account.usage_updated_at = Some(chrono::Utc::now().timestamp_millis());
    account.quota_query_last_error = None;
    account.quota_query_last_error_at = None;
    Ok(account)
}

#[cfg(test)]
mod tests;
