use std::{
    io::Write,
    process::{Command, Output},
    time::Duration,
};

pub(super) fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
pub(super) fn nonce() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Use the shared file-backed bounded process-tree executor; no reader threads or inherited pipes.
pub(in crate::commands::cert) fn run_bounded(
    cmd: &mut Command,
    timeout: Duration,
) -> Result<Output, String> {
    cert_deployment::reload_command::capture(cmd, timeout).map_err(|error| {
        format!("{error}; remote transaction state is unknown; confirm rollback before retry")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::time::Instant;
    #[test]
    fn private_files_are_unique_and_removed_on_drop() {
        let first = PrivateFiles::new().unwrap();
        let second = PrivateFiles::new().unwrap();
        assert_ne!(first.dir, second.dir);
        let path = first.write("key.pem", "synthetic-key").unwrap();
        assert!(first.write("key.pem", "overwrite").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(&first.dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        drop(first);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    fn transaction_case(
        post: &str,
        mismatch: bool,
        fail_second_move: bool,
    ) -> (bool, String, String, String, String, bool) {
        use std::os::unix::fs::PermissionsExt;
        let files = PrivateFiles::new().unwrap();
        let cert_dir = files.dir.join("cert dir");
        let key_dir = files.dir.join("key'dir");
        std::fs::create_dir(&cert_dir).unwrap();
        std::fs::create_dir(&key_dir).unwrap();
        let cert = cert_dir.join("fullchain.pem");
        let key = key_dir.join("private.key");
        std::fs::write(&cert, "old-cert").unwrap();
        std::fs::write(&key, "old-key").unwrap();
        std::fs::write(format!("{}.kira-test.new", cert.display()), "new-pair").unwrap();
        std::fs::write(
            format!("{}.kira-test.new", key.display()),
            if mismatch { "different" } else { "new-pair" },
        )
        .unwrap();
        let openssl = files
            .write("openssl", "#!/bin/sh\ncat -- \"$3\"\n")
            .unwrap();
        std::fs::set_permissions(openssl, std::fs::Permissions::from_mode(0o700)).unwrap();
        if fail_second_move {
            let mv = files.write("mv", "#!/bin/sh\ncase \"$3\" in *private.key.kira-test.new) exit 9;; esac\nexec /bin/mv \"$@\"\n").unwrap();
            std::fs::set_permissions(mv, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let runtime = files.dir.join("runtime");
        std::fs::write(&runtime, "old-cert").unwrap();
        let post = format!("runtime={}; {post}", quote(&runtime.to_string_lossy()));
        let script = install_script(
            &cert.to_string_lossy(),
            &key.to_string_lossy(),
            "test",
            &post,
        );
        let mut cmd = Command::new("sh");
        cmd.args(["-c", &script]);
        cmd.env(
            "PATH",
            format!("{}:{}", files.dir.display(), std::env::var("PATH").unwrap()),
        );
        let out = run_bounded(&mut cmd, Duration::from_secs(5)).unwrap();
        (
            out.status.success(),
            std::fs::read_to_string(&cert).unwrap(),
            std::fs::read_to_string(&key).unwrap(),
            String::from_utf8_lossy(&out.stderr).into(),
            std::fs::read_to_string(runtime).unwrap(),
            std::path::PathBuf::from(format!("{}.kira-test.old", cert.display())).exists()
                && std::path::PathBuf::from(format!("{}.kira-lock", cert.display())).exists(),
        )
    }
    #[cfg(unix)]
    #[test]
    fn remote_pair_commits_across_different_directories() {
        let (ok, c, k, _, _, _) = transaction_case(":", false, false);
        assert!(ok);
        assert_eq!(c, "new-pair");
        assert_eq!(k, "new-pair");
    }
    #[cfg(unix)]
    #[test]
    fn mismatched_pair_does_not_modify_live_files() {
        let (ok, c, k, _, _, _) = transaction_case(":", true, false);
        assert!(!ok);
        assert_eq!(c, "old-cert");
        assert_eq!(k, "old-key");
    }
    #[cfg(unix)]
    #[test]
    fn second_move_or_reload_failure_restores_both_files() {
        for (post, fail_move) in [("false", false), (":", true)] {
            let (ok, c, k, stderr, _, preserved) = transaction_case(post, false, fail_move);
            assert!(!ok);
            assert_eq!(c, "old-cert");
            assert_eq!(k, "old-key");
            assert!(stderr.contains("KIRA_FILES_ROLLBACK_CONFIRMED"));
            if post == "false" {
                assert!(stderr.contains("KIRA_RUNTIME_ROLLBACK_UNCONFIRMED"));
                assert!(stderr.contains("KIRA_ROLLBACK_UNCONFIRMED"));
                assert!(preserved);
            } else {
                assert!(stderr.contains("KIRA_ROLLBACK_CONFIRMED"));
                assert!(!preserved);
            }
        }
    }
    #[cfg(unix)]
    #[test]
    fn partial_reload_failure_restores_files_and_runtime_generation() {
        let post = r#"cp -- "$c" "$runtime"; if [ "$(cat -- "$c")" = new-pair ]; then false; fi"#;
        let (ok, cert, key, stderr, runtime, preserved) = transaction_case(post, false, false);
        assert!(!ok);
        assert_eq!(cert, "old-cert");
        assert_eq!(key, "old-key");
        assert_eq!(runtime, "old-cert");
        assert!(stderr.contains("KIRA_RUNTIME_ROLLBACK_CONFIRMED"));
        assert!(stderr.contains("KIRA_ROLLBACK_CONFIRMED"));
        assert!(!preserved);
    }
    #[cfg(unix)]
    #[test]
    fn failed_recovery_reload_does_not_claim_confirmation_or_remove_backups() {
        let post = r#"cp -- "$c" "$runtime"; false"#;
        let (ok, cert, key, stderr, _, preserved) = transaction_case(post, false, false);
        assert!(!ok);
        assert_eq!(cert, "old-cert");
        assert_eq!(key, "old-key");
        assert!(stderr.contains("KIRA_FILES_ROLLBACK_CONFIRMED"));
        assert!(stderr.contains("KIRA_RUNTIME_ROLLBACK_UNCONFIRMED"));
        assert!(!stderr.lines().any(|line| line == "KIRA_ROLLBACK_CONFIRMED"));
        assert!(preserved);
    }
    #[cfg(unix)]
    #[test]
    fn recovery_reload_keeps_errexit_for_compound_commands() {
        let (ok, _, _, stderr, _, preserved) = transaction_case("false; true", false, false);
        assert!(!ok);
        assert!(preserved);
        assert!(stderr.contains("KIRA_RUNTIME_ROLLBACK_UNCONFIRMED"));
        assert!(!stderr.lines().any(|line| line == "KIRA_ROLLBACK_CONFIRMED"));
    }
    #[cfg(unix)]
    #[test]
    fn hanging_transport_has_a_deadline() {
        let now = Instant::now();
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 5"]);
        assert!(run_bounded(&mut cmd, Duration::from_millis(50)).is_err());
        assert!(now.elapsed() < Duration::from_secs(2));
    }
}

pub(super) struct PrivateFiles {
    dir: std::path::PathBuf,
}
impl PrivateFiles {
    pub(super) fn new() -> Result<Self, String> {
        let dir = std::env::temp_dir().join(format!("kira-cert-{}", nonce()?));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&dir).map_err(|e| e.to_string())?;
        let files = Self { dir };
        #[cfg(windows)]
        {
            // Remove inherited access before writing any private material.
            let user = std::env::var("USERNAME").map_err(|e| e.to_string())?;
            let mut cmd = Command::new("icacls");
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000).arg(&files.dir).args([
                "/inheritance:r",
                "/grant:r",
                &format!("{user}:(OI)(CI)F"),
            ]);
            let output = run_bounded(&mut cmd, Duration::from_secs(10))?;
            if !output.status.success() {
                return Err("无法隔离临时证书目录权限".into());
            }
        }
        Ok(files)
    }
    pub(super) fn write(&self, name: &str, data: &str) -> Result<std::path::PathBuf, String> {
        let path = self.dir.join(name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(|e| e.to_string())?;
        file.write_all(data.as_bytes()).map_err(|e| e.to_string())?;
        Ok(path)
    }
}
impl Drop for PrivateFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Stages are siblings of their targets, including legacy targets on different filesystems.
pub(super) fn install_script(cert: &str, key: &str, suffix: &str, post: &str) -> String {
    let c = quote(cert);
    let k = quote(key);
    let cs = quote(&format!("{cert}.kira-{suffix}.new"));
    let ks = quote(&format!("{key}.kira-{suffix}.new"));
    let cb = quote(&format!("{cert}.kira-{suffix}.old"));
    let kb = quote(&format!("{key}.kira-{suffix}.old"));
    format!(
        r#"set -eu
c={c}; k={k}; cs={cs}; ks={ks}; cb={cb}; kb={kb}
export c k cs ks cb kb
umask 077
lock="$c.kira-lock"; keylock="$k.kira-lock"; keylocked=0
mkdir -- "$lock" || {{ echo 'Deployment already active or stale lock requires inspection' >&2; exit 1; }}
hc=0; hk=0; changed=0
restore_one() {{
  backup=$1; destination=$2
  rm -f -- "$destination" && cp -a -- "$backup" "$destination" || return 1
  if [ -L "$backup" ]; then
    [ -L "$destination" ] && [ "$(readlink -- "$backup")" = "$(readlink -- "$destination")" ]
  else cmp -s -- "$backup" "$destination"; fi
}}
rollback() {{
  code=$?; trap - EXIT HUP INT TERM
  if [ "$changed" = 1 ]; then
    restored=1
    if [ "$hc" = 1 ]; then restore_one "$cb" "$c" || restored=0; else rm -f -- "$c" || restored=0; fi
    if [ "$hk" = 1 ]; then restore_one "$kb" "$k" || restored=0; else rm -f -- "$k" || restored=0; fi
    runtime_restored=0
    if [ "$restored" = 1 ]; then
      echo KIRA_FILES_ROLLBACK_CONFIRMED >&2
      if sh -e -c {post_script}; then runtime_restored=1; fi
    else echo KIRA_FILES_ROLLBACK_UNCONFIRMED >&2; fi
    if [ "$runtime_restored" = 1 ]; then echo KIRA_RUNTIME_ROLLBACK_CONFIRMED >&2; else echo KIRA_RUNTIME_ROLLBACK_UNCONFIRMED >&2; fi
    if [ "$restored" = 1 ] && [ "$runtime_restored" = 1 ]; then echo KIRA_ROLLBACK_CONFIRMED >&2; else echo KIRA_ROLLBACK_UNCONFIRMED >&2; fi
  fi
  rm -f -- "$cs" "$ks" "$cs.pub" "$ks.pub"
  if [ "$changed" = 0 ] || {{ [ "${{restored:-0}}" = 1 ] && [ "${{runtime_restored:-0}}" = 1 ]; }}; then
    rm -f -- "$cb" "$kb"
    rmdir -- "$lock" || true
    if [ "$keylocked" = 1 ]; then rmdir -- "$keylock" || true; fi
  else echo 'Recovery unconfirmed; backups and deployment locks retained for inspection' >&2; fi
  exit "$code"
}}
trap rollback EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -- "$keylock" || {{ echo 'Key deployment already active or stale lock requires inspection' >&2; exit 1; }}
keylocked=1
for destination in "$c" "$k"; do
  if [ -e "$destination" ] && [ ! -f "$destination" ]; then echo 'Target is not a regular file' >&2; exit 1; fi
done
openssl x509 -in "$cs" -noout -pubkey > "$cs.pub"
openssl pkey -in "$ks" -pubout > "$ks.pub"
cmp -s -- "$cs.pub" "$ks.pub"
rm -f -- "$cs.pub" "$ks.pub"
if [ -e "$c" ] || [ -L "$c" ]; then cp -a -- "$c" "$cb"; hc=1; fi
if [ -e "$k" ] || [ -L "$k" ]; then cp -a -- "$k" "$kb"; hk=1; fi
changed=1
chmod 600 -- "$ks"
mv -f -- "$cs" "$c"
mv -f -- "$ks" "$k"
( {post}
)
changed=0
rm -f -- "$cb" "$kb"
"#,
        post = if post.trim().is_empty() { ":" } else { post },
        post_script = quote(if post.trim().is_empty() { ":" } else { post })
    )
}
