//! Public hash control and actual zccache key encoding; not full policy acceptance.
use kernal_api::guest::{Blake3Hasher, OperationError};

const EXPECTED: [u8; 32] = [
    0x35, 0x70, 0x71, 0xd5, 0x54, 0xb8, 0x55, 0x45, 0xe7, 0xab, 0xc6, 0x4c, 0x4d, 0x5b, 0xfe, 0x68,
    0x5c, 0x0a, 0xa5, 0xf7, 0x13, 0x27, 0xe4, 0x11, 0xba, 0xf7, 0x82, 0xa8, 0xae, 0xed, 0x10, 0x2c,
];

pub async fn proof() -> Result<(), OperationError> {
    request_key_proof().await?;
    let empty = Blake3Hasher::new().await?.finalize().await?;
    if empty
        != [
            0xaf, 0x13, 0x49, 0xb9, 0xf5, 0xf9, 0xa1, 0xa6, 0xa0, 0x40, 0x4d, 0xea, 0x36, 0xdc,
            0xc9, 0x49, 0x9b, 0xcb, 0x25, 0xc9, 0xad, 0xc1, 0x12, 0xb7, 0xcc, 0x9a, 0x93, 0xca,
            0xe4, 0x1f, 0x32, 0x62,
        ]
    {
        return Err(OperationError::Failed);
    }
    let chunk = [0x5a; 65536];
    for width in [65536, 4093] {
        let mut hash = Blake3Hasher::new().await?;
        let mut remaining = 64 * 1024 * 1024;
        while remaining != 0 {
            let count = remaining.min(width);
            hash.update(&chunk[..count]).await?;
            remaining -= count;
        }
        if hash.finalize().await? != EXPECTED {
            return Err(OperationError::Failed);
        }
    }
    // Drop must reclaim a created resource without requiring another operation slot.
    drop(Blake3Hasher::new().await?);
    Ok(())
}

async fn request_key_proof() -> Result<(), OperationError> {
    use zccache_hash::request_fingerprint::RequestFingerprint;

    let raw = ["-MD", "-MF-", "source.c"].map(String::from);
    let env = [("A", ""), ("Z", "last")];
    // Independent b3sum of the literal v2 protocol fixture, not a second encoder.
    let expected = [
        0xdb, 0xed, 0xcb, 0xc5, 0x83, 0xf5, 0x1d, 0x14, 0x3b, 0xae, 0xb1, 0x9d, 0xbe, 0xac, 0xfd,
        0x3f, 0xa0, 0xcb, 0x40, 0x8c, 0xe8, 0x39, 0x9b, 0x56, 0xce, 0xc7, 0x22, 0x24, 0xd8, 0x54,
        0xe9, 0x93,
    ];
    for width in [1, 7, 65536] {
        let mut cursor =
            RequestFingerprint::new("cc", ["-O2", "-O0", ""].into_iter(), &raw, "work", &env);
        let mut hash = Blake3Hasher::new().await?;
        let mut buffer = [0u8; 65536];
        let mut used = 0;
        while let Some(mut fragment) = cursor.next_fragment() {
            while !fragment.is_empty() {
                let count = fragment.len().min(width - used);
                buffer[used..used + count].copy_from_slice(&fragment[..count]);
                used += count;
                fragment = &fragment[count..];
                if used == width {
                    hash.update(&buffer[..used]).await?;
                    used = 0;
                }
            }
        }
        if used != 0 {
            hash.update(&buffer[..used]).await?;
        }
        if hash.finalize().await? != expected {
            return Err(OperationError::Failed);
        }
    }
    Ok(())
}
