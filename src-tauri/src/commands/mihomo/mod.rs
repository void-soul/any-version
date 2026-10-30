// mihomo 模块（对齐 clash-party 架构）
// 两层配置: AppConfig(应用配置) + ControledMihomoConfig(受控 mihomo 配置)
// Profiles / Overrides / Factory / Manager / Controller API
pub mod api;
pub mod backup;
pub mod config;
pub mod cron;
pub mod factory;
pub mod github;
pub mod groups;
pub mod manager;
pub mod misc;
pub mod netinfo;
pub mod smart;
pub mod ssid;
pub mod subparse;
pub mod substore;
pub mod traffic;

pub use manager::launch_core;

use crate::commands::hidden_cmd::hidden_cmd;
use crate::commands::mihomo::api::mihomo_api_raw;
use crate::commands::mihomo::config::*;
use crate::commands::mihomo::factory::{build_subscription_profile, deep_merge};
use crate::commands::mihomo::manager::*;
use chrono::{Datelike, Timelike};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};
use winreg::enums::*;
use winreg::RegKey;

mod state;
pub use state::*;
mod legacy;
pub use legacy::*;
mod lifecycle;
pub use lifecycle::*;
mod download;
pub use download::*;
mod core_commands;
pub use core_commands::*;
mod profiles;
pub use profiles::*;
mod groups_commands;
pub use groups_commands::*;
mod subscriptions;
pub use subscriptions::*;
mod overrides_commands;
pub use overrides_commands::*;
mod controller_commands;
pub use controller_commands::*;
mod upgrade;
pub use upgrade::*;
mod rules_commands;
pub use rules_commands::*;
mod tray;
pub use tray::*;
