//! `.rdp` 文件的拼装。
//!
//! 抽成纯函数的原因：这些行是「字符串协议」，写错一个字段名 mstsc 就静默忽略，
//! 表现为「某个选项在界面上勾了但没生效」——只能靠断言字段内容兜住。
//!
//! 行尾必须是 CRLF，mstsc 对 LF 的容忍度并不可靠。

use super::models::RdpConnection;

/// 生成交给 mstsc 的 `.rdp` 内容。
///
/// `password_line` 是已经 DPAPI 加密好的 `password 51:b:<hex>\r\n`；
/// 传 `None` 或空串表示没有可用密码，此时退回系统凭据框。
pub fn build_rdp_file(conn: &RdpConnection, password_line: Option<&str>) -> String {
    let mut text = format!(
        "full address:s:{}:{}\r\nusername:s:{}\r\n",
        conn.host, conn.port, conn.username
    );
    if !conn.domain.is_empty() {
        text.push_str(&format!("domain:s:{}\r\n", conn.domain));
    }
    // 剪贴板共享：关闭时显式写 0，避免继承客户端默认（默认可能是开）
    text.push_str(if conn.clipboard {
        "redirectclipboard:i:1\r\n"
    } else {
        "redirectclipboard:i:0\r\n"
    });
    // 本地驱动器映射（与 FinalShell 的「驱动器」选项对应）：重定向全部盘符
    if conn.map_drives {
        text.push_str("drivestoredirect:s:*\r\n");
    }
    match password_line {
        Some(line) if !line.is_empty() => text.push_str(line),
        _ => text.push_str("prompt for credentials:i:1\r\n"),
    }
    text
}
