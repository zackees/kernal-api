//! Shared candidate fixture: an exact, controlled compiler run.
use kernal_api::guest::{Blake3Hasher, CompilerGrant, CompilerOutputEvent, OperationError};
#[cfg(feature = "compiler-artifact-output")]
use kernal_api::guest::{Blob, OutputFile};

// Keep the representative parser in this guest path rather than validating it
// only through the separate hash probe. Both runtime candidates include this
// file, so a compiler miss now exercises the same explicit-host Rustc policy
// before it asks the host to spawn.
#[path = "rustc_policy.rs"]
mod rustc_policy;

pub async fn proof() -> Result<(), OperationError> {
    controlled_run().await
}

/// The allocator-fault fixture's entry point: the same controlled run, so its
/// trap can be shown to occur while lowering output data. It used to differ
/// from `proof` by skipping a cache-key lowering; the compiler-artifact cache
/// experiment that key fed is gone, so the two are now the same run.
#[allow(dead_code)] // used by the separately selected lowering-trap fixture
pub async fn output_lowering_proof() -> Result<(), OperationError> {
    controlled_run().await
}

async fn controlled_run() -> Result<(), OperationError> {
    if !rustc_policy::proof() {
        return Err(OperationError::Failed);
    }
    let grant = CompilerGrant::granted()?.ok_or(OperationError::Rejected)?;
    if CompilerGrant::granted()?.is_some() {
        return Err(OperationError::Failed);
    }
    // The Core fixture persists only the verified compiler payload through an
    // embedding-selected exact destination. The Component candidate shares
    // this policy but deliberately has no output world, so it remains a
    // separate experiment rather than gaining a hidden storage capability.
    #[cfg(feature = "compiler-artifact-output")]
    let (output, artifact) = (
        OutputFile::granted()?.ok_or(OperationError::Rejected)?,
        Blob::create().await?,
    );
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
            // Persist the same verified marker runs, not host textual output
            // or a second whole-output buffer. Each bounded write waits for
            // existing blob capacity accounting before another read.
            #[cfg(feature = "compiler-artifact-output")]
            artifact.write_chunk(&remaining[..end])?.wait().await?;
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
    process.close().await?;
    #[cfg(feature = "compiler-artifact-output")]
    {
        artifact.seal().await?;
        output.write_blob(&artifact).await?;
    }
    Ok(())
}
