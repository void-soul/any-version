//! Compatibility facade for the shared AI transport.
pub use crate::proxy::channel::*;
impl ChannelProvider for super::models::AiProvider {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn api_key(&self) -> &str {
        &self.api_key
    }
    fn openai_url(&self) -> &str {
        &self.openai_url
    }
}
