//! 配置读写。
//!
//! 仍然是同一份 `cert-receiver.toml`（GUI 与命令行共用），只是多了「找不到就生成一份」
//! 的行为 —— 以前双击 exe 因为缺配置直接 `exit(1)`，控制台一闪就关，看起来像打不开。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = "cert-receiver.toml";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// 监听地址，形如 `127.0.0.1:9000`
    pub listen_addr: String,
    pub token: String,
    pub output_dir: String,
    /// cert（按证书 ID）| domain（按域名）| both | flat（旧版平铺）
    #[serde(default = "default_layout")]
    pub layout: String,
    /// API / Docker 拉取接口的共享密钥；为空则关闭拉取接口
    #[serde(default)]
    pub api_key: String,
    /// none | command | nginx | service
    #[serde(default = "default_reload_mode")]
    pub reload_mode: String,
    #[serde(default)]
    pub reload_command: String,
    #[serde(default)]
    pub nginx_prefix: String,
    #[serde(default)]
    pub nginx_conf: String,
    #[serde(default)]
    pub service_name: String,
    /// 兼容旧配置：收到证书后执行的脚本（优先于 reload_mode）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_script: Option<String>,
}

fn default_layout() -> String {
    "cert".to_string()
}
fn default_reload_mode() -> String {
    "none".to_string()
}

impl Config {
    /// 缺省配置：给第一次运行用，token/api_key 随机生成。
    pub fn fresh() -> Self {
        Self {
            listen_addr: "127.0.0.1:9000".to_string(),
            token: random_hex(16),
            output_dir: "C:\\certs".to_string(),
            layout: default_layout(),
            api_key: random_hex(16),
            reload_mode: default_reload_mode(),
            reload_command: String::new(),
            nginx_prefix: String::new(),
            nginx_conf: String::new(),
            service_name: String::new(),
            post_script: None,
        }
    }

    /// 保存前的校验：错误信息直接显示在界面上，所以都写成用户能照着做的句子。
    pub fn validate(&self) -> Result<(), String> {
        if self.listen_addr.parse::<std::net::SocketAddr>().is_err() {
            return Err(format!(
                "监听地址不合法：{}（应形如 127.0.0.1:9000）",
                self.listen_addr
            ));
        }
        cert_deployment::credentials::validate_token(&self.token)?;
        if self.output_dir.trim().is_empty() {
            return Err("证书输出目录不能为空".into());
        }
        match self.layout.as_str() {
            "cert" | "domain" | "both" | "flat" => {}
            other => return Err(format!("未知落盘布局：{}", other)),
        }
        match self.reload_mode.as_str() {
            "none" | "" | "command" | "nginx" | "service" => {}
            other => return Err(format!("未知重载方式：{}", other)),
        }
        if self.reload_mode == "command" && self.reload_command.trim().is_empty() {
            return Err("重载方式为 command 时必须填写命令".into());
        }
        if self.reload_mode == "service" && self.service_name.trim().is_empty() {
            return Err("重载方式为 service 时必须填写服务名".into());
        }
        Ok(())
    }
}

/// 按既有顺序找配置文件（`--config` → exe 同目录 → 当前目录 → %ProgramData%\Kira → C:\certs）。
pub fn find_config_path() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--config") {
        if let Some(p) = args.get(i + 1) {
            return Some(PathBuf::from(p));
        }
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(CONFIG_FILE));
        }
    }
    candidates.push(PathBuf::from(CONFIG_FILE));
    if let Ok(program_data) = std::env::var("ProgramData") {
        candidates.push(PathBuf::from(program_data).join("Kira").join(CONFIG_FILE));
    }
    candidates.push(PathBuf::from("C:\\certs").join(CONFIG_FILE));
    candidates.into_iter().find(|p| p.is_file())
}

/// 默认落点：exe 同目录（双击场景下最直观）。取不到 exe 目录就退回当前目录。
pub fn default_config_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(CONFIG_FILE)))
        .unwrap_or_else(|| PathBuf::from(CONFIG_FILE))
}

pub fn load(path: &Path) -> Result<Config, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
    toml::from_str(&text).map_err(|e| format!("{} 配置格式错误: {}", path.display(), e))
}

/// 写回配置文件（保留一段说明性注释；GUI 编辑会丢掉原有的手写注释）。
pub fn save(path: &Path, cfg: &Config) -> Result<(), String> {
    let body = toml::to_string_pretty(cfg).map_err(|e| format!("配置序列化失败: {}", e))?;
    let text = format!(
        "# 证书接收端配置（由 cert-receiver 界面维护）\n\
         # 查找顺序：--config <path> → exe 同目录 → 当前工作目录 → %ProgramData%\\Kira\\ → C:\\certs\\\n\n{}",
        body
    );
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    let temp = parent.join(format!(".receiver-config-{}.tmp", random_hex(12)));
    let result = (|| {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp).map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        std::fs::rename(&temp, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(|e| format!("写入 {} 失败: {e}", path.display()))
}

pub fn random_hex(n: usize) -> String {
    let mut buf = vec![0u8; n];
    if getrandom::getrandom(&mut buf).is_err() {
        // 极端情况下退回时间戳混合，保证不会写出空的 token
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (((nanos >> (i % 16)) & 0xff) as u8) ^ (i as u8).wrapping_mul(31);
        }
    }
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 空 token / 非法地址必须在保存前挡住，界面才有话可说。
    #[test]
    fn validate_rejects_obviously_wrong_values() {
        let mut c = Config::fresh();
        assert!(c.validate().is_ok());

        for token in ["changeme", "YOUR_SHARED_TOKEN", " YOUR_SHARED_TOKEN "] {
            c.token = token.into();
            assert!(c.validate().is_err());
        }
        c.token = "  ".into();
        assert!(c.validate().unwrap_err().contains("Token"));

        let mut c = Config::fresh();
        c.listen_addr = "127.0.0.1".into();
        assert!(c.validate().unwrap_err().contains("监听地址"));

        let mut c = Config::fresh();
        c.reload_mode = "command".into();
        assert!(c.validate().unwrap_err().contains("命令"));
    }

    /// 落盘再读回来必须等价（含 Option 字段不写出 null）。
    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("cert-receiver-cfg-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(CONFIG_FILE);

        let mut c = Config::fresh();
        c.output_dir = "D:\\certs".into();
        save(&path, &c).unwrap();
        let back = load(&path).unwrap();
        assert_eq!(back.listen_addr, c.listen_addr);
        assert_eq!(back.token, c.token);
        assert_eq!(back.output_dir, "D:\\certs");
        assert!(back.post_script.is_none());

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("post_script"), "None 不该被写进配置");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn random_hex_has_expected_shape() {
        let t = random_hex(16);
        assert_eq!(t.len(), 32);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(t, random_hex(16));
    }
}
