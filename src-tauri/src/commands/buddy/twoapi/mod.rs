//! 2API：把 Buddy 的 WorkBuddy 登录态转成本地 OpenAI / Anthropic 兼容 API。
//!
//! 架构见 `docs/plans/2026-10-03-2api-rust-service-design.md`：
//! 协议转换复用 `crate::proxy`（axum + Anthropic/OpenAI/Responses 转换），
//! 本模块只负责「Buddy 凭据 → 上游鉴权头」与生命周期。
//!
//! **为什么内嵌而不是拉起 Python 服务**：原实现（workbuddy2api）取 at-rest 密钥时
//! 只查默认安装路径，本机 WorkBuddy 装在 `D:\sim-tool\WorkBuddy` → 密钥取不到 →
//! 所有对话请求 500，而 `/docs`、`/health`、`/v1/models` 全 200，极难定位。
//! 内嵌后密钥获取、账号切换、错误呈现都在我们自己的代码里，可测可控。

pub(crate) mod atrest;
pub(crate) mod credentials;
pub(crate) mod upstream;
