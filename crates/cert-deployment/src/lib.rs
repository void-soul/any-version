//! Certificate deployment shared by local deployment and both HTTP receivers.
//! This crate has no Tauri, application configuration or logging dependencies.
pub mod deployment;
pub mod protocol;
pub mod reading;
pub mod credentials;
pub mod reload_command;
mod reload_plan;

fn random_hex(length: usize) -> String {
    let mut bytes = vec![0; length];
    if getrandom::getrandom(&mut bytes).is_err() {
        // Only used for create-new scratch paths, never credentials. A collision
        // safely fails directory creation rather than reusing an existing path.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = (time.rotate_right(index as u32)
                ^ sequence as u128
                ^ std::process::id() as u128) as u8;
        }
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
