//! 远程连接管理：RDP 与 SSH 共用一张连接表（FinalShell 100/101 同构）。
//! FinalShell 密文未编辑时逐字节透传，解密能力在 finalshell_crypto。
mod commands;
pub mod credentials;
pub mod finalshell;
pub mod finalshell_crypto;
pub mod models;
pub mod store;

pub use commands::*;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "credentials_tests.rs"]
mod credentials_tests;
