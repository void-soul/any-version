//! 内建 SSH 客户端（阶段 1：shell；阶段 2：SFTP；阶段 3：转发/密钥/主机信任）。
mod commands;
pub mod forward;
pub mod forward_runtime;
pub mod host_keys;
pub mod keys;
pub mod monitor;
pub mod procinfo;
pub mod session;
pub mod sftp;
pub mod sftp_path;
pub mod sftp_archive;
pub mod sftp_resume;
pub mod snippets;
pub mod sysinfo;
pub mod target;

pub use commands::*;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "sftp_tests.rs"]
mod sftp_tests;

#[cfg(test)]
#[path = "forward_tests.rs"]
mod forward_tests;

#[cfg(test)]
#[path = "auth_tests.rs"]
mod auth_tests;

#[cfg(test)]
#[path = "resume_tests.rs"]
mod resume_tests;

#[cfg(test)]
#[path = "sysinfo_tests.rs"]
mod sysinfo_tests;
