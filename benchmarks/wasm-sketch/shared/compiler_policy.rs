//! Shared candidate fixture: exact host command, bounded pull, hash, exit, close.
//! This is not yet the zccache artifact-cache hit/miss workflow.
use kernal_api::guest::{Blake3Hasher, CompilerGrant, CompilerOutputEvent, OperationError};

pub async fn proof() -> Result<(), OperationError> {
    let grant = CompilerGrant::granted()?.ok_or(OperationError::Rejected)?;
    if CompilerGrant::granted()?.is_some() {
        return Err(OperationError::Failed);
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
