//! Shared candidate fixture: cache decision plus exact controlled compiler miss.
use kernal_api::guest::{
    Blake3Hasher, CompilerCacheStatus, CompilerGrant, CompilerOutputEvent, OperationError,
};

pub async fn proof() -> Result<(), OperationError> {
    proof_with_cache(true).await
}

/// Same controlled compiler miss path without the request-key list lowering.
/// This exists only for the allocator-fault fixture, so its trap can be shown
/// to occur while lowering output data rather than an earlier hash digest.
#[allow(dead_code)] // used by the separately selected lowering-trap fixture
pub async fn output_lowering_proof() -> Result<(), OperationError> {
    proof_with_cache(false).await
}

async fn proof_with_cache(cache: bool) -> Result<(), OperationError> {
    let grant = CompilerGrant::granted()?.ok_or(OperationError::Rejected)?;
    if CompilerGrant::granted()?.is_some() {
        return Err(OperationError::Failed);
    }
    if cache {
        match grant.cache_status(&request_key().await?)? {
            // A hit must avoid even submitting the host-owned compiler grant.
            CompilerCacheStatus::Hit => return Ok(()),
            CompilerCacheStatus::Miss => {}
        }
    }
    let mut process = grant.spawn().await?;
    if process.read_output(&mut []).await != Err(OperationError::Rejected) {
        return Err(OperationError::Failed);
    }
    let mut stdout = Blake3Hasher::new().await?;
    let mut stderr = Blake3Hasher::new().await?;
    let mut counts = [0usize; 2];
    let mut eof = [false; 2];
    let mut buffer = [0_u8; 65536];
    loop {
        let (stream, count) = match process.read_output(&mut buffer).await? {
            CompilerOutputEvent::Stdout(count) => (0, count),
            CompilerOutputEvent::Stderr(count) => (1, count),
            CompilerOutputEvent::StdoutEof => {
                eof[0] = true;
                continue;
            }
            CompilerOutputEvent::StderrEof => {
                eof[1] = true;
                continue;
            }
            CompilerOutputEvent::Exhausted => break,
            _ => return Err(OperationError::Failed),
        };
        if eof[stream] {
            return Err(OperationError::Failed);
        }
        // The self-executing native test harness writes a textual prefix and
        // suffix. Hash only fixture markers, without a second payload buffer.
        let marker = if stream == 0 { 0xf1 } else { 0xf2 };
        let hasher = if stream == 0 {
            &mut stdout
        } else {
            &mut stderr
        };
        let mut remaining = &buffer[..count];
        while let Some(start) = remaining.iter().position(|byte| *byte == marker) {
            remaining = &remaining[start..];
            let end = remaining
                .iter()
                .position(|byte| *byte != marker)
                .unwrap_or(remaining.len());
            hasher.update(&remaining[..end]).await?;
            counts[stream] += end;
            remaining = &remaining[end..];
        }
    }
    if counts != [2 * 1024 * 1024; 2] || eof != [true; 2] {
        return Err(OperationError::Failed);
    }
    for (actual, marker) in [
        (stdout.finalize().await?, 0xf1),
        (stderr.finalize().await?, 0xf2),
    ] {
        let mut expected = Blake3Hasher::new().await?;
        for _ in 0..512 {
            expected.update(&[marker; 4096]).await?;
        }
        if actual != expected.finalize().await? {
            return Err(OperationError::Failed);
        }
    }
    let exit = process.wait().await?;
    if exit.code != Some(0) || !exit.success || process.wait().await? != exit {
        return Err(OperationError::Failed);
    }
    process.close().await
}

/// Assemble the actual zccache request protocol through the public bounded
/// hash facade. The private fixture represents the host identity with the
/// grant's precomputed expected key, never with a cache path or artifact byte
/// stream. A production host would derive that identity from metadata/content
/// facts before instantiation.
async fn request_key() -> Result<[u8; 32], OperationError> {
    use zccache_hash::request_fingerprint::RequestFingerprint;

    let raw = ["-MD", "-MF-", "source.c"].map(String::from);
    let env = [("A", ""), ("Z", "last")];
    let mut cursor =
        RequestFingerprint::new("cc", ["-O2", "-O0", ""].into_iter(), &raw, "work", &env);
    let mut hash = Blake3Hasher::new().await?;
    let mut buffer = [0u8; 65536];
    let mut used = 0;
    while let Some(mut fragment) = cursor.next_fragment() {
        while !fragment.is_empty() {
            let count = fragment.len().min(buffer.len() - used);
            buffer[used..used + count].copy_from_slice(&fragment[..count]);
            used += count;
            fragment = &fragment[count..];
            if used == buffer.len() {
                hash.update(&buffer).await?;
                used = 0;
            }
        }
    }
    if used != 0 {
        hash.update(&buffer[..used]).await?;
    }
    hash.finalize().await
}
