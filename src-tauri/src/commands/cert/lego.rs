use super::*;

// ---------------------------------------------------------------------------
// lego 集成
// ---------------------------------------------------------------------------

/// 运行时检测真实 CPU 架构（非编译目标）。
pub(super) fn cpu_arch() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        if let Ok(a) = std::env::var("PROCESSOR_ARCHITECTURE") {
            if a.eq_ignore_ascii_case("ARM64") {
                return "arm64";
            }
        }
    }
    "x64"
}

pub(super) fn lego_candidates() -> Vec<String> {
    let arch = cpu_arch(); // "x64" / "arm64"
                           // 版本化目录命名：lego_v5.3.1_windows_amd64 / windows_arm64
                           // 同时兼容 amd64/x64 两种写法
    let arch_keywords: Vec<&str> = if arch == "arm64" {
        vec!["arm64"]
    } else {
        vec!["amd64", "x64"]
    };

    let mut cand = Vec::new();

    let bases = crate::commands::utils::get_bin_search_dirs();

    // 在给定目录中查找 lego 候选：版本化子目录、扁平命名
    fn scan(dir: &PathBuf, arch_keywords: &[&str], cand: &mut Vec<String>) {
        if !dir.is_dir() {
            return;
        }
        // 3a. 版本化子目录：lego_*_windows_{arch}/lego.exe
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if let Some(fname) = p.file_name() {
                        let name = fname.to_string_lossy().to_ascii_lowercase();
                        if name.starts_with("lego")
                            && arch_keywords.iter().any(|k| name.contains(k))
                        {
                            let exe = p.join("lego.exe");
                            if exe.is_file() {
                                cand.push(exe.to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
        }
        // 3b. 扁平命名：lego-amd64.exe / lego-x64.exe / lego-arm64.exe
        for k in arch_keywords {
            let flat = dir.join(format!("lego-{}.exe", k));
            if flat.is_file() {
                cand.push(flat.to_string_lossy().to_string());
            }
        }
        // 3c. 扁平 lego.exe
        let flat2 = dir.join("lego.exe");
        if flat2.is_file() {
            cand.push(flat2.to_string_lossy().to_string());
        }
    }

    for b in bases {
        // 旧布局：bin/lego_*_windows_{arch}/ 直接在 bin 下
        scan(&b, &arch_keywords, &mut cand);
        // 新布局：bin/lego/lego_*_windows_{arch}/ 多一层 lego 目录
        scan(&b.join("lego"), &arch_keywords, &mut cand);
    }

    // 4. 系统 PATH
    cand.push("lego.exe".to_string());
    cand.push("lego".to_string());

    cand
}

pub(super) fn find_lego() -> String {
    let mut tried = Vec::new();
    for c in lego_candidates() {
        tried.push(c.clone());
        if std::path::Path::new(&c).exists() {
            eprintln!("[cert] 使用 lego: {}", c);
            return c;
        }
    }
    eprintln!("[cert] 未找到 lego，已尝试候选: {:?}", tried);
    "lego".to_string()
}

pub(super) fn ca_server(ca: &str) -> Option<String> {
    match ca {
        "letsencrypt-staging" => {
            Some("https://acme-staging-v02.api.letsencrypt.org/directory".to_string())
        }
        "letsencrypt" | "" => None,       // 默认生产
        other => Some(other.to_string()), // 自定义 ACME 目录 URL
    }
}

/// 依据 dns_provider + 凭据 data 组装 lego 所需环境变量。
pub(super) fn lego_env(dns_provider: &str, cred: &Credential) -> Vec<(String, String)> {
    let d = &cred.data;
    if dns_provider == "custom" {
        // custom：凭据 data 的键即 lego 环境变量名
        return d.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    }
    let mut env: Vec<(String, String)> = Vec::new();
    let get = |k: &str| d.get(k).cloned().unwrap_or_default();
    match dns_provider {
        "alidns" => {
            env.push(("ALICLOUD_ACCESS_KEY".into(), get("access_key")));
            env.push(("ALICLOUD_SECRET_KEY".into(), get("secret_key")));
            if let Some(r) = d.get("region") {
                env.push(("ALICLOUD_REGION".into(), r.clone()));
            }
        }
        "route53" => {
            env.push(("AWS_ACCESS_KEY_ID".into(), get("AWS_ACCESS_KEY_ID")));
            env.push(("AWS_SECRET_ACCESS_KEY".into(), get("AWS_SECRET_ACCESS_KEY")));
            if let Some(r) = d.get("AWS_REGION") {
                env.push(("AWS_REGION".into(), r.clone()));
            }
        }
        "godaddy" => {
            env.push(("GODADDY_API_KEY".into(), get("GODADDY_API_KEY")));
            env.push(("GODADDY_API_SECRET".into(), get("GODADDY_API_SECRET")));
        }
        "cloudflare" => {
            env.push(("CLOUDFLARE_DNS_API_TOKEN".into(), get("api_token")));
        }
        "dnspod" => {
            env.push(("DNSPOD_API_KEY".into(), get("DNSPOD_API_KEY")));
        }
        "huaweicloud" => {
            env.push((
                "HUAWEICLOUD_ACCESS_KEY_ID".into(),
                get("HUAWEICLOUD_ACCESS_KEY_ID"),
            ));
            env.push((
                "HUAWEICLOUD_SECRET_ACCESS_KEY".into(),
                get("HUAWEICLOUD_SECRET_ACCESS_KEY"),
            ));
            if let Some(r) = d.get("HUAWEICLOUD_REGION") {
                env.push(("HUAWEICLOUD_REGION".into(), r.clone()));
            }
        }
        "baiducloud" => {
            env.push((
                "BAIDUCLOUD_ACCESS_KEY_ID".into(),
                get("BAIDUCLOUD_ACCESS_KEY_ID"),
            ));
            env.push((
                "BAIDUCLOUD_SECRET_ACCESS_KEY".into(),
                get("BAIDUCLOUD_SECRET_ACCESS_KEY"),
            ));
        }
        "dnsla" => {
            env.push(("DNSLA_API_ID".into(), get("DNSLA_API_ID")));
            env.push(("DNSLA_API_SECRET".into(), get("DNSLA_API_SECRET")));
        }
        "westcn" => {
            env.push(("WESTCN_USERNAME".into(), get("WESTCN_USERNAME")));
            env.push(("WESTCN_API_PASSWORD".into(), get("WESTCN_API_PASSWORD")));
        }
        "volcengine" => {
            env.push(("VOLCENGINE_ACCESS_KEY".into(), get("VOLCENGINE_ACCESS_KEY")));
            env.push(("VOLCENGINE_SECRET_KEY".into(), get("VOLCENGINE_SECRET_KEY")));
        }
        "tencentdns" => {
            env.push(("TENCENTCLOUD_SECRET_ID".into(), get("secret_id")));
            env.push(("TENCENTCLOUD_SECRET_KEY".into(), get("secret_key")));
        }
        _ => {
            // 未知 provider：把 data 原样当作环境变量尝试
            for (k, v) in d.iter() {
                env.push((k.clone(), v.clone()));
            }
        }
    }
    env
}

/// lego 数据目录布局：`--path` 指向的是**数据根目录**，
/// 证书位于 `<path>/certificates/`、账号位于 `<path>/accounts/`
/// （实测 lego v5.3.1；v4 同理，只是默认 `--path` 恰为 `~/.lego`）。
/// 历史实现误以为根目录下还有一层 `.lego`（那是默认路径自身的形态），
/// 导致 lego 成功签发后仍读不到 PEM，报「未找到生成的 PEM 文件」。
pub(super) fn lego_cert_dirs(cert_dir: &PathBuf) -> Vec<PathBuf> {
    let mut dirs = vec![cert_dir.join("certificates")];
    // 兼容历史错误布局（曾按 `<path>/.lego/certificates` 读取）
    dirs.push(cert_dir.join(".lego").join("certificates"));
    // 兜底：任意一级子目录下的 certificates/（防 lego 后续版本再改布局）
    if let Ok(entries) = std::fs::read_dir(cert_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let sub = p.join("certificates");
                if sub.is_dir() && !dirs.contains(&sub) {
                    dirs.push(sub);
                }
            }
        }
    }
    dirs
}

/// lego 以域名作为证书文件名，非法字符替换为 `_`
/// （实测 `*.seemove.cn` → `_.seemove.cn`）。
pub(super) fn lego_cert_file_stem(domain: &str) -> String {
    domain
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// 在候选文件名中挑选目标：优先精确命中 `want`，否则取排序后的第一个。
/// 排序保证同一目录多次读取结果稳定（`read_dir` 的返回顺序不保证）。
pub(super) fn pick_name(
    names: &[String],
    want: Option<String>,
    pred: impl Fn(&str) -> bool,
) -> Option<String> {
    let mut cands: Vec<&String> = names.iter().filter(|n| pred(n.as_str())).collect();
    cands.sort();
    if let Some(w) = want {
        if let Some(m) = cands.iter().find(|n| n.as_str() == w) {
            return Some((*m).clone());
        }
    }
    cands.first().map(|n| (*n).clone())
}

/// lego 生成的三个关键文件：内容 + 各自在证书目录中的**原始文件名**
/// （导出 zip 时要按原名附一份，便于与本地证书目录对照）。
pub(super) struct LegoPems {
    pub(super) crt_file: String,
    pub(super) crt: String,
    pub(super) key_file: String,
    pub(super) key: String,
    pub(super) issuer_file: Option<String>,
    pub(super) issuer: String,
}

/// 读取某个 lego 证书目录下的 PEM 三段（cert / key / issuer）。
/// `prefer_domain` 用于同目录存在多份证书时精确匹配，
/// 否则可能取到别的域名的证书并部署出去。
pub(super) fn read_pems_in(cert_store: &PathBuf, prefer_domain: Option<&str>) -> Option<LegoPems> {
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(cert_store) {
        for e in entries.flatten() {
            names.push(e.file_name().to_string_lossy().to_string());
        }
    }
    if names.is_empty() {
        return None;
    }
    let stem = prefer_domain.map(lego_cert_file_stem);
    let crt = pick_name(&names, stem.as_ref().map(|s| format!("{}.crt", s)), |n| {
        n.ends_with(".crt") && !n.contains("issuer")
    })?;
    let key = pick_name(&names, stem.as_ref().map(|s| format!("{}.key", s)), |n| {
        n.ends_with(".key")
    })?;
    let issuer = pick_name(
        &names,
        stem.as_ref().map(|s| format!("{}.issuer.crt", s)),
        |n| n.ends_with(".issuer.crt"),
    );

    let read = |n: &String| std::fs::read_to_string(cert_store.join(n)).unwrap_or_default();
    Some(LegoPems {
        crt: read(&crt),
        crt_file: crt,
        key: read(&key),
        key_file: key,
        issuer: issuer.as_ref().map(read).unwrap_or_default(),
        issuer_file: issuer,
    })
}

/// 读取某证书 id 下 lego 生成的 PEM 三段，自动适配 lego 的数据目录布局。
pub(super) fn read_lego_pems(cert_dir: &PathBuf, prefer_domain: Option<&str>) -> Option<LegoPems> {
    for store in lego_cert_dirs(cert_dir) {
        if let Some(pems) = read_pems_in(&store, prefer_domain) {
            return Some(pems);
        }
    }
    None
}

/// 证书存储目录里现有证书文件的指纹（路径 + 大小 + 修改时间）。
///
/// 用来判断一次 lego 调用到底有没有签发新证书：lego 在「还没到续期时间」时会
/// 跳过续期并以退出码 0 结束（v5.3.1 实测），此时磁盘上一个文件都不会变。
pub(super) fn cert_store_fingerprint(cert_dir: &PathBuf) -> Vec<(String, u64, u128)> {
    let mut out: Vec<(String, u64, u128)> = Vec::new();
    for store in lego_cert_dirs(cert_dir) {
        let entries = match std::fs::read_dir(&store) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !(name.ends_with(".crt") || name.ends_with(".key")) {
                continue;
            }
            let meta = e.metadata().ok();
            let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime = meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            out.push((format!("{}/{}", store.display(), name), len, mtime));
        }
    }
    out.sort();
    out
}

/// 证书目录里是否已有可用证书 —— 决定 lego 走「首次签发」还是「续期」语义。
///
/// 之前用 `last_issue_at.is_some()` 判断，但「跳过续期」也会写 last_issue_at，
/// 跳过一次之后判定就永远偏了；只看磁盘上有没有证书才可靠。
pub(super) fn has_existing_cert(cert_dir: &PathBuf) -> bool {
    lego_cert_dirs(cert_dir).iter().any(|dir| {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .any(|e| e.file_name().to_string_lossy().ends_with(".crt"))
            })
            .unwrap_or(false)
    })
}

