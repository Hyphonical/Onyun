//! Atomic Tor v3 `HiddenServiceDir` persistence into `./mints/<target>/`
//! and database-free resume from existing `hostname` files.

use crate::core::base32::PrefixMatcher;
use crate::core::onion::OnionKeypair;
use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Returns the target output directory `<base_dir>/<target>`.
pub(crate) fn target_dir(base_dir: &Path, target: &str) -> PathBuf {
    base_dir.join(target)
}

/// Checks if `<base_dir>/<target>/hostname` already exists.
/// If so, returns `(existing_address, matched_prefix_len)`.
pub(crate) fn load_existing_best(base_dir: &Path, target: &str) -> Option<(String, usize)> {
    let hostname_path = target_dir(base_dir, target).join("hostname");
    let raw = fs::read_to_string(hostname_path).ok()?;
    let addr = raw.trim().to_ascii_lowercase();
    if addr.len() != 62 || !addr.ends_with(".onion") {
        return None;
    }
    let matcher = PrefixMatcher::new(target);
    let matched = matcher.matched_len_hostname(&addr);
    Some((addr, matched))
}

/// Atomically writes `hs_ed25519_secret_key`, `hs_ed25519_public_key`, and
/// `hostname` into `<base_dir>/<target>/`.
pub(crate) fn save_keypair_atomic(base_dir: &Path, target: &str, kp: &OnionKeypair) -> Result<PathBuf> {
    let dir = target_dir(base_dir, target);
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create output directory {}", dir.display()))?;
    set_dir_permissions(&dir)?;

    let secret_bytes = kp.secret_key_file_bytes();
    write_atomic(&dir, "hs_ed25519_secret_key", &secret_bytes, true)?;

    let public_bytes = kp.public_key_file_bytes();
    write_atomic(&dir, "hs_ed25519_public_key", &public_bytes, false)?;

    let hostname_content = format!("{}\n", kp.address);
    write_atomic(&dir, "hostname", hostname_content.as_bytes(), false)?;

    Ok(dir)
}

fn write_atomic(dir: &Path, filename: &str, bytes: &[u8], restricted: bool) -> Result<()> {
    let tmp_path = dir.join(format!("{filename}.tmp"));
    let final_path = dir.join(filename);

    {
        let mut file = File::create(&tmp_path)
            .with_context(|| format!("failed to create {}", tmp_path.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("failed to write {}", tmp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to sync {}", tmp_path.display()))?;
    }

    set_file_permissions(&tmp_path, restricted)?;

    fs::rename(&tmp_path, &final_path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            tmp_path.display(),
            final_path.display()
        )
    })?;

    Ok(())
}

#[cfg(unix)]
fn set_dir_permissions(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to set 0700 permissions on {}", dir.display()))
}

#[cfg(not(unix))]
fn set_dir_permissions(_dir: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_file_permissions(path: &Path, restricted: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if restricted { 0o600 } else { 0o644 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("failed to set {mode:o} permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn set_file_permissions(_path: &Path, _restricted: bool) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::onion::derive_address;

    #[test]
    fn save_and_resume_roundtrip() {
        let tmp_base = std::env::temp_dir().join(format!("onyun_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp_base);

        let pubkey = [0x11u8; 32];
        let addr = derive_address(&pubkey);
        let target = &addr[..4];

        let kp = OnionKeypair {
            address: addr.clone(),
            public_key: pubkey,
            expanded_secret: [0x22u8; 64],
        };

        let saved_dir = save_keypair_atomic(&tmp_base, target, &kp).unwrap_or_default();
        assert!(saved_dir.join("hostname").exists());
        assert!(saved_dir.join("hs_ed25519_public_key").exists());
        assert!(saved_dir.join("hs_ed25519_secret_key").exists());

        let resumed = load_existing_best(&tmp_base, target);
        assert_eq!(resumed, Some((addr, 4)));

        let _ = fs::remove_dir_all(&tmp_base);
    }
}
