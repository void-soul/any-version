use super::*;

// ---------------------------------------------------------------------------
// Tauri 命令
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn cert_list() -> Vec<Certificate> {
    let mut certs = load_certs();
    // 旧数据补解析：已申请但缺有效期的证书，从本地 PEM 读取
    let mut changed = false;
    for c in certs.iter_mut() {
        if c.not_after.is_some() {
            continue;
        }
        let cert_dir = ensure_dir().join(&c.id);
        let mut parsed = false;
        for dir in lego_cert_dirs(&cert_dir) {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.ends_with(".crt") && !name.contains("issuer") {
                        if let Ok(content) = std::fs::read_to_string(e.path()) {
                            let (nb, na) = extract_validity(&content);
                            if na.is_some() {
                                c.not_before = nb;
                                c.not_after = na;
                                if c.status == "pending" {
                                    c.status = "issued".to_string();
                                }
                                changed = true;
                            }
                        }
                        parsed = true;
                        break;
                    }
                }
            }
            if parsed {
                break;
            }
        }
    }
    if changed {
        save_json("certificates.json", &certs);
    }
    certs
}

#[tauri::command]
pub fn cert_create(
    domains: Vec<String>,
    email: String,
    ca: Option<String>,
    dns_provider: String,
    credential_id: String,
    renew_before_days: Option<i64>,
    deploy_node_ids: Option<Vec<String>>,
    validation: Option<String>,
    http_port: Option<u16>,
    deploy_links: Option<Vec<DeployLink>>,
) -> Certificate {
    let mut certs = load_certs();
    let links = deploy_links.unwrap_or_else(|| {
        deploy_node_ids
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|node_id| DeployLink {
                node_id,
                target: String::new(),
            })
            .collect()
    });
    let node_ids =
        deploy_node_ids.unwrap_or_else(|| links.iter().map(|l| l.node_id.clone()).collect());
    let cert = Certificate {
        id: gen_id("cert"),
        domains,
        email,
        ca: ca.unwrap_or_else(default_ca),
        dns_provider,
        credential_id,
        validation: validation.unwrap_or_else(default_validation),
        http_port: http_port.unwrap_or_else(default_http_port),
        renew_before_days: renew_before_days.unwrap_or_else(default_days),
        not_before: None,
        not_after: None,
        last_issue_at: None,
        status: default_status(),
        last_error: None,
        deploy_links: links,
        deploy_node_ids: node_ids,
    };
    certs.push(cert.clone());
    save_json("certificates.json", &certs);
    cert
}

#[tauri::command]
pub fn cert_update(
    id: String,
    domains: Vec<String>,
    email: String,
    ca: String,
    dns_provider: String,
    credential_id: String,
    validation: String,
    http_port: u16,
    renew_before_days: i64,
) -> Result<Certificate, String> {
    let mut certs = load_certs();
    let c = certs
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| "证书不存在".to_string())?;
    c.domains = domains;
    c.email = email;
    c.ca = ca;
    c.dns_provider = dns_provider;
    c.credential_id = credential_id;
    c.validation = validation;
    c.http_port = http_port.max(1);
    c.renew_before_days = renew_before_days;
    let updated = c.clone();
    save_json("certificates.json", &certs);
    Ok(updated)
}

#[tauri::command]
pub fn cert_update_nodes(id: String, deploy_node_ids: Vec<String>) -> Result<(), String> {
    let mut certs = load_certs();
    let c = certs
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| "证书不存在".to_string())?;
    let old = c.deploy_links.clone();
    c.deploy_links = deploy_node_ids
        .iter()
        .map(|node_id| DeployLink {
            node_id: node_id.clone(),
            target: old
                .iter()
                .find(|l| &l.node_id == node_id)
                .map(|l| l.target.clone())
                .unwrap_or_default(),
        })
        .collect();
    c.deploy_node_ids = deploy_node_ids;
    save_json("certificates.json", &certs);
    Ok(())
}