/// 申请日志事件名（逐行推送，前端 `listen` 同名事件）。
pub const CERT_ISSUE_LOG_EVENT: &str = "cert-issue-log";

#[derive(Clone, serde::Serialize)]
pub(super) struct IssueLogEvent {
    pub(super) cert_id: String,
    pub(super) line: String,
}

/// 申请 / 续期的日志出口：落盘到 `certs/logs/<证书 id>.log`，同时逐行推给前端。
///
/// 为什么要有：lego 一次签发常要 30s~2min（DNS 传播 + ACME 轮询），以前用
/// `cmd.output()` 一次收全量输出、只在失败时拼进弹窗，用户全程看不到进度，
/// 排查还得去跑 app 的终端翻 eprintln。
pub(super) struct IssueLog {
    pub(super) cert_id: String,
    pub(super) file: Option<std::fs::File>,
    pub(super) app: Option<tauri::AppHandle>,
}

/// 单个证书的日志文件路径。
pub(super) fn issue_log_path(cert_id: &str) -> PathBuf {
    ensure_dir().join("logs").join(format!("{}.log", cert_id))
}

/// 超过这个大小就把旧日志轮转成 `.log.1`，避免无限增长。
pub(super) const ISSUE_LOG_MAX_BYTES: u64 = 2 * 1024 * 1024;

