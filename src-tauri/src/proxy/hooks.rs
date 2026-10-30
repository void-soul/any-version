//! Application integration contract. Protocol code never imports business services.
use super::server::ProxyState;
use axum::Router;
use std::sync::{Arc, OnceLock};

pub struct UsageRecord<'a> {
    pub tool_id: &'a str,
    pub provider_id: &'a str,
    pub model: &'a str,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub duration_ms: u64,
    pub first_token_ms: u64,
}
pub trait ApplicationHooks: Send + Sync {
    fn extend_routes(&self, router: Router<ProxyState>) -> Router<ProxyState> {
        router
    }
    fn independently_authenticated(&self, _path: &str) -> bool {
        false
    }
    fn record_usage(&self, _record: UsageRecord<'_>) {}
    fn record_failure(&self, _tool: &str, _model: &str, _provider: &str, _class: &str) {}
}
struct NoHooks;
impl ApplicationHooks for NoHooks {}
static HOOKS: OnceLock<Arc<dyn ApplicationHooks>> = OnceLock::new();
pub fn install(hooks: Arc<dyn ApplicationHooks>) -> Result<(), Arc<dyn ApplicationHooks>> {
    HOOKS.set(hooks)
}
pub fn current() -> Arc<dyn ApplicationHooks> {
    HOOKS.get().cloned().unwrap_or_else(|| Arc::new(NoHooks))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standalone_contract_has_no_application_auth_exemptions() {
        let hooks = NoHooks;
        assert!(!hooks.independently_authenticated("/collab/agent/send"));
        assert!(!hooks.independently_authenticated("/v1/messages"));
        let _: Router<ProxyState> = hooks.extend_routes(Router::new());
        hooks.record_usage(UsageRecord {
            tool_id: "isolated",
            provider_id: "fake",
            model: "fake",
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            duration_ms: 0,
            first_token_ms: 0,
        });
    }
}
