use super::*;
use base64::Engine as _;

// ---------------------------------------------------------------------------
// 证书详情（查看 / 下载）
// ---------------------------------------------------------------------------

/// subjectAltName 扩展 OID（2.5.29.17）
pub(super) const SAN_OID: &[u8] = &[0x55, 0x1D, 0x11];
/// commonName 属性 OID（2.5.4.3）
pub(super) const CN_OID: &[u8] = &[0x55, 0x04, 0x03];

/// 证书详情：解析自 PEM 的展示信息 + 证书目录内的 PEM 原文。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertDetail {
    /// 证书主体 CN
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// 签发者 CN
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    /// 序列号（大写十六进制）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_after: Option<String>,
    /// 距到期剩余天数（负数表示已过期）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_left: Option<i64>,
    /// subjectAltName 中的域名
    #[serde(default)]
    pub san: Vec<String>,
    /// 证书目录内实际存在的 PEM 文件（原名 → 内容）
    #[serde(default)]
    pub pems: HashMap<String, String>,
}

/// 取出 PEM 中**第一个**块的 base64 主体（去掉换行）。
pub(super) fn pem_first_body_b64(pem: &str) -> Option<String> {
    let mut body = String::new();
    let mut in_pem = false;
    for line in pem.lines() {
        let l = line.trim();
        if l.starts_with("-----BEGIN") {
            if in_pem {
                break;
            }
            in_pem = true;
            continue;
        }
        if l.starts_with("-----END") {
            break;
        }
        if in_pem {
            body.push_str(l);
        }
    }
    if body.is_empty() {
        None
    } else {
        Some(body)
    }
}

/// 取出 PEM 中第一个块（含 BEGIN/END 行），用于生成「仅含叶子证书」的 cert.cer。
pub(super) fn pem_first_block(pem: &str) -> String {
    let mut out = String::new();
    let mut in_pem = false;
    for line in pem.lines() {
        let t = line.trim();
        if t.starts_with("-----BEGIN") {
            if in_pem {
                break;
            }
            in_pem = true;
        }
        if !in_pem {
            continue;
        }
        out.push_str(t);
        out.push('\n');
        if t.starts_with("-----END") {
            break;
        }
    }
    out
}

/// 拼出 fullchain：lego 的 `.crt` 在新版本里**已包含**签发者证书，
/// 检测到未包含时（老版本行为）才把 issuer 追加其后，两种布局都得到完整链。
pub(super) fn build_fullchain(crt: &str, issuer: &str) -> String {
    if issuer.trim().is_empty() {
        return crt.to_string();
    }
    let flat: String = crt.chars().filter(|c| !c.is_whitespace()).collect();
    match pem_first_body_b64(issuer) {
        Some(b) if flat.contains(&b) => crt.to_string(),
        Some(_) => format!("{}\n{}", crt.trim_end(), issuer.trim_start()),
        None => crt.to_string(),
    }
}

