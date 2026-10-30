//! 证书管理（Q-005）：免费 ACME 证书申请 + 多节点部署 + 应用内调度。
//!
//! 架构：
//! - 数据模型（Certificate / DeployNode / Credential）持久化在 `get_data_dir()/certs/` 下三个 JSON 文件。
//! - 验证方式：DNS-01（lego --dns <provider>）与 HTTP-01（lego --http）；CNAME 委托需公网权威 DNS，暂不支持。
//! - 域名列表 → 部署方式：`Certificate.deploy_links` 保存关联；部署方式本身不保存域名。
//! - 部署：Windows 接收端 / API / Docker / SSH / Webhook / 阿里云 SSL / 腾讯云 SSL / 七牛 SSL。
//! - 调度：应用内 tokio 定时器，扫描到期并触发申请/部署。

use aes_gcm::aead::Aead;
use aes_gcm::KeyInit;
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use chrono::{DateTime, Duration, TimeZone, Utc};
use hmac::Hmac;
use hmac::Mac;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use tauri::Emitter;
use tauri::Manager;

#[allow(dead_code)]
type HmacSha1 = Hmac<Sha1>;

mod models;
pub use models::*;
mod store;
pub use store::*;
mod credentials;
pub use credentials::*;
mod lego;
pub use lego::*;
mod deploy_ssh;
pub use deploy_ssh::*;
mod deploy_baota;
pub use deploy_baota::*;
mod deploy_aliyun;
pub use deploy_aliyun::*;
mod deploy_tencent;
pub use deploy_tencent::*;
mod deploy_onepanel;
pub use deploy_onepanel::*;
mod deploy_synology;
pub use deploy_synology::*;
mod deploy_upyun;
pub use deploy_upyun::*;
mod deploy_cloud;
pub use deploy_cloud::*;
mod pipeline;
pub use pipeline::*;
mod validity;
pub use validity::*;
mod details;
pub use details::*;
mod monitor;
pub use monitor::*;
mod notify;
pub use notify::*;
mod scheduler;
pub use scheduler::*;
mod commands;
pub use commands::*;
mod monitor_commands;
pub use monitor_commands::*;

#[cfg(test)]
mod tests;
