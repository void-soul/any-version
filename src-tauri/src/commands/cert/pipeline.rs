use super::*;

// ---------------------------------------------------------------------------
// 申请 / 续期 + 部署编排
// ---------------------------------------------------------------------------

/// 一次「申请 / 续期」的结果。
///
/// 为什么不直接回 `Certificate`：lego 在证书还没到续期时间时会**跳过**签发却
/// 仍以退出码 0 结束，只回 Certificate 的话前端无从分辨「真的签了」和「什么都没做」，
/// 表现就是「点了申请没反应」。`renewed == false` 即表示后者。
#[derive(serde::Serialize)]
pub struct IssueResult {
    pub cert: Certificate,
    /// 本次是否真的重新签发了证书（false = 未到续期时间，被 lego 跳过）
    pub renewed: bool,
    /// 证书剩余有效期（天，负数表示已过期）
    pub days_left: Option<i64>,
    /// Issuance and deployment are separate outcomes; a fresh certificate may fail to deploy.
    pub deployment: DeploySummary,
}

/// 申请（或续期）指定证书，并部署到其所有关联节点。
///
/// `force`：用户手动点「申请」时为 true —— 立刻重签，不看续期阈值；
/// 定时任务传 false，交给 lego 按 `renew_before_days` 自己判断该不该续。
/// `app`：用于把整个过程（lego 输出 + 部署结果）实时推给前端；没有也能跑，只是没有实时日志。
pub async fn issue_and_deploy(
    app: Option<tauri::AppHandle>,
    cert_id: &str,
    force: bool,
) -> Result<IssueResult, String> {
    let mut log = IssueLog::new(app, cert_id);
    let mut certs = load_certs();
    let mut nodes = load_json::<DeployNode>("deploy_nodes.json");
    let creds = load_json::<Credential>("credentials.json");

    let idx = certs
        .iter()
        .position(|c| c.id == cert_id)
        .ok_or_else(|| "证书不存在".to_string())?;
    let cert = certs[idx].clone();
    log.line(format!(
        "===== {} | {} | {} | 域名：{} =====",
        Utc::now().format("%Y-%m-%d %H:%M:%S"),
        if force {
            "强制重签"
        } else {
            "按阈值续期"
        },
        if cert.ca.is_empty() {
            "letsencrypt"
        } else {
            cert.ca.as_str()
        },
        cert.domains.join(", ")
    ));

    // HTTP 验证不需要 DNS 授权凭据；DNS 验证必须有
    let cred = creds
        .into_iter()
        .find(|c| c.id == cert.credential_id)
        .map(decrypt_credential);

    let cert_dir = ensure_dir().join(&cert.id);
    let fingerprint_before = cert_store_fingerprint(&cert_dir);
    let renew = has_existing_cert(&cert_dir);
    let pems = match run_lego(&cert, cred.as_ref(), renew, force, &mut log) {
        Ok(v) => v,
        Err(e) => {
            log.line(format!("✗ 申请失败：{}", e));
            certs[idx].status = "error".to_string();
            certs[idx].last_error = Some(e.clone());
            save_json("certificates.json", &certs);
            return Err(e);
        }
    };
    // lego 跳过续期时退出码仍是 0，只能靠「文件有没有变」判断这次到底签没签
    let renewed = fingerprint_before != cert_store_fingerprint(&cert_dir);
    if !renewed {
        log.line("未到续期时间：lego 跳过了续期，磁盘上的证书没有变化（退出码 0）");
    }

    // 更新有效期（从证书 DER 解析 notBefore / notAfter）
    let (not_before, not_after) = extract_validity(&pems.0);
    let now = Utc::now().to_rfc3339();
    certs[idx].not_before = not_before;
    certs[idx].not_after = not_after;
    if renewed {
        certs[idx].last_issue_at = Some(now.clone());
        certs[idx].status = "issued".to_string();
        certs[idx].last_error = None;
    }
    // 被跳过时不动 last_issue_at / status：证书还是旧的那张，
    // 写成「刚刚签发」会让界面的签发时间失真（这正是一直以来的假象来源）。
    let days_left = certs[idx]
        .not_after
        .as_ref()
        .and_then(|na| DateTime::parse_from_rfc3339(na).ok())
        .map(|d| (d.with_timezone(&Utc) - Utc::now()).num_days());
    log.line(format!(
        "证书有效期：{} → {}（剩余 {} 天）",
        certs[idx].not_before.clone().unwrap_or_else(|| "-".into()),
        certs[idx].not_after.clone().unwrap_or_else(|| "-".into()),
        days_left
            .map(|d| d.to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    // 域名列表 → 部署方式关联；target 用于 CDN / 负载均衡等需要目标域名的节点
    // Deploy the renewed validity metadata, rather than the pre-issuance snapshot.
    let cert = certs[idx].clone();
    let links = cert_links(&cert);
    let deployment = if links.is_empty() {
        log.line("（未关联任何部署方式，跳过部署）");
        DeploySummary {
            results: vec![],
            ok_count: 0,
            fail_count: 0,
        }
    } else {
        let (summary, records) =
            deploy_links(&mut log, &cert, &pems, &mut nodes, &links, "issue").await;
        append_deploy_history(&records);
        log.line(format!(
            "部署汇总：成功 {} 个，失败 {} 个",
            summary.ok_count, summary.fail_count
        ));
        summary
    };

    let updated = certs[idx].clone();
    save_json("certificates.json", &certs);
    save_json("deploy_nodes.json", &nodes);
    log.line(format!(
        "===== 结束：{} =====",
        if renewed {
            "已重新签发"
        } else {
            "未到续期时间，证书未变化"
        }
    ));
    Ok(IssueResult {
        cert: updated,
        renewed,
        days_left,
        deployment,
    })
}

/* ─── 部署：申请后自动执行，也可手动单独触发 ─── */

/// 证书关联的部署方式（兼容旧数据里只有 deploy_node_ids 的情况）。
pub(super) fn cert_links(cert: &Certificate) -> Vec<DeployLink> {
    if cert.deploy_links.is_empty() {
        cert.deploy_node_ids
            .iter()
            .map(|id| DeployLink {
                node_id: id.clone(),
                target: String::new(),
            })
            .collect()
    } else {
        cert.deploy_links.clone()
    }
}

/// 一条部署记录（`certs/deploy_history.json`，只保留最近 500 条）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployRecord {
    pub at: String,
    pub cert_id: String,
    pub node_id: String,
    pub node_name: String,
    /// 实际推送的目标域名；空串表示用了证书首个域名
    pub target: String,
    pub ok: bool,
    pub message: String,
    /// 触发来源：issue（申请后自动）| manual（手动部署）| scheduler
    pub trigger: String,
}

/// 单次部署里一个节点的结果。
#[derive(Debug, Clone, Serialize)]
pub struct DeployOutcome {
    pub node_id: String,
    pub node_name: String,
    pub target: String,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeploySummary {
    pub results: Vec<DeployOutcome>,
    pub ok_count: usize,
    pub fail_count: usize,
}

pub(super) const DEPLOY_HISTORY_FILE: &str = "deploy_history.json";
pub(super) const DEPLOY_HISTORY_CAP: usize = 500;

/// 只保留最近 `DEPLOY_HISTORY_CAP` 条（纯函数，便于测试）。
pub(super) fn trim_deploy_history(mut all: Vec<DeployRecord>) -> Vec<DeployRecord> {
    if all.len() > DEPLOY_HISTORY_CAP {
        let cut = all.len() - DEPLOY_HISTORY_CAP;
        all.drain(0..cut);
    }
    all
}

/// 追加部署记录；避免文件无限增长。
pub(super) fn append_deploy_history(records: &[DeployRecord]) {
    if records.is_empty() {
        return;
    }
    let mut all = load_json::<DeployRecord>(DEPLOY_HISTORY_FILE);
    all.extend(records.iter().cloned());
    save_json(DEPLOY_HISTORY_FILE, &trim_deploy_history(all));
}

/// 把一枚证书推到给定的部署方式：逐个节点写日志、更新节点状态，并返回本次的记录。
///
/// 记录由调用方落盘（这样单测里不会往真实数据目录写东西）。
/// `trigger` 只用于记录来源（申请后自动 / 手动 / 调度），不影响行为。
pub(super) async fn deploy_links(
    log: &mut IssueLog,
    cert: &Certificate,
    pems: &(String, String, String),
    nodes: &mut Vec<DeployNode>,
    links: &[DeployLink],
    trigger: &str,
) -> (DeploySummary, Vec<DeployRecord>) {
    let default_target = cert.domains.first().cloned().unwrap_or_default();
    let at = Utc::now().to_rfc3339();
    let mut results: Vec<DeployOutcome> = Vec::new();
    let mut records: Vec<DeployRecord> = Vec::new();

    for link in links {
        let target = if link.target.trim().is_empty() {
            default_target.clone()
        } else {
            link.target.trim().to_string()
        };
        let node = nodes.iter().find(|n| n.id == link.node_id).cloned();
        let Some(node) = node else {
            let msg = "节点不存在（可能已被删除）".to_string();
            log.line(format!("→ 部署到节点 {} 失败：{}", link.node_id, msg));
            results.push(DeployOutcome {
                node_id: link.node_id.clone(),
                node_name: link.node_id.clone(),
                target: target.clone(),
                ok: false,
                message: msg.clone(),
            });
            records.push(DeployRecord {
                at: at.clone(),
                cert_id: cert.id.clone(),
                node_id: link.node_id.clone(),
                node_name: link.node_id.clone(),
                target,
                ok: false,
                message: msg,
                trigger: trigger.to_string(),
            });
            continue;
        };

        log.line(format!(
            "→ 部署到「{}」（{}）…",
            node.name,
            if target.is_empty() {
                "默认域名"
            } else {
                target.as_str()
            }
        ));
        let deployment_result = if node.node_type == "local" {
            super::local_pending::run_registered(
                &cert.id,
                &node.id,
                &target,
                deploy_to_node(&node, cert, pems, &target),
            )
            .await
        } else {
            deploy_to_node(&node, cert, pems, &target).await
        };
        let (ok, message) = match deployment_result {
            Ok(()) => {
                log.line("  部署成功");
                (true, "成功".to_string())
            }
            Err(e) => {
                log.line(format!("  部署失败：{}", e));
                (false, e)
            }
        };

        if let Some(n) = nodes.iter_mut().find(|n| n.id == node.id) {
            if ok {
                n.last_deploy_at = Some(at.clone());
                n.last_deploy_error = None;
            } else {
                n.last_deploy_error = Some(message.clone());
            }
        }

        results.push(DeployOutcome {
            node_id: node.id.clone(),
            node_name: node.name.clone(),
            target: target.clone(),
            ok,
            message: message.clone(),
        });
        records.push(DeployRecord {
            at: at.clone(),
            cert_id: cert.id.clone(),
            node_id: node.id.clone(),
            node_name: node.name.clone(),
            target,
            ok,
            message,
            trigger: trigger.to_string(),
        });
    }

    let fail_count = results.iter().filter(|r| !r.ok).count();
    (
        DeploySummary {
            ok_count: results.len() - fail_count,
            fail_count,
            results,
        },
        records,
    )
}