#[tauri::command]
pub fn cert_update_links(id: String, deploy_links: Vec<DeployLink>) -> Result<(), String> {
    let mut certs = load_certs();
    let c = certs
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| "证书不存在".to_string())?;
    c.deploy_node_ids = deploy_links.iter().map(|l| l.node_id.clone()).collect();
    c.deploy_links = deploy_links;
    save_json("certificates.json", &certs);
    Ok(())
}

#[tauri::command]
pub fn cert_delete(id: String) {
    let certs: Vec<Certificate> = load_certs().into_iter().filter(|c| c.id != id).collect();
    save_json("certificates.json", &certs);
}

#[tauri::command]
pub async fn cert_issue_now(app: tauri::AppHandle, id: String) -> Result<IssueResult, String> {
    // 手动点「申请」= 立刻重签，不受续期阈值限制（否则 lego 会静默跳过）
    issue_and_deploy(Some(app), &id, true).await
}

/// 手动部署：把当前证书推给指定（或证书已关联的）部署方式，过程日志实时推给前端。
///
/// 与「申请」的区别：只部署、不签发 —— 证书内容直接取磁盘上已签发的那份。
#[tauri::command]
pub async fn cert_deploy_now(
    app: tauri::AppHandle,
    id: String,
    links: Option<Vec<DeployLink>>,
) -> Result<DeploySummary, String> {
    let certs = load_certs();
    let cert = find_cert(&certs, &id)
        .cloned()
        .ok_or_else(|| "证书不存在".to_string())?;
    let mut nodes = load_json::<DeployNode>("deploy_nodes.json");

    let cert_dir = ensure_dir().join(&cert.id);
    let pems = read_lego_pems(&cert_dir, cert.domains.first().map(|s| s.as_str()))
        .ok_or_else(|| "证书目录里没有 PEM（还没签发成功过），先点「申请」".to_string())?;
    let pems = (pems.crt, pems.key, pems.issuer);

    let links = match links {
        Some(l) if !l.is_empty() => l,
        _ => cert_links(&cert),
    };
    if links.is_empty() {
        return Err("还没有关联任何部署方式：先到「部署方式」建节点，再在证书里勾选".to_string());
    }

    let mut log = IssueLog::new(Some(app), &cert.id);
    log.line(format!(
        "===== {} | 手动部署 | 域名：{} =====",
        Utc::now().format("%Y-%m-%d %H:%M:%S"),
        cert.domains.join(", ")
    ));
    let (summary, records) =
        deploy_links(&mut log, &cert, &pems, &mut nodes, &links, "manual").await;
    append_deploy_history(&records);
    save_json("deploy_nodes.json", &nodes);
    log.line(format!(
        "===== 部署结束：成功 {} 个，失败 {} 个 =====",
        summary.ok_count, summary.fail_count
    ));
    Ok(summary)
}

/// 某证书的部署记录（新的在前，最多 100 条）。
#[tauri::command]
pub fn deploy_history(id: String) -> Vec<DeployRecord> {
    let mut all = load_json::<DeployRecord>(DEPLOY_HISTORY_FILE);
    all.reverse();
    all.into_iter()
        .filter(|r| r.cert_id == id)
        .take(100)
        .collect()
}

/// 读取某证书的申请日志（最近 200KB，前端「日志」按钮用；没有日志时返回空串）。
#[tauri::command]
pub fn cert_issue_log(id: String) -> Result<String, String> {
    const MAX_READ: usize = 200 * 1024;
    match std::fs::read(issue_log_path(&id)) {
        Ok(bytes) => {
            let start = bytes.len().saturating_sub(MAX_READ);
            Ok(String::from_utf8_lossy(&bytes[start..]).to_string())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("读取申请日志失败: {}", e)),
    }
}

