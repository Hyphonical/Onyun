//! Tor v3 RFC 4648 lowercase Base32 (`a-z`, `2-7`) validation, encoding, and
//! zero-allocation bit-level prefix matching.

use anyhow::{bail, Result};

const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// Normalizes and validates a user-supplied vanity prefix.
/// Returns the lowercase Base32 string or a clear error explaining invalid characters.
pub(crate) fn normalize_target(input: &str) -> Result<String> {
    let trimmed = input.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        bail!("target prefix cannot be empty");
    }
    if trimmed.len() > 52 {
        bail!(
            "target prefix is {} chars; max prefix length is 52 (characters 53..56 include the Tor v3 checksum)",
            trimmed.len()
        );
    }

    let mut invalid = Vec::new();
    for ch in trimmed.chars() {
        let valid = ch.is_ascii() && decode_char(ch as u8).is_some();
        if !valid && !invalid.contains(&ch) {
            invalid.push(ch);
        }
    }

    if !invalid.is_empty() {
        let bad: String = invalid
            .iter()
            .map(|c| format!("'{c}'"))
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "invalid Tor v3 Base32 character(s): {bad} (allowed: a-z and 2-7; 0, 1, 8, 9 are not valid)"
        );
    }

    Ok(trimmed)
}

#[inline]
pub(crate) const fn decode_char(b: u8) -> Option<u8> {
    match b {
        b'a'..=b'z' => Some(b - b'a'),
        b'A'..=b'Z' => Some(b - b'A'),
        b'2'..=b'7' => Some(26 + (b - b'2')),
        _ => None,
    }
}

/// Encodes 35 raw Tor v3 address bytes into 56 lowercase Base32 characters.
pub(crate) fn encode_onion_bytes(bytes: &[u8; 35]) -> [u8; 56] {
    let mut out = [0u8; 56];
    let mut i = 0;
    let mut o = 0;
    while i < 35 {
        let b0 = bytes[i];
        let b1 = bytes[i + 1];
        let b2 = bytes[i + 2];
        let b3 = bytes[i + 3];
        let b4 = bytes[i + 4];

        out[o] = ALPHABET[(b0 >> 3) as usize];
        out[o + 1] = ALPHABET[(((b0 & 0x07) << 2) | (b1 >> 6)) as usize];
        out[o + 2] = ALPHABET[((b1 >> 1) & 0x1f) as usize];
        out[o + 3] = ALPHABET[(((b1 & 0x01) << 4) | (b2 >> 4)) as usize];
        out[o + 4] = ALPHABET[(((b2 & 0x0f) << 1) | (b3 >> 7)) as usize];
        out[o + 5] = ALPHABET[((b3 >> 2) & 0x1f) as usize];
        out[o + 6] = ALPHABET[(((b3 & 0x03) << 3) | (b4 >> 5)) as usize];
        out[o + 7] = ALPHABET[(b4 & 0x1f) as usize];

        i += 5;
        o += 8;
    }
    out
}

/// Precomputed target prefix nibbles for fast matching against a 32-byte Ed25519
/// public key without Base32 encoding.
#[derive(Clone, Debug)]
pub(crate) struct PrefixMatcher {
    /// Target 5-bit Base32 values (0..31) for each character.
    nibbles: Vec<u8>,
}

impl PrefixMatcher {
    pub(crate) fn new(target: &str) -> Self {
        let nibbles: Vec<u8> = target.bytes().filter_map(decode_char).collect();
        Self { nibbles }
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.nibbles.len()
    }

    /// Returns the number of leading target characters matched by `pubkey` (0..=self.len()).
    /// Optimized to fail fast on character 0, then count upward.
    #[inline]
    pub(crate) fn matched_len(&self, pubkey: &[u8; 32]) -> usize {
        // Fast-path check for the very first character (top 5 bits of pubkey[0]).
        if self.nibbles.is_empty() || (pubkey[0] >> 3) != self.nibbles[0] {
            return 0;
        }
        let mut matched = 1;
        let limit = self.nibbles.len().min(51);
        while matched < limit {
            let bit_offset = matched * 5;
            let byte_idx = bit_offset >> 3;
            let bit_in_byte = bit_offset & 7;
            let got = if bit_in_byte <= 3 {
                (pubkey[byte_idx] >> (3 - bit_in_byte)) & 0x1f
            } else {
                let hi = pubkey[byte_idx] << (bit_in_byte - 3);
                let lo = pubkey[byte_idx + 1] >> (11 - bit_in_byte);
                (hi | lo) & 0x1f
            };
            if got != self.nibbles[matched] {
                break;
            }
            matched += 1;
        }
        matched
    }

    /// Counts how many leading characters of an existing `.onion` hostname match this target.
    pub(crate) fn matched_len_hostname(&self, hostname: &str) -> usize {
        let stem = hostname.trim().strip_suffix(".onion").unwrap_or(hostname.trim());
        let mut matched = 0;
        for (a, &expected) in stem.bytes().zip(self.nibbles.iter()) {
            if decode_char(a) == Some(expected) {
                matched += 1;
            } else {
                break;
            }
        }
        matched
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_base32_targets() {
        assert_eq!(normalize_target("OnyunShop").ok().as_deref(), Some("onyunshop"));
        assert_eq!(normalize_target("a234567z").ok().as_deref(), Some("a234567z"));
        assert!(normalize_target("").is_err());
        assert!(normalize_target("onyun0189").is_err());
        assert!(normalize_target("onyun!").is_err());
    }

    #[test]
    fn prefix_matcher_matches_encoded_bytes() {
        let mut raw = [0u8; 35];
        for (i, b) in raw.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(37).wrapping_add(13);
        }
        let encoded = encode_onion_bytes(&raw);
        let onion_str = std::str::from_utf8(&encoded).unwrap_or("");
        let prefix = &onion_str[..9];
        let matcher = PrefixMatcher::new(prefix);
        let pubkey: [u8; 32] = raw[..32].try_into().unwrap_or([0u8; 32]);
        assert_eq!(matcher.matched_len(&pubkey), 9);

        let mut bad_prefix = prefix.to_string();
        bad_prefix.replace_range(5..6, if &prefix[5..6] == "a" { "b" } else { "a" });
        let matcher2 = PrefixMatcher::new(&bad_prefix);
        assert_eq!(matcher2.matched_len(&pubkey), 5);
    }

    #[test]
    fn prefix_matcher_handles_max_52_char_target_without_panic() {
        let pubkey = [0xffu8; 32];
        let target = "7".repeat(52);
        let matcher = PrefixMatcher::new(&target);
        assert_eq!(matcher.matched_len(&pubkey), 51);
        assert!(normalize_target("onyun\u{2061}").is_err());
    }
}
