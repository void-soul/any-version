//! Native TCP listener enumeration. Does not depend on netstat PATH, encoding or language.
#[link(name = "iphlpapi")]
extern "system" {
    fn GetExtendedTcpTable(
        table: *mut std::ffi::c_void,
        size: *mut u32,
        order: i32,
        family: u32,
        class: u32,
        reserved: u32,
    ) -> u32;
}

pub(super) fn port_owner_pids(port: u16) -> Vec<u32> {
    let mut pids = Vec::new();
    for (family, row_words, port_word, pid_word) in [(2, 6, 2, 5), (23, 14, 5, 13)] {
        let mut size = 0;
        // TCP_TABLE_OWNER_PID_LISTENER = 3; ERROR_INSUFFICIENT_BUFFER = 122.
        let status =
            unsafe { GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, family, 3, 0) };
        if status != 122 && status != 0 {
            continue;
        }
        // The table can grow between calls. Retry with fresh capacity, always bounded.
        for _ in 0..4 {
            if size < 4 || size > 16 * 1024 * 1024 {
                break;
            }
            let mut words = vec![0u32; (size as usize + 3) / 4];
            let mut actual = (words.len() * 4) as u32;
            let status = unsafe {
                GetExtendedTcpTable(words.as_mut_ptr().cast(), &mut actual, 0, family, 3, 0)
            };
            if status == 122 {
                size = actual;
                continue;
            }
            if status != 0 {
                break;
            }
            let count = words[0] as usize;
            let available = (actual as usize / 4).min(words.len());
            if count > available.saturating_sub(1) / row_words {
                break;
            }
            for row in words[1..1 + count * row_words].chunks_exact(row_words) {
                if u16::from_be(row[port_word] as u16) == port {
                    let pid = row[pid_word];
                    if !pids.contains(&pid) {
                        pids.push(pid);
                    }
                }
            }
            break;
        }
    }
    pids
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_table_finds_ipv4_and_ipv6_loopback() {
        for address in ["127.0.0.1:0", "[::1]:0"] {
            let listener = std::net::TcpListener::bind(address).expect("loopback listener");
            let port = listener.local_addr().unwrap().port();
            assert!(
                port_owner_pids(port).contains(&std::process::id()),
                "missing {address} owner"
            );
        }
    }
}