#[tauri::command]
pub fn cert_get_pem(id: String) -> Result<HashMap<String, String>, String> {
    let certs = load_certs();
    let cert = find_cert(&certs, &id).ok_or_else(|| "证书不存在".to_string())?;
    Ok(collect_pem_files(&ensure_dir().join(&cert.id)))
}

#[tauri::command]
pub fn deploy_node_list() -> Vec<DeployNode> {
    load_json::<DeployNode>("deploy_nodes.json")
}

#[tauri::command]
pub fn deploy_node_upsert(
    id: Option<String>,
    node_type: String,
    name: String,
    config: HashMap<String, String>,
    deploy_before_days: Option<i64>,
    note: Option<String>,
) -> Result<DeployNode, String> {
    if node_type == "local" {
        super::deploy_local::LocalDeployment::from_config(&config)?;
    }
    let mut nodes = load_json_checked::<DeployNode>("deploy_nodes.json")?;
    let nid = id.unwrap_or_else(|| gen_id("node"));
    let node = if let Some(pos) = nodes.iter().position(|n| n.id == nid) {
        let mut n = nodes[pos].clone();
        n.node_type = node_type;
        n.name = name;
        n.config = config;
        n.deploy_before_days = deploy_before_days.unwrap_or_else(default_days);
        n.note = note.unwrap_or_default();
        n
    } else {
        DeployNode {
            id: nid.clone(),
            node_type,
            name,
            config,
            deploy_before_days: deploy_before_days.unwrap_or_else(default_days),
            last_deploy_at: None,
            last_deploy_error: None,
            note: note.unwrap_or_default(),
        }
    };
    if let Some(pos) = nodes.iter().position(|n| n.id == nid) {
        nodes[pos] = node.clone();
    } else {
        nodes.push(node.clone());
    }
    save_json_checked("deploy_nodes.json", &nodes)?;
    Ok(node)
}

#[tauri::command]
pub fn deploy_node_delete(id: String) {
    let nodes: Vec<DeployNode> = load_json::<DeployNode>("deploy_nodes.json")
        .into_iter()
        .filter(|n| n.id != id)
        .collect();
    save_json("deploy_nodes.json", &nodes);
    // 同时解除证书对它的关联
    let certs: Vec<Certificate> = load_certs()
        .into_iter()
        .map(|mut c| {
            c.deploy_links.retain(|l| l.node_id != id);
            c.deploy_node_ids.retain(|d| d != &id);
            c
        })
        .collect();
    save_json("certificates.json", &certs);
}

