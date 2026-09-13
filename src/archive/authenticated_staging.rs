//! Test-gated native experiment. No guest resource or public API yet.
use std::fs::File;
use std::io::{self, Seek, Write};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use openssl::symm::{Cipher, Crypter, Mode};

const CHUNK: usize = 64 * 1024;

#[derive(Debug)]
struct Reservation {
    used: Arc<AtomicU64>,
    bytes: u64,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.used.fetch_sub(self.bytes, Ordering::SeqCst);
    }
}
#[derive(Debug)]
struct Authenticated {
    file: File,
    _reservation: Reservation,
}

struct Pending {
    file: File,
    cipher: Crypter,
    expected: u64,
    written: u64,
    reservation: Reservation,
}

// There is intentionally no read, seek, path, or file accessor on this type.
struct Authentication {
    pending: Option<Pending>,
}

impl Authentication {
    fn begin(
        key: &[u8; 16],
        nonce: &[u8; 12],
        aad: &[u8],
        expected: u64,
        limit: u64,
        used: &Arc<AtomicU64>,
    ) -> io::Result<Self> {
        if expected > limit || aad.len() > 16 * 1024 + 12 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        used.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            current.checked_add(expected).filter(|next| *next <= limit)
        })
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "staging quota exhausted"))?;
        let reservation = Reservation {
            used: Arc::clone(used),
            bytes: expected,
        };
        let mut cipher = Crypter::new(Cipher::aes_128_gcm(), Mode::Decrypt, key, Some(nonce))
            .map_err(io::Error::other)?;
        cipher.aad_update(aad).map_err(io::Error::other)?;
        Ok(Self {
            pending: Some(Pending {
                file: tempfile::tempfile()?,
                cipher,
                expected,
                written: 0,
                reservation,
            }),
        })
    }

    fn update(&mut self, ciphertext: &[u8]) -> io::Result<()> {
        // Taking ownership poisons the operation on any error, closing its
        // private staging even when the caller retains this failed wrapper.
        let mut state = self.pending.take().ok_or(io::ErrorKind::BrokenPipe)?;
        if ciphertext.len() > CHUNK || ciphertext.len() as u64 > state.expected - state.written {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let mut plaintext = [0; CHUNK + 16];
        let count = state
            .cipher
            .update(ciphertext, &mut plaintext)
            .map_err(io::Error::other)?;
        if count != ciphertext.len() {
            return Err(io::Error::other("unexpected GCM output length"));
        }
        state.file.write_all(&plaintext[..count])?;
        state.written += count as u64;
        self.pending = Some(state);
        Ok(())
    }

    fn authenticate(mut self, tag: &[u8; 16]) -> io::Result<Authenticated> {
        let mut state = self.pending.take().ok_or(io::ErrorKind::BrokenPipe)?;
        if state.written != state.expected {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        state.cipher.set_tag(tag).map_err(io::Error::other)?;
        let mut tail = [0; 16];
        let count = state
            .cipher
            .finalize(&mut tail)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "authentication failed"))?;
        if count != 0 {
            return Err(io::Error::other("unexpected GCM final output"));
        }
        state.file.flush()?;
        state.file.rewind()?;
        Ok(Authenticated {
            file: state.file,
            _reservation: state.reservation,
        })
    }
}

