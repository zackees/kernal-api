//! Actual public hash capability control; not zccache policy acceptance.
use kernal_api::guest::{self as kernel, Blake3Hasher, OperationError};

const EXPECTED: [u8; 32] = [
    0x35, 0x70, 0x71, 0xd5, 0x54, 0xb8, 0x55, 0x45, 0xe7, 0xab, 0xc6, 0x4c, 0x4d, 0x5b, 0xfe, 0x68,
    0x5c, 0x0a, 0xa5, 0xf7, 0x13, 0x27, 0xe4, 0x11, 0xba, 0xf7, 0x82, 0xa8, 0xae, 0xed, 0x10, 0x2c,
];

async fn proof() -> Result<(), OperationError> {
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

#[export_name = "kernal-api-run"]
pub extern "C" fn kernal_api_run() -> u32 {
    u32::from(kernel::run(proof()).is_err())
}

fn main() {
    if kernal_api_run() != 0 {
        std::process::exit(1);
    }
}
