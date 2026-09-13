//! Header/authentication/inventory controls and the bounded streaming proof.
use kernal_api::guest::{self as kernel, EncryptedArchive, OperationError};
use kernal_extension2_guest_proof::policy;

async fn header() -> Result<(EncryptedArchive, [u8; 12]), OperationError> {
    let encrypted = EncryptedArchive::granted()?.ok_or(OperationError::Rejected)?;
    if EncryptedArchive::granted()?.is_some() {
        return Err(OperationError::Rejected);
    }
    let mut header = [0; policy::MAX_HEADER];
    let count = encrypted.read_header(&mut header).await?;
    let nonce = policy::validated_nonce(&header[..count]).ok_or(OperationError::Rejected)?;
    Ok((encrypted, nonce))
}

#[cfg(feature = "header-proof")]
async fn proof() -> Result<(), OperationError> {
    // Exercise the complete admitted async lifecycle even though the bounded
    // header copy itself completes synchronously.
    kernel::sleep(1).await?;
    drop(header().await?);
    if EncryptedArchive::granted()?.is_some() {
        return Err(OperationError::Rejected);
    }
    Ok(())
}

#[cfg(all(
    feature = "auth-proof",
    not(any(feature = "header-proof", feature = "inventory-proof"))
))]
async fn proof() -> Result<(), OperationError> {
    let (encrypted, nonce) = header().await?;
    encrypted.authenticate(nonce).await?.close().await
}

#[cfg(all(feature = "inventory-proof", not(feature = "header-proof")))]
async fn proof() -> Result<(), OperationError> {
    let (encrypted, nonce) = header().await?;
    let mut archive = encrypted.authenticate(nonce).await?;
    let mut inventory = policy::Inventory::default();
    let mut entries = 0;
    while let Some(entry) = archive.next_entry().await? {
        if !inventory.accept(entry.name(), entry.uncompressed_bytes())
            || entry.name() != "payload"
            || entry.uncompressed_bytes() != policy::PAYLOAD_BYTES
        {
            return Err(OperationError::Rejected);
        }
        entries += 1;
    }
    archive.close().await?;
    if entries != 1 {
        return Err(OperationError::Rejected);
    }
    Ok(())
}

#[cfg(not(any(
    feature = "header-proof",
    feature = "auth-proof",
    feature = "inventory-proof"
)))]
async fn proof() -> Result<(), OperationError> {
    let (encrypted, nonce) = header().await?;
    // The host retains its key and exact original AAD. No plaintext archive
    // authority may exist until this asynchronous operation succeeds.
    let mut archive = encrypted.authenticate(nonce).await?;
    let mut inventory = policy::Inventory::default();
    let mut payloads = 0;
    while let Some(entry) = archive.next_entry().await? {
        if !inventory.accept(entry.name(), entry.uncompressed_bytes())
            || entry.name() != "payload"
            || entry.uncompressed_bytes() != policy::PAYLOAD_BYTES
        {
            return Err(OperationError::Rejected);
        }
        let stream = entry.open().await?;
        let mut chunk = [0; 64 * 1024];
        let mut total = 0_u64;
        loop {
            let count = stream
                .read_chunk(chunk.len() as u32)?
                .read_into(&mut chunk)
                .await?;
            if count == 0 {
                break;
            }
            if chunk[..count].iter().any(|byte| *byte != 0x5a) {
                return Err(OperationError::Failed);
            }
            total = total
                .checked_add(count as u64)
                .ok_or(OperationError::Rejected)?;
            if total > policy::PAYLOAD_BYTES {
                return Err(OperationError::Rejected);
            }
        }
        stream.close().await?;
        if total != policy::PAYLOAD_BYTES {
            return Err(OperationError::Failed);
        }
        payloads += 1;
    }
    archive.close().await?;
    if payloads != 1 {
        return Err(OperationError::Rejected);
    }
    Ok(())
}

#[export_name = "kernal-api-run"]
pub extern "C" fn kernal_api_run() -> u32 {
    match kernel::run(proof()) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn main() {
    if kernal_api_run() != 0 {
        std::process::exit(1);
    }
}
