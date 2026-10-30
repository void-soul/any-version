use crate::commands::config::load_config;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use winreg::enums::*;
use winreg::RegKey;

mod models;
pub use models::*;
mod registry;
pub use registry::*;
mod path;
pub use path::*;
mod sdk;
pub use sdk::*;
mod backups;
pub use backups::*;
mod directories;
pub use directories::*;

#[cfg(test)]
#[path = "env/tests.rs"]
mod tests;