/// 从 X.509 Name（SEQUENCE OF RDN）中取指定 OID 的字符串值（本项目只用 CN）。
pub(super) fn name_value(name: &[u8], oid: &[u8]) -> Option<String> {
    let (tag, body) = DerReader::new(name).read_tlv()?;
    if tag != 0x30 {
        return None;
    }
    for (t, set) in DerReader::new(body).read_children() {
        if t != 0x31 {
            continue;
        }
        for (t2, seq) in DerReader::new(set).read_children() {
            if t2 != 0x30 {
                continue;
            }
            let mut atv = DerReader::new(seq);
            if let Some((toid, oidv)) = atv.read_tlv() {
                if toid == 0x06 && oidv == oid {
                    if let Some((tval, val)) = atv.read_tlv() {
                        // PrintableString / UTF8String / IA5String
                        if tval == 0x13 || tval == 0x0C || tval == 0x16 {
                            return Some(String::from_utf8_lossy(val).to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

/// 从 extensions [3] EXPLICIT 的 value 中提取 subjectAltName 的 dNSName 列表。
pub(super) fn extract_san(extensions: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let seq = match DerReader::new(extensions).read_tlv() {
        Some((0x30, v)) => v,
        _ => return out,
    };
    for (t, ext) in DerReader::new(seq).read_children() {
        if t != 0x30 {
            continue;
        }
        let mut er = DerReader::new(ext);
        let (toid, oid) = match er.read_tlv() {
            Some(x) => x,
            None => continue,
        };
        if toid != 0x06 || oid != SAN_OID {
            continue;
        }
        // Extension ::= SEQUENCE { extnID, critical BOOLEAN DEFAULT FALSE, extnValue OCTET STRING }
        let mut inner = None;
        while let Some((t2, v2)) = er.read_tlv() {
            if t2 == 0x04 {
                inner = Some(v2);
                break;
            }
        }
        let gn = match inner.and_then(|v| DerReader::new(v).read_tlv()) {
            Some((0x30, v)) => v,
            _ => continue,
        };
        for (gt, gv) in DerReader::new(gn).read_children() {
            if gt == 0x82 {
                // dNSName [2] IA5String
                out.push(String::from_utf8_lossy(gv).to_string());
            }
        }
    }
    out
}

/// 解析证书 PEM 得到展示信息（CN / 序列号 / 有效期 / SAN）。
pub(super) fn parse_cert_detail(crt: &str) -> CertDetail {
    let mut detail = CertDetail::default();
    let der = match pem_first_body_b64(crt).and_then(|b| B64.decode(b).ok()) {
        Some(d) => d,
        None => return detail,
    };
    // Certificate ::= SEQUENCE { tbsCertificate, signatureAlgorithm, signatureValue }
    let cert_val = match DerReader::new(&der).read_tlv() {
        Some((0x30, v)) => v,
        _ => return detail,
    };
    let tbs = match DerReader::new(cert_val).read_tlv() {
        Some((0x30, v)) => v,
        _ => return detail,
    };
    // TBSCertificate ::= SEQUENCE { version?, serialNumber, signature, issuer,
    //                                validity, subject, ..., extensions? }
    let mut fields = DerReader::new(tbs).read_children();
    if matches!(fields.first(), Some((0xA0, _))) {
        fields.remove(0); // version [0] EXPLICIT
    }
    let at = |i: usize| fields.get(i).map(|(_, v)| *v);
    if let Some(s) = at(0) {
        detail.serial = Some(s.iter().map(|b| format!("{:02X}", b)).collect());
    }
    if let Some(v) = at(2) {
        detail.issuer = name_value(v, CN_OID);
    }
    if let Some(v) = at(3) {
        let (nb, na) = parse_validity_value(v);
        detail.not_before = nb;
        detail.not_after = na;
    }
    if let Some(v) = at(4) {
        detail.subject = name_value(v, CN_OID);
    }
    if let Some((_, ext)) = fields.iter().find(|(t, _)| *t == 0xA3) {
        detail.san = extract_san(ext);
    }
    detail
}

/// 收集证书目录内的 PEM 文件（原名 → 内容）：仅 `.crt` / `.key`，跳过 `.json` 等元信息。
pub(super) fn collect_pem_files(cert_dir: &PathBuf) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for dir in lego_cert_dirs(cert_dir) {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.ends_with(".crt") || name.ends_with(".key") {
                    out.insert(name, std::fs::read_to_string(e.path()).unwrap_or_default());
                }
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out
}

/// 查看证书：解析出的详情 + 目录内 PEM 原文。
#[tauri::command]
pub fn cert_detail(id: String) -> Result<CertDetail, String> {
    let certs = load_certs();
    let cert = find_cert(&certs, &id).ok_or_else(|| "证书不存在".to_string())?;
    let cert_dir = ensure_dir().join(&cert.id);
    let prefer = cert.domains.first().map(|s| s.as_str());
    let pems = read_lego_pems(&cert_dir, prefer)
        .ok_or_else(|| "未找到证书文件，请先申请证书".to_string())?;

    let mut detail = parse_cert_detail(&pems.crt);
    // 解析失败时退回证书记录里已保存的有效期，界面至少能显示已有信息
    if detail.not_after.is_none() {
        detail.not_before = cert.not_before.clone();
        detail.not_after = cert.not_after.clone();
    }
    if let Some(na) = detail
        .not_after
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
    {
        detail.days_left = Some((na.with_timezone(&Utc) - Utc::now()).num_days());
    }
    detail.pems = collect_pem_files(&cert_dir);
    Ok(detail)
}

/// 打包为 zip（Deflated）。
pub(super) fn build_zip(files: &[(String, String)]) -> Result<Vec<u8>, String> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let opts: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in files {
            zip.start_file(name.as_str(), opts)
                .map_err(|e| format!("写入 zip 条目失败 ({}): {}", name, e))?;
            std::io::Write::write_all(&mut zip, content.as_bytes())
                .map_err(|e| format!("写入 zip 内容失败 ({}): {}", name, e))?;
        }
        zip.finish().map_err(|e| format!("生成 zip 失败: {}", e))?;
    }
    Ok(cursor.into_inner())
}

/// 导出证书为 zip：通用命名（可直接照服务商文档部署）+ lego 原生文件（可追溯）。
///
/// 结构（`<域名>/`，通配符 `*.a.com` 按 lego 规则写作 `_.a.com`）：
/// - `cert.key` 私钥
/// - `cert.cer` 域名证书（仅叶子）
/// - `chain.cer` 中间证书
/// - `fullchain.cer` 叶子 + 中间（lego 的 `.crt` 本身已含链）
/// - `lego/<原名>` lego 原生文件
#[tauri::command]
pub fn cert_export_zip(id: String, target_path: String) -> Result<String, String> {
    let certs = load_certs();
    let cert = find_cert(&certs, &id).ok_or_else(|| "证书不存在".to_string())?;
    let cert_dir = ensure_dir().join(&cert.id);
    let prefer = cert.domains.first().map(|s| s.as_str());
    let pems = read_lego_pems(&cert_dir, prefer)
        .ok_or_else(|| "未找到证书文件，请先申请证书".to_string())?;

    let dir_name = cert
        .domains
        .first()
        .map(|d| lego_cert_file_stem(d))
        .unwrap_or_else(|| cert.id.clone());

    let mut files: Vec<(String, String)> = vec![
        (format!("{}/cert.key", dir_name), pems.key.clone()),
        (format!("{}/cert.cer", dir_name), pem_first_block(&pems.crt)),
        (format!("{}/chain.cer", dir_name), pems.issuer.clone()),
        (
            format!("{}/fullchain.cer", dir_name),
            build_fullchain(&pems.crt, &pems.issuer),
        ),
        (
            format!("{}/lego/{}", dir_name, pems.crt_file),
            pems.crt.clone(),
        ),
        (
            format!("{}/lego/{}", dir_name, pems.key_file),
            pems.key.clone(),
        ),
    ];
    if let Some(f) = &pems.issuer_file {
        files.push((format!("{}/lego/{}", dir_name, f), pems.issuer.clone()));
    }

    let data = build_zip(&files)?;
    let target = PathBuf::from(&target_path);
    std::fs::write(&target, &data)
        .map_err(|e| format!("写入 zip 失败: path={}, error={}", target.display(), e))?;
    Ok(target.to_string_lossy().to_string())
}
