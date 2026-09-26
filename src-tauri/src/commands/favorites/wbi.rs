//! B站 WBI 签名（2023-03 起 web 端风控）。
//!
//! 算法与常量来自 bilibili-API-collect 的 `docs/misc/sign/wbi.md`：
//! img_key + sub_key 经混洗表重排取前 32 位得 mixin_key，
//! 参数按 key 升序 + wts 拼成 query 后接 mixin_key 取 MD5 即 w_rid。
//!
//! 这是**逆向得到**的签名，B站随时可能改；文档里的官方测试向量在下面测试里固定住，
//! 一旦 B站换了算法或常量，这个测试会先炸（而不是用户点了导入才发现拉不到数据）。
//!
//! ⚠️ 原公开出处 `SocialSisterYi/bilibili-API-collect` 已于 2026-01-28 收到 B站委托
//! 律所的律师函后永久关停（文档与源码已删除），上面的链接不再可用。**契约基线现由本项目
//! 自持**：`.agents/skills/bilibili-api-sync/references/contracts.md`（含算法、混洗表、
//! 官方向量与端点），维护流程见该技能的 SKILL.md。改动前先跑
//! `.agents/skills/bilibili-api-sync/scripts/sync.ps1` 自检。

/// 重排映射表（长 64，只取前 32 项参与 mixin_key）。
const MIXIN_KEY_ENC_TAB: [usize; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42, 19, 29,
    28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25,
    54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
];

/// 拼接 img_key + sub_key 后按混洗表重排，取前 32 位。
pub fn mixin_key(img_key: &str, sub_key: &str) -> String {
    let raw = format!("{}{}", img_key, sub_key);
    let bytes = raw.as_bytes();
    MIXIN_KEY_ENC_TAB
        .iter()
        .take(32)
        .map(|&index| bytes.get(index).copied().unwrap_or(b'0') as char)
        .collect()
}

/// 从 `https://i0.hdslb.com/bfs/wbi/7cd0...c.png` 取出 `7cd0...c`。
pub fn key_from_url(url: &str) -> Option<String> {
    let file = url.rsplit_once('/')?.1;
    let stem = file.rsplit_once('.').map(|(name, _)| name).unwrap_or(file);
    if stem.is_empty() {
        None
    } else {
        Some(stem.to_string())
    }
}

/// 百分号编码（大写十六进制，空格 → `%20`），并剔除 `!'()*` 这 5 个字符。
///
/// 细节决定成败：小写十六进制或空格编码成 `+` 都会算出错误的 w_rid。
fn url_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || "-_.~".contains(ch) {
            out.push(ch);
            continue;
        }
        if "!'()*".contains(ch) {
            continue;
        }
        let mut buffer = [0u8; 4];
        for byte in ch.encode_utf8(&mut buffer).as_bytes() {
            out.push_str(&format!("%{:02X}", byte));
        }
    }
    out
}

/// 给请求参数签名，返回**完整 query**（含 wts 与 w_rid）。
pub fn sign_query(params: &[(&str, String)], img_key: &str, sub_key: &str, wts: u64) -> String {
    let mut pairs: Vec<(&str, String)> = params.to_vec();
    pairs.push(("wts", wts.to_string()));
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    let query = pairs
        .iter()
        .map(|(key, value)| format!("{}={}", url_encode(key), url_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    let digest = md5::compute(format!("{}{}", query, mixin_key(img_key, sub_key)));
    format!("{}&w_rid={:x}", query, digest)
}

#[cfg(test)]
mod tests {
    use super::{key_from_url, mixin_key, sign_query, url_encode};

    const IMG_KEY: &str = "7cd084941338484aae1ad9425b84077c";
    const SUB_KEY: &str = "4932caff0ff746eab6f01bf08b70ac45";

    /// 官方测试向量：混洗结果必须完全对得上，否则签名一定失败。
    #[test]
    fn mixin_key_matches_official_vector() {
        assert_eq!(mixin_key(IMG_KEY, SUB_KEY), "ea1db124af3c7062474693fa704f4ff8");
    }

    /// 官方测试向量：整条 query（含 w_rid）逐字符一致。
    #[test]
    fn signed_query_matches_official_vector() {
        let params = vec![
            ("foo", "114".to_string()),
            ("bar", "514".to_string()),
            ("zab", "1919810".to_string()),
        ];
        assert_eq!(
            sign_query(&params, IMG_KEY, SUB_KEY, 1702204169),
            "bar=514&foo=114&wts=1702204169&zab=1919810&w_rid=8f6f2b5b3d485fe1886cec6a0be8c5d4"
        );
    }

    #[test]
    fn key_from_url_takes_filename_stem() {
        assert_eq!(
            key_from_url("https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png").as_deref(),
            Some("7cd084941338484aae1ad9425b84077c")
        );
        // 无扩展名 / 空串都不能 panic
        assert_eq!(key_from_url("https://x/wbi/abc").as_deref(), Some("abc"));
        assert_eq!(key_from_url(""), None);
    }

    /// 空格必须是 %20（不是 +），十六进制必须大写，且 `!'()*` 要被剔除。
    #[test]
    fn url_encode_follows_bilibili_rules() {
        assert_eq!(url_encode("one one four"), "one%20one%20four");
        assert_eq!(url_encode("五一四"), "%E4%BA%94%E4%B8%80%E5%9B%9B");
        assert_eq!(url_encode("a!b'c(d)e*f"), "abcdef");
        assert_eq!(url_encode("A-_.~9"), "A-_.~9");
    }
}
