//! Tor v3 `.onion` address derivation, SHA3-256 checksum, and Tor/Arti
//! `hs_ed25519_*` key serialization.

use crate::core::base32::encode_onion_bytes;
use sha3::{Digest, Sha3_256};

const CHECKSUM_PREFIX: &[u8] = b".onion checksum";
const ONION_VERSION: u8 = 0x03;

pub(crate) const PUBLIC_KEY_HEADER: &[u8; 32] = b"== ed25519v1-public: type0 ==\x00\x00\x00";
pub(crate) const SECRET_KEY_HEADER: &[u8; 32] = b"== ed25519v1-secret: type0 ==\x00\x00\x00";

/// Complete Tor v3 keypair ready to be written to a `HiddenServiceDir`.
#[derive(Clone, Debug)]
pub(crate) struct OnionKeypair {
    /// Full 62-char `<56-base32>.onion` address.
    pub(crate) address: String,
    /// 32-byte compressed Ed25519 public key.
    pub(crate) public_key: [u8; 32],
    /// 64-byte expanded Ed25519 secret key (32-byte clamped scalar LE + 32-byte nonce prefix).
    pub(crate) expanded_secret: [u8; 64],
}

/// Computes the 2-byte Tor v3 checksum for `pubkey`.
pub(crate) fn compute_checksum(pubkey: &[u8; 32]) -> [u8; 2] {
    let mut hasher = Sha3_256::new();
    hasher.update(CHECKSUM_PREFIX);
    hasher.update(pubkey);
    hasher.update([ONION_VERSION]);
    let digest = hasher.finalize();
    [digest[0], digest[1]]
}

pub(crate) fn derive_address(pubkey: &[u8; 32]) -> String {
    let checksum = compute_checksum(pubkey);
    let mut raw = [0u8; 35];
    raw[..32].copy_from_slice(pubkey);
    raw[32] = checksum[0];
    raw[33] = checksum[1];
    raw[34] = ONION_VERSION;

    let encoded = encode_onion_bytes(&raw);
    let mut out = String::with_capacity(62);
    if let Ok(s) = std::str::from_utf8(&encoded) {
        out.push_str(s);
    }
    out.push_str(".onion");
    out
}

impl OnionKeypair {
    pub(crate) fn public_key_file_bytes(&self) -> [u8; 64] {
        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(PUBLIC_KEY_HEADER);
        buf[32..].copy_from_slice(&self.public_key);
        buf
    }

    pub(crate) fn secret_key_file_bytes(&self) -> [u8; 96] {
        let mut buf = [0u8; 96];
        buf[..32].copy_from_slice(SECRET_KEY_HEADER);
        buf[32..].copy_from_slice(&self.expanded_secret);
        buf
    }
}

