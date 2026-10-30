//! MusicFree 插件宿主（常驻 Node 子进程桥）。
//!
//! 插件是**标准 CommonJS**（真实插件顶部就是 `require("axios") / require("cheerio") /
//! require("crypto-js")`），所以交给真实 Node 跑、配一份真实 `node_modules` 就够，
//! 不需要内嵌 JS 引擎，也不需要复刻 MusicFree 的宿主注入。
//!
//! 目录布局（`data_dir/music/plugins/`）：
//!
//! ```text
//! music/plugins/
//!   bridge.js        ← 宿主桥（随 app 资产释放，升级时自动更新）
//!   package.json     ← 依赖清单（同上）
//!   scripts/*.js     ← 插件本体。放子目录是为了：快照只挑这里、插件的相对引用不外溢
//!   node_modules/    ← 依赖（一次装、所有插件共享；不进快照，可重建）
//! ```
//!
//! `scripts/` 下能直接 `require("axios")`：Node 的解析会从该文件所在目录向上找
//! `node_modules`，因此 `music/plugins/node_modules` 正好命中。
//!
//! **node/npm 只认裸名，且只在「用户真实 PATH」里找** —— 见 [`user_path_entries`]。

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

mod layout;
pub use layout::*;
mod toolchain;
pub use toolchain::*;
mod sandbox;
pub use sandbox::*;
mod dependencies;
pub use dependencies::*;
mod bridge;
pub use bridge::*;
mod invocation;
pub use invocation::*;

#[cfg(test)]
#[path = "plugin_host/tests.rs"]
mod tests;