impl IssueLog {
    pub(super) fn new(app: Option<tauri::AppHandle>, cert_id: &str) -> Self {
        Self::open(app, cert_id, issue_log_path(cert_id))
    }

    pub(super) fn open(app: Option<tauri::AppHandle>, cert_id: &str, path: PathBuf) -> Self {
        let file = (|| {
            let dir = path.parent()?;
            std::fs::create_dir_all(dir).ok()?;
            let too_big = std::fs::metadata(&path)
                .map(|m| m.len() > ISSUE_LOG_MAX_BYTES)
                .unwrap_or(false);
            if too_big {
                let _ = std::fs::rename(&path, path.with_extension("log.1"));
            }
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .ok()
        })();
        Self {
            cert_id: cert_id.to_string(),
            file,
            app,
        }
    }

    /// 写一行：终端 + 日志文件 + 前端（任何一路失败都不影响签发本身）。
    pub(super) fn line(&mut self, s: impl AsRef<str>) {
        let line = s.as_ref();
        eprintln!("[cert:{}] {}", self.cert_id, line);
        if let Some(f) = self.file.as_mut() {
            use std::io::Write;
            let _ = writeln!(f, "{}", line);
            // 逐行 flush：用户可能正在盯实时日志，缓一行就白等了
            let _ = f.flush();
        }
        if let Some(app) = self.app.as_ref() {
            let _ = app.emit(
                CERT_ISSUE_LOG_EVENT,
                IssueLogEvent {
                    cert_id: self.cert_id.clone(),
                    line: line.to_string(),
                },
            );
        }
    }
}