#[test]
fn nist_single_message_vector_requires_valid_tag_before_returning_file() {
    use std::io::Read;
    let ciphertext = [
        0x03, 0x88, 0xda, 0xce, 0x60, 0xb6, 0xa3, 0x92, 0xf3, 0x28, 0xc2, 0xb9, 0x71, 0xb2, 0xfe,
        0x78,
    ];
    let tag = [
        0xab, 0x6e, 0x47, 0xd4, 0x2c, 0xec, 0x13, 0xbd, 0xf5, 0x3a, 0x67, 0xb2, 0x12, 0x57, 0xbd,
        0xdf,
    ];
    for corrupt in [false, true] {
        let used = Arc::new(AtomicU64::new(0));
        let mut pending = Authentication::begin(&[0; 16], &[0; 12], &[], 16, 16, &used).unwrap();
        assert_eq!(used.load(Ordering::SeqCst), 16);
        assert!(Authentication::begin(&[0; 16], &[0; 12], &[], 1, 16, &used).is_err());
        pending.update(&ciphertext[..7]).unwrap();
        pending.update(&ciphertext[7..]).unwrap();
        let mut supplied = tag;
        supplied[0] ^= u8::from(corrupt);
        let result = pending.authenticate(&supplied);
        if corrupt {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        } else {
            let mut plain = [1; 16];
            result.unwrap().file.read_exact(&mut plain).unwrap();
            assert_eq!(plain, [0; 16]);
        }
        assert_eq!(used.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn quota_and_update_errors_never_leave_pending_staging() {
    let used = Arc::new(AtomicU64::new(0));
    assert!(Authentication::begin(&[0; 16], &[0; 12], &[], 17, 16, &used).is_err());
    let mut pending = Authentication::begin(&[0; 16], &[0; 12], &[], 16, 16, &used).unwrap();
    assert!(pending.update(&[0; 17]).is_err());
    assert!(pending.pending.is_none());
    assert_eq!(used.load(Ordering::SeqCst), 0);
    assert_eq!(
        pending.update(&[]).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    let pending = Authentication::begin(&[0; 16], &[0; 12], &[], 16, 16, &used).unwrap();
    assert_eq!(
        pending.authenticate(&[0; 16]).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    assert_eq!(used.load(Ordering::SeqCst), 0);
    drop(Authentication::begin(&[0; 16], &[0; 12], &[], 16, 16, &used).unwrap());
    assert_eq!(used.load(Ordering::SeqCst), 0);
}

#[test]
fn large_single_message_authenticates_aad_ciphertext_and_tag_with_bounded_updates() {
    use std::io::{Read, SeekFrom};
    const LENGTH: u64 = 17 * 1024 * 1024;
    for mutation in 0..4 {
        let used = Arc::new(AtomicU64::new(0));
        let key = [7; 16];
        let nonce = [9; 12];
        let aad = b"synthetic envelope";
        let supplied_aad: &[u8] = if mutation == 1 {
            b"different envelope"
        } else {
            aad
        };
        let mut pending =
            Authentication::begin(&key, &nonce, supplied_aad, LENGTH, LENGTH, &used).unwrap();
        let mut encoder =
            Crypter::new(Cipher::aes_128_gcm(), Mode::Encrypt, &key, Some(&nonce)).unwrap();
        encoder.aad_update(aad).unwrap();
        let plain = [0x5a; CHUNK];
        let mut encrypted = [0; CHUNK + 16];
        for index in 0..LENGTH / CHUNK as u64 {
            let count = encoder.update(&plain, &mut encrypted).unwrap();
            if mutation == 2 && index == 0 {
                encrypted[0] ^= 1;
            }
            pending.update(&encrypted[..count]).unwrap();
            assert_eq!(used.load(Ordering::SeqCst), LENGTH);
        }
        assert_eq!(encoder.finalize(&mut encrypted).unwrap(), 0);
        let mut tag = [0; 16];
        encoder.get_tag(&mut tag).unwrap();
        if mutation == 3 {
            tag[0] ^= 1;
        }
        let result = pending.authenticate(&tag);
        if mutation == 0 {
            let mut authenticated = result.unwrap();
            assert_eq!(used.load(Ordering::SeqCst), LENGTH);
            assert_eq!(authenticated.file.metadata().unwrap().len(), LENGTH);
            authenticated
                .file
                .seek(SeekFrom::Start(LENGTH - CHUNK as u64))
                .unwrap();
            let mut tail = [0; CHUNK];
            authenticated.file.read_exact(&mut tail).unwrap();
            assert_eq!(tail, plain);
        } else {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
        assert_eq!(used.load(Ordering::SeqCst), 0);
    }
}
