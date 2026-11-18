//! Device Bearer-token registry and zeroizing secret container.
//!
//! Raw API keys and device tokens never implement `Debug` or `Display` and are
//! zeroed in memory on drop. Incoming Bearer tokens are hashed with SHA-256 and
//! compared in fixed 32-byte constant time.

use sha2::{Digest, Sha256};
use std::sync::atomic::{compiler_fence, Ordering};

/// Secret byte buffer with volatile zeroing on drop and no `Debug`/`Display`.
pub struct SecretString(Vec<u8>);

impl SecretString {
    pub fn new(raw: impl Into<String>) -> Option<Self> {
        let mut bytes = raw.into().into_bytes();
        while bytes.last().is_some_and(u8::is_ascii_whitespace) {
            bytes.pop();
        }
        let first = bytes
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .unwrap_or(bytes.len());
        bytes.drain(..first);
        if bytes.is_empty()
            || bytes.len() > 16_384
            || bytes.iter().any(|b| !matches!(b, 0x21..=0x7e))
        {
            zeroize_vec(&mut bytes);
            return None;
        }
        Some(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        zeroize_vec(&mut self.0);
    }
}

fn zeroize_vec(buf: &mut Vec<u8>) {
    for idx in 0..buf.capacity() {
        unsafe {
            std::ptr::write_volatile(buf.as_mut_ptr().add(idx), 0);
        }
    }
    compiler_fence(Ordering::SeqCst);
}

fn sha256_digest(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

fn constant_time_eq_32(left: &[u8; 32], right: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for i in 0..32 {
        diff |= left[i] ^ right[i];
    }
    diff == 0
}

#[derive(Clone)]
struct DeviceTokenEntry {
    device_id: String,
    token_sha256: [u8; 32],
}

#[derive(Clone, Default)]
pub struct TokenRegistry {
    entries: Vec<DeviceTokenEntry>,
}

impl TokenRegistry {
    /// Parse `GATEWAY_AUTH_TOKENS` from comma- or newline-separated entries.
    ///
    /// Supports both named per-device tokens (`desktop=tokenA,laptop=tokenB`)
    /// and unnamed tokens (`tokenA,tokenB` -> `device-1`, `device-2`) so
    /// individual laptops/phones can be revoked independently.
    pub fn parse(raw: &str) -> Result<Self, &'static str> {
        let mut entries = Vec::new();
        for part in raw.split([',', '\n', '\r']) {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                continue;
            }
            let (device_id, token_part) = if let Some((label, token)) = trimmed.split_once('=') {
                let label = label.trim();
                if is_valid_device_label(label) && !token.trim().is_empty() {
                    (label.to_string(), token.trim())
                } else {
                    (format!("device-{}", entries.len() + 1), trimmed)
                }
            } else {
                (format!("device-{}", entries.len() + 1), trimmed)
            };

            let Some(secret) = SecretString::new(token_part) else {
                return Err("invalid_gateway_auth_token");
            };
            if secret.as_bytes().len() < 12 {
                return Err("gateway_auth_token_too_short");
            }
            let digest = sha256_digest(secret.as_bytes());
            if entries.iter().any(|existing: &DeviceTokenEntry| {
                constant_time_eq_32(&existing.token_sha256, &digest)
            }) {
                continue;
            }
            entries.push(DeviceTokenEntry {
                device_id,
                token_sha256: digest,
            });
        }
        if entries.is_empty() {
            return Err("missing_gateway_auth_tokens");
        }
        Ok(Self { entries })
    }

    pub fn device_count(&self) -> usize {
        self.entries.len()
    }

    /// Authenticate an `Authorization: Bearer <token>` header value in constant time.
    /// Returns the matched `device_id` on success.
    pub fn verify_authorization_header(&self, header_value: &str) -> Option<&str> {
        let trimmed = header_value.trim();
        let (scheme, rest) = trimmed.split_once(' ')?;
        if !scheme.eq_ignore_ascii_case("bearer") {
            return None;
        }
        let candidate = SecretString::new(rest)?;
        let candidate_digest = sha256_digest(candidate.as_bytes());
        let mut matched: Option<&str> = None;
        for entry in &self.entries {
            if constant_time_eq_32(&entry.token_sha256, &candidate_digest) {
                matched = Some(entry.device_id.as_str());
            }
        }
        matched
    }
}

fn is_valid_device_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 32
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_and_unnamed_tokens_and_verifies_bearer() {
        let reg = TokenRegistry::parse(
            "desktop=qsgw_desktop_secret_token_123, laptop=qsgw_laptop_secret_token_456 , qsgw_plain_token_7890",
        )
        .unwrap();
        assert_eq!(reg.device_count(), 3);
        assert_eq!(
            reg.verify_authorization_header("Bearer qsgw_desktop_secret_token_123"),
            Some("desktop")
        );
        assert_eq!(
            reg.verify_authorization_header("bearer   qsgw_laptop_secret_token_456  "),
            Some("laptop")
        );
        assert_eq!(
            reg.verify_authorization_header("Bearer qsgw_plain_token_7890"),
            Some("device-3")
        );
        assert_eq!(
            reg.verify_authorization_header("Bearer qsgw_revoked_token_0000"),
            None
        );
        assert_eq!(
            reg.verify_authorization_header("Basic qsgw_desktop_secret_token_123"),
            None
        );
    }

    #[test]
    fn rejects_empty_or_short_tokens() {
        assert_eq!(
            TokenRegistry::parse(" , ").err(),
            Some("missing_gateway_auth_tokens")
        );
        assert_eq!(
            TokenRegistry::parse("desktop=short").err(),
            Some("gateway_auth_token_too_short")
        );
    }
}
