//! Portable subset of extension2's envelope/inventory decisions, using only
//! synthetic identities. This is not the upstream crate or its complete policy.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Deserialize;
use std::collections::BTreeSet;

pub const MAX_HEADER: usize = 16 * 1024 + 12;
pub const PAYLOAD_BYTES: u64 = 17 * 1024 * 1024;

/// Exercise the unchanged portable extension2 preview-feed policy before this
/// guest consumes any archive authority.  The feed is a deterministic fixture;
/// no network capability is granted to the guest.
pub fn validates_real_extension2_preview_policy() -> bool {
    const FEED_URL: &str = "https://techwatchproject.github.io/extension/manifest.dev.json";
    let commit = "a".repeat(40);
    let digest = "b".repeat(64);
    let feed = format!(
        r#"{{"schemaVersion":1,"addonId":"{{92875af4-bd83-4876-92d9-7c6d716b002f}}","previews":[{{"version":"4.8.7","commit":"{commit}","commitTimestamp":42,"url":"preview/tw-orange-preview-v1-4.8.7-{commit}-{digest}.zip.aes","sha256":"{digest}","size":1234,"algorithm":"AES-128-GCM","keyId":"0123456789abcdef","testSuites":["unit","playwright"]}}]}}"#,
    );
    if tw_orange_preview_policy::validate_preview_feed_json(feed.as_bytes(), FEED_URL).is_err() {
        return false;
    }
    let hostile = feed.replace("preview/tw-orange", "https://attacker.test/preview/tw-orange");
    tw_orange_preview_policy::validate_preview_feed_json(hostile.as_bytes(), FEED_URL).is_err()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Header {
    schema_version: u32,
    algorithm: String,
    version: String,
    commit: String,
    key_id: String,
    nonce: String,
}

/// Validate original prefix/header bytes without reserializing authenticated
/// data. Ciphertext and the final tag must not be included in this record.
pub fn validate_header(bytes: &[u8]) -> bool {
    validated_nonce(bytes).is_some()
}

/// Decode the explicit crypto nonce only after the complete identity policy.
pub fn validated_nonce(bytes: &[u8]) -> Option<[u8; 12]> {
    if bytes.len() < 12 || bytes.len() > MAX_HEADER || &bytes[..8] != b"TWPV1AES" {
        return None;
    }
    let length = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    if length != bytes.len() - 12 {
        return None;
    }
    let Ok(header) = serde_json::from_slice::<Header>(&bytes[12..]) else {
        return None;
    };
    let valid = header.schema_version == 1
        && header.algorithm == "AES-128-GCM"
        && header.version == "synthetic-1"
        && header.commit == "synthetic-commit"
        && header.key_id == "synthetic-key";
    if !valid {
        return None;
    }
    STANDARD.decode(header.nonce).ok()?.try_into().ok()
}

#[derive(Default)]
pub struct Inventory {
    names: BTreeSet<String>,
    total: u64,
}

impl Inventory {
    pub fn accept(&mut self, name: &str, bytes: u64) -> bool {
        // Extension2 is stricter than the general-purpose native extractor.
        if name.is_empty()
            || name.len() > 4096
            || name.starts_with('/')
            || name.contains('\\')
            || name.split('/').any(|part| matches!(part, "" | "." | ".."))
            || self.names.len() >= 16_384
            || self.names.contains(name)
            || bytes > 32 * 1024 * 1024
        {
            return false;
        }
        let Some(total) = self.total.checked_add(bytes) else {
            return false;
        };
        if total > 512 * 1024 * 1024 {
            return false;
        }
        self.names.insert(name.to_owned());
        self.total = total;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(json: &[u8]) -> Vec<u8> {
        let mut bytes = b"TWPV1AES".to_vec();
        bytes.extend_from_slice(&(json.len() as u32).to_be_bytes());
        bytes.extend_from_slice(json);
        bytes
    }

    #[test]
    fn original_header_accepts_whitespace_but_rejects_wrong_identity_and_lengths() {
        let json = br#"{ "schemaVersion":1, "algorithm":"AES-128-GCM", "version":"synthetic-1", "commit":"synthetic-commit", "keyId":"synthetic-key", "nonce":"AAAAAAAAAAAAAAAA" }"#;
        let bytes = envelope(json);
        assert!(validate_header(&bytes));
        for field in [
            "schemaVersion",
            "algorithm",
            "version",
            "commit",
            "keyId",
            "nonce",
        ] {
            let mut header: serde_json::Value = serde_json::from_slice(json).unwrap();
            header[field] = serde_json::Value::Null;
            assert!(!validate_header(&envelope(
                &serde_json::to_vec(&header).unwrap()
            )));
            header[field] = if field == "schemaVersion" {
                serde_json::json!(2)
            } else {
                serde_json::json!("wrong")
            };
            assert!(!validate_header(&envelope(
                &serde_json::to_vec(&header).unwrap()
            )));
        }
        for length in 0..bytes.len() {
            assert!(!validate_header(&bytes[..length]));
        }
        for length in [0, 11, 13] {
            let mut header: serde_json::Value = serde_json::from_slice(json).unwrap();
            header["nonce"] = serde_json::json!(STANDARD.encode(vec![0; length]));
            assert!(!validate_header(&envelope(
                &serde_json::to_vec(&header).unwrap()
            )));
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(!validate_header(&trailing));
        let mut wrong_magic = bytes;
        wrong_magic[0] ^= 1;
        assert!(!validate_header(&wrong_magic));
        assert!(!validate_header(&envelope(&vec![b' '; 16 * 1024 + 1])));
    }

    #[test]
    fn inventory_rejects_duplicate_unsafe_and_oversized_entries() {
        let mut inventory = Inventory::default();
        for name in [
            "",
            "/absolute",
            "a\\b",
            "a//b",
            "a/./b",
            "a/../b",
            "trailing/",
        ] {
            assert!(!inventory.accept(name, 1), "{name:?}");
        }
        assert!(!inventory.accept("payload", 32 * 1024 * 1024 + 1));
        assert!(!inventory.accept(&"a".repeat(4097), 0));
        assert!(inventory.accept("payload", PAYLOAD_BYTES));
        assert!(!inventory.accept("payload", 0));
        for i in 0..15 {
            assert!(inventory.accept(&format!("chunk-{i}"), 32 * 1024 * 1024));
        }
        assert!(!inventory.accept("over-total", 32 * 1024 * 1024));
        let mut count = Inventory::default();
        for i in 0..16_384 {
            assert!(count.accept(&format!("empty-{i}"), 0));
        }
        assert!(!count.accept("one-too-many", 0));
    }

    #[test]
    fn real_extension2_preview_policy_accepts_only_its_official_encrypted_asset() {
        assert!(validates_real_extension2_preview_policy());
    }
}