#[tauri::command]
pub async fn deploy_node_test(id: String) -> Result<String, String> {
    let nodes = load_json::<DeployNode>("deploy_nodes.json");
    let node = find_node(&nodes, &id).ok_or_else(|| "节点不存在".to_string())?;
    match node.node_type.as_str() {
        "local" => super::deploy_local::test_local(node.config.clone()).await,
        "windows" | "api" | "docker" => {
            let url = node.config.get("url").cloned().unwrap_or_default();
            let token = node.config.get("token").cloned().unwrap_or_default();
            let base = windows_base_url(&url);
            if base.is_empty() {
                return Err(format!("{} 节点缺少 url", node.node_type));
            }
            // 与部署同一基础地址派生 /health（兼容用户填写的 /push 后缀）
            let health = format!("{}/health", base);
            let resp = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .connect_timeout(std::time::Duration::from_secs(5))
                .build()
                .map_err(|e| e.to_string())?
                .get(&health)
                .bearer_auth(token)
                .send()
                .await
                .map_err(|e| format!("连接失败: {}", e))?;
            Ok(format!("接收端响应 {}", resp.status()))
        }
        "ssh" | "linux" => {
            let auth = ssh_auth(node);
            let (mut ssh_cmd, target) = ssh_cmd_args(node, &auth, "echo ok")?;
            match ssh_cmd.output() {
                Ok(o) if o.status.success() => Ok(format!("SSH 连通（{}）", target)),
                Ok(o) => Err(format!(
                    "SSH 失败: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                )),
                Err(e) => Err(format!("无法执行 ssh: {}", e)),
            }
        }
        "webhook" => {
            let url = cfg(node, "url");
            if url.is_empty() {
                return Err("webhook 节点缺少 url".into());
            }
            let resp = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .map_err(|e| e.to_string())?
                .post(&url)
                .json(&serde_json::json!({ "event": "test" }))
                .send()
                .await
                .map_err(|e| format!("连接失败: {}", e))?;
            Ok(format!("Webhook 响应 {}", resp.status()))
        }
        "baota" => baota_test(node).await,
        "aliyun" | "aliyun_ssl" | "tencent_ssl" | "qiniu" | "qiniu_ssl" => {
            Ok("云厂商类型不支持预检，凭据会在实际部署时校验".to_string())
        }
        other => Ok(format!("节点类型 {} 暂不支持连通性测试", other)),
    }
}

#[tauri::command]
pub fn credential_list() -> Vec<Credential> {
    load_json::<Credential>("credentials.json")
        .into_iter()
        .map(decrypt_credential)
        .collect()
}

#[tauri::command]
pub fn credential_upsert(
    id: Option<String>,
    name: String,
    cred_type: String,
    data: HashMap<String, String>,
    note: Option<String>,
) -> Credential {
    let mut creds = load_json::<Credential>("credentials.json");
    let cid = id.unwrap_or_else(|| gen_id("cred"));
    let cred = Credential {
        id: cid.clone(),
        name,
        cred_type,
        data,
        note: note.unwrap_or_default(),
    };
    let stored = encrypt_credential(cred.clone());
    if let Some(pos) = creds.iter().position(|c| c.id == cid) {
        creds[pos] = stored;
    } else {
        creds.push(stored);
    }
    save_json("credentials.json", &creds);
    cred // 返回明文版本给前端
}

#[tauri::command]
pub fn credential_delete(id: String) {
    let creds: Vec<Credential> = load_json::<Credential>("credentials.json")
        .into_iter()
        .filter(|c| c.id != id)
        .collect();
    save_json("credentials.json", &creds);
}

#[tauri::command]
pub fn cert_scheduler_status(state: tauri::State<CertScheduler>) -> SchedulerState {
    state.state.lock().unwrap().clone()
}

#[tauri::command]
pub fn cert_scheduler_set(
    enabled: bool,
    interval_minutes: Option<u64>,
    state: tauri::State<CertScheduler>,
) {
    let mut s = state.state.lock().unwrap();
    s.enabled = enabled;
    if let Some(i) = interval_minutes {
        s.interval_minutes = i.max(1);
    }
}

#[tauri::command]
pub async fn cert_scheduler_run_now(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let certs = load_certs();
    let now = Utc::now();
    let mut results = Vec::new();
    for c in &certs {
        // 与后台调度同一口径：只有到期（或从未签发、没有有效期）的证书才需要动作。
        // 以前这里无条件全量调用，未到期的也会「跑一遍」，结果全是 lego 静默跳过。
        let due = c
            .not_after
            .as_ref()
            .and_then(|na| DateTime::parse_from_rfc3339(na).ok())
            .map(|d| now + Duration::days(c.renew_before_days) >= d.with_timezone(&Utc))
            .unwrap_or(true);
        if !due {
            results.push(format!("{}: 未到续期时间，已跳过", c.id));
            continue;
        }
        match issue_and_deploy(Some(app.clone()), &c.id, false).await {
            Ok(r) if r.renewed => results.push(format!("{}: ok", c.id)),
            Ok(r) => results.push(format!(
                "{}: 未到续期时间，已跳过（剩余 {} 天）",
                c.id,
                r.days_left.unwrap_or(0)
            )),
            Err(e) => results.push(format!("{}: {}", c.id, e)),
        }
    }
    Ok(results)
}
