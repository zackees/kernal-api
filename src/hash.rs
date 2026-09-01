//! Kernel-owned BLAKE3 content hashing.
//!
//! The concrete operations here deliberately expose only `kernal-api`
//! semantic values. The BLAKE3 implementation remains a private dependency,
//! so callers can hash bytes, readers, and files without coupling their APIs
//! to implementation-crate types.

use std::fmt;
use std::io::{self, Read};
use std::path::Path;

/// The number of bytes in a BLAKE3 content digest.
pub const BLAKE3_DIGEST_LENGTH: usize = 32;

const READ_BUFFER_LENGTH: usize = 64 * 1024;

/// An opaque, kernel-owned BLAKE3 content digest.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Blake3Digest([u8; BLAKE3_DIGEST_LENGTH]);

impl Blake3Digest {
    /// Borrow the canonical 32-byte digest encoding.
    pub fn as_bytes(&self) -> &[u8; BLAKE3_DIGEST_LENGTH] {
        &self.0
    }

    /// Render the canonical lower-case hexadecimal encoding.
    pub fn to_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";

        let mut rendered = String::with_capacity(BLAKE3_DIGEST_LENGTH * 2);
        for byte in self.0 {
            rendered.push(HEX[usize::from(byte >> 4)] as char);
            rendered.push(HEX[usize::from(byte & 0x0f)] as char);
        }
        rendered
    }
}

impl fmt::Display for Blake3Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

/// Hash an in-memory byte slice with BLAKE3.
pub fn blake3_bytes(bytes: &[u8]) -> Blake3Digest {
    Blake3Digest(*blake3::hash(bytes).as_bytes())
}

/// Bounds for a synchronous reader or file hashing operation.
///
/// These operations are synchronous and intentionally do not introduce an
/// async cancellation contract. Run them through [`crate::async_engine`]'s
/// blocking lane when an async caller needs scheduling isolation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Blake3ReadOptions {
    maximum_bytes: Option<u64>,
}

impl Blake3ReadOptions {
    /// Create an unbounded reader/file hashing policy.
    pub const fn new() -> Self {
        Self {
            maximum_bytes: None,
        }
    }

    /// Reject input that contains more than `maximum_bytes` bytes.
    pub const fn maximum_bytes(mut self, maximum_bytes: u64) -> Self {
        self.maximum_bytes = Some(maximum_bytes);
        self
    }

    /// Return the configured input limit, if any.
    pub const fn maximum_byte_count(self) -> Option<u64> {
        self.maximum_bytes
    }
}

/// Stable category for a reader or file hashing failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Blake3HashErrorKind {
    /// Opening a requested file failed.
    Open,
    /// Reading requested content failed.
    Read,
    /// The input contained more bytes than the configured maximum.
    SizeLimitExceeded {
        /// The configured maximum number of input bytes.
        maximum_bytes: u64,
    },
}

/// Failure from a reader or file BLAKE3 operation.
#[derive(Debug)]
pub struct Blake3HashError {
    kind: Blake3HashErrorKind,
    source: Option<io::Error>,
}

impl Blake3HashError {
    fn open(source: io::Error) -> Self {
        Self {
            kind: Blake3HashErrorKind::Open,
            source: Some(source),
        }
    }

    fn read(source: io::Error) -> Self {
        Self {
            kind: Blake3HashErrorKind::Read,
            source: Some(source),
        }
    }

    fn size_limit_exceeded(maximum_bytes: u64) -> Self {
        Self {
            kind: Blake3HashErrorKind::SizeLimitExceeded { maximum_bytes },
            source: None,
        }
    }

    /// Return the stable semantic failure category.
    pub const fn kind(&self) -> Blake3HashErrorKind {
        self.kind
    }

    /// Return the native I/O category when an open or read caused the error.
    pub fn io_error_kind(&self) -> Option<io::ErrorKind> {
        self.source.as_ref().map(io::Error::kind)
    }
}

impl fmt::Display for Blake3HashError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            Blake3HashErrorKind::Open => formatter.write_str("failed to open BLAKE3 input"),
            Blake3HashErrorKind::Read => formatter.write_str("failed to read BLAKE3 input"),
            Blake3HashErrorKind::SizeLimitExceeded { maximum_bytes } => {
                write!(
                    formatter,
                    "BLAKE3 input exceeds the {maximum_bytes}-byte limit"
                )
            }
        }
    }
}

impl std::error::Error for Blake3HashError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

/// Hash content from a blocking byte reader with BLAKE3.
///
/// When a maximum is configured, the operation reads at most one byte beyond
/// the accepted prefix to distinguish EOF from an oversized input. That byte
/// is never incorporated into the returned digest.
pub fn blake3_reader(
    mut reader: impl Read,
    options: Blake3ReadOptions,
) -> Result<Blake3Digest, Blake3HashError> {
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; READ_BUFFER_LENGTH];
    let mut bytes_hashed = 0_u64;

    loop {
        let read_length = next_read_length(options.maximum_bytes, bytes_hashed);
        let read = reader
            .read(&mut buffer[..read_length])
            .map_err(Blake3HashError::read)?;
        if read == 0 {
            return Ok(Blake3Digest(*hasher.finalize().as_bytes()));
        }

        let read = u64::try_from(read).expect("buffer length always fits in u64");
        if let Some(maximum_bytes) = options.maximum_bytes {
            if read > maximum_bytes.saturating_sub(bytes_hashed) {
                return Err(Blake3HashError::size_limit_exceeded(maximum_bytes));
            }
            bytes_hashed += read;
        }

        hasher.update(&buffer[..usize::try_from(read).expect("read length fits usize")]);
    }
}

/// Hash a file's content with BLAKE3.
pub fn blake3_file(
    path: impl AsRef<Path>,
    options: Blake3ReadOptions,
) -> Result<Blake3Digest, Blake3HashError> {
    let file = std::fs::File::open(path).map_err(Blake3HashError::open)?;
    blake3_reader(file, options)
}

fn next_read_length(maximum_bytes: Option<u64>, bytes_hashed: u64) -> usize {
    let Some(maximum_bytes) = maximum_bytes else {
        return READ_BUFFER_LENGTH;
    };

    let remaining = maximum_bytes.saturating_sub(bytes_hashed);
    let one_more_than_remaining =
        remaining.min(u64::try_from(READ_BUFFER_LENGTH - 1).expect("buffer length fits u64")) + 1;
    usize::try_from(one_more_than_remaining).expect("bounded buffer length fits usize")
}