/// 把 lego 的「连不上」类错误翻译成人话 —— 原始输出里 `no such host` 对用户
/// 没有任何信息量，只会让人以为证书模块坏了。
pub(super) fn lego_network_hint(stdout: &str, stderr: &str) -> Option<String> {
    let all = format!("{}\n{}", stdout, stderr);
    if all.contains("no such host") {
        // 实测：本机 DNS（如移动的 211.136.112.50）解析不了阿里云 GDS 那条
        // `alidns.cn-hangzhou.aliyuncs.com → …alibabadns.com → popunify-…` 的 CNAME 链，
        // Go 的解析器就报 no such host；换成 223.5.5.5 立刻正常。
        return Some(
            "lego 解析不了上面报错里的域名（DNS 返回 no such host）。这通常是本机 DNS 或代理的问题：\
             把网卡 DNS 换成 223.5.5.5 / 119.29.29.29，或让代理放行 *.aliyuncs.com 后重试。"
                .to_string(),
        );
    }
    if all.contains("Client.Timeout exceeded")
        || all.contains("i/o timeout")
        || all.contains("context deadline exceeded")
        || all.contains("connection reset")
    {
        return Some("连接上游超时/被重置。确认网络或代理稳定后重试。".to_string());
    }
    None
}

/// `force` = 手动「申请」：即使还没到续期时间也要求 lego 重签。
pub(super) fn run_lego(
    cert: &Certificate,
    cred: Option<&Credential>,
    renew: bool,
    force: bool,
    log: &mut IssueLog,
) -> Result<(String, String, String), String> {
    let _operation = crate::runtime_tasks::begin_operation("cert")?;
    let lego = find_lego();
    let cert_dir = ensure_dir().join(&cert.id);
    let _ = std::fs::create_dir_all(&cert_dir);

    // lego v5：--email/--accept-tos/--key-type/--server/--dns/--domains/--path
    // 均为 `run` 子命令旗标（非全局旗标），必须放在 `run` 之后，否则会被当成
    // 全局旗标解析而报 "flag provided but not defined: -email"。
    // 续期在 v5 中没有独立 `renew` 子命令，重新执行 `run` 即可，天数字段为 --renew-days。
    let mut cmd = Command::new(&lego);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    cmd.arg("run");
    // 纯文本日志：默认的 colored 格式带 ANSI 控制符，在前端日志面板里是乱码
    cmd.arg("--log.format").arg("text");
    cmd.arg("--email").arg(&cert.email);
    cmd.arg("--accept-tos");
    cmd.arg("--key-type").arg("rsa4096");
    cmd.arg("--path").arg(&cert_dir);
    if let Some(s) = ca_server(&cert.ca) {
        cmd.arg("--server").arg(s);
    }
    match cert.validation.as_str() {
        "http" => {
            if cert.domains.iter().any(|d| d.starts_with("*.")) {
                return Err("HTTP-01 验证不支持泛域名，请改用 DNS 授权".into());
            }
            cmd.arg("--http");
            // lego v5 使用 --http.address（默认 ":80"），不是 --http.port
            cmd.arg("--http.address")
                .arg(format!(":{}", cert.http_port.max(1)));
        }
        "cname" => {
            return Err(
                "免 DNS 授权（CNAME 委托）需要一个公网可达的权威 DNS 服务；桌面版无法单独提供，\
                 请改用 DNS 授权或 HTTP 验证"
                    .into(),
            );
        }
        _ => {
            let cred = cred.ok_or_else(|| "DNS 验证需要关联 DNS 授权凭据".to_string())?;
            cmd.arg("--dns").arg(&cert.dns_provider);
            let envs = lego_env(&cert.dns_provider, cred);
            for (k, v) in &envs {
                cmd.env(k, v);
            }
            if !envs.iter().any(|(k, _)| k == "LEGO_DNS_RESOLVERS") {
                cmd.arg("--dns.resolvers")
                    .arg("223.5.5.5:53,114.114.114.114:53,8.8.8.8:53");
            }
            // 不要用「递归解析器能不能查到」当传播判据：
            // 国内权威 DNS（如阿里云 hichina）SOA 默认 TTL 是 600s —— 记录刚写入时，
            // 递归解析器只要有一次查到 NXDOMAIN 就会缓存 10 分钟，而 lego 等传播只等
            // 约 2 分钟（provider 自带超时），于是整个窗口都命中缓存，最后必然报
            // `time limit exceeded: ... NS 223.5.5.5:53 returned NXDOMAIN`。
            // 保留权威 NS 校验（那几台是实时同步的，几秒内就能查到）——真实校验本来就是
            // Let's Encrypt 从多个视角直连权威服务器做的，本地这项只是提前自查。
            cmd.arg("--dns.propagation.disable-rns");
        }
    }
    for dom in &cert.domains {
        cmd.arg("--domains").arg(dom);
    }
    if renew {
        cmd.arg("--renew-days")
            .arg(cert.renew_before_days.to_string());
    }
    if force {
        // lego 在「剩余天数 > --renew-days」时会直接跳过续期，且**退出码仍是 0**
        // （v5.3.1 实测：`INFO Skip renewal: ...` + `EXIT=0`）。后端过去把它当成
        // 签发成功，于是刷新后数据毫无变化 —— 用户看到的就是「点申请没反应」。
        // 手动申请要走真实续期，必须加 --renew-force 绕过到期判断。
        cmd.arg("--renew-force");
        // 续期前 lego 会随机 sleep（实测有 2m50s 的），本意是分散 CA 尖峰、只对
        // 自动化续期有意义；用户手动点「申请」时这 3 分钟纯属干等，跳过它。
        cmd.arg("--no-random-sleep");
    }

    // 打印完整命令行，便于在终端排查
    let cmd_dbg = {
        let mut s = lego.clone();
        for a in cmd.get_args() {
            s.push(' ');
            s.push_str(&a.to_string_lossy());
        }
        s
    };
    log.line(format!(
        "$ {}{}",
        if renew {
            "(续期) "
        } else {
            "(首次签发) "
        },
        cmd_dbg
    ));

    // File-backed streaming avoids detached pipe readers when a module is disabled.
    let mut pending = [Vec::<u8>::new(), Vec::<u8>::new()];
    let output = cert_deployment::reload_command::capture_observed(
        &mut cmd,
        std::time::Duration::from_secs(30 * 60),
        || !crate::runtime_tasks::is_enabled("cert"),
        |stderr, bytes| {
            let line = &mut pending[usize::from(stderr)];
            for byte in bytes {
                if *byte == b'\n' || line.len() == 65536 {
                    log.line(String::from_utf8_lossy(line).trim_end_matches('\r'));
                    line.clear();
                }
                if *byte != b'\n' {
                    line.push(*byte);
                }
            }
        },
    )?;
    for line in pending {
        if !line.is_empty() {
            log.line(String::from_utf8_lossy(&line));
        }
    }
    let stdout_all = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr_all = String::from_utf8_lossy(&output.stderr).into_owned();
    let status = output.status;
    if !status.success() {
        log.line(format!("lego 执行失败（退出码 {}）", status));
        let mut msg = format!(
            "lego 执行失败 ({}):\n{}\n{}",
            status, stdout_all, stderr_all
        );
        if let Some(hint) = lego_network_hint(&stdout_all, &stderr_all) {
            log.line(format!("提示：{}", hint));
            msg.push_str("\n\n提示：");
            msg.push_str(&hint);
        }
        return Err(msg);
    }
    log.line(format!(
        "lego 执行成功（{}）",
        if renew { "续期" } else { "签发" }
    ));
    let prefer = cert.domains.first().map(|s| s.as_str());
    let stores = lego_cert_dirs(&cert_dir);
    let pems = read_lego_pems(&cert_dir, prefer).ok_or_else(|| {
        let listed: Vec<String> = stores
            .iter()
            .map(|d| {
                let files: Vec<String> = std::fs::read_dir(d)
                    .map(|rd| {
                        rd.flatten()
                            .map(|e| e.file_name().to_string_lossy().to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                format!(
                    "{}（{} 个文件{}）",
                    d.display(),
                    files.len(),
                    if files.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", files.join(", "))
                    }
                )
            })
            .collect();
        format!(
            "lego 执行成功但未找到生成的 PEM 文件；已查找: {}",
            listed.join("；")
        )
    })?;
    Ok((pems.crt, pems.key, pems.issuer))
}

/// The worker owns its operation guard, even if an async scheduler is cancelled while it runs.
pub(super) async fn run_lego_owned(
    cert: Certificate,
    cred: Option<Credential>,
    renew: bool,
    force: bool,
    mut log: IssueLog,
) -> Result<(IssueLog, Result<(String, String, String), String>), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let result = run_lego(&cert, cred.as_ref(), renew, force, &mut log);
        (log, result)
    })
    .await
    .map_err(|e| format!("证书申请工作线程失败: {e}"))
}
