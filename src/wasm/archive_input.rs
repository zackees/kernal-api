//! Host-owned encrypted input grant for the test-only guest archive experiment.
use super::*;
use std::io::{self, Read, Seek};

pub(crate) const MAX_HEADER_BYTES: usize = 16 * 1024 + 12;
const INPUT_KIND: u8 = 7;
const READ_HEADER: u8 = 1;

pub(crate) struct EncryptedInput {
    file: File,
    key: [u8; 16],
    header: Vec<u8>,
    ciphertext_bytes: u64,
}

impl EncryptedInput {
    /// Read only the bounded public envelope prefix before the guest starts.
    /// Retain the opened source and key on the host, never in guest memory.
    pub(crate) fn open(mut file: File, key: [u8; 16], maximum_bytes: u64) -> io::Result<Self> {
        let length = file.metadata()?.len();
        if length > maximum_bytes {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        file.rewind()?;
        let mut prefix = [0; 12];
        file.read_exact(&mut prefix)?;
        if &prefix[..8] != b"TWPV1AES" {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let header_length = u32::from_be_bytes(
            prefix[8..]
                .try_into()
                .map_err(|_| io::ErrorKind::InvalidData)?,
        ) as usize;
        if header_length > MAX_HEADER_BYTES - 12 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let header_end = (12 + header_length) as u64;
        let ciphertext_bytes = length
            .checked_sub(header_end)
            .and_then(|bytes| bytes.checked_sub(16))
            .ok_or(io::ErrorKind::InvalidData)?;
        let mut header = Vec::with_capacity(12 + header_length);
        header.extend_from_slice(&prefix);
        header.resize(12 + header_length, 0);
        file.read_exact(&mut header[12..])?;
        Ok(Self {
            file,
            key,
            header,
            ciphertext_bytes,
        })
    }
}

impl OperationHub {
    pub(crate) fn grant_encrypted_input(
        &self,
        store: u64,
        input: EncryptedInput,
    ) -> Result<OpaqueToken, HubError> {
        let token = self.create_resource_value(
            store,
            INPUT_KIND,
            READ_HEADER,
            false,
            ResourceValue::EncryptedInput(input),
        )?;
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        state
            .resources
            .get_mut(&token)
            .ok_or(HubError::Closed)?
            .reserved = false;
        Ok(token)
    }

    pub(crate) fn read_encrypted_header(
        &self,
        store: u64,
        token: u64,
        capacity: usize,
        copy: impl FnOnce(&[u8]),
    ) -> Result<usize, HubError> {
        let state = self.state.lock().map_err(|_| HubError::Closed)?;
        let resource = state
            .resources
            .get(&OpaqueToken(token))
            .ok_or(HubError::Invalid)?;
        Self::validate_resource(resource, store, INPUT_KIND, READ_HEADER)?;
        let ResourceValue::EncryptedInput(input) = &resource.value else {
            return Err(HubError::WrongKind);
        };
        if capacity < input.header.len() {
            return Err(HubError::Quota);
        }
        copy(&input.header);
        Ok(input.header.len())
    }

    pub(crate) fn abandon_encrypted_input(&self, store: u64, token: u64) -> Result<(), HubError> {
        let notifications = {
            let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
            let token = OpaqueToken(token);
            let resource = state.resources.get(&token).ok_or(HubError::Invalid)?;
            Self::validate_resource(resource, store, INPUT_KIND, READ_HEADER)?;
            Self::close_resource_with_terminal_locked(&mut state, token, Terminal::Closed)?
        };
        for notify in notifications {
            notify.notify_one();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn source(header: &[u8], tail_bytes: usize) -> File {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"TWPV1AES").unwrap();
        file.write_all(&(header.len() as u32).to_be_bytes())
            .unwrap();
        file.write_all(header).unwrap();
        file.set_len((12 + header.len() + tail_bytes) as u64)
            .unwrap();
        file
    }

    #[test]
    fn authenticated_input_header_is_bounded_scoped_and_revocable() {
        let header = b"{ \"original\": true }";
        let input = EncryptedInput::open(
            source(header, 17 * 1024 * 1024 + 16),
            [9; 16],
            512 * 1024 * 1024,
        )
        .unwrap();
        assert_eq!(input.ciphertext_bytes, 17 * 1024 * 1024);
        assert_eq!(
            input.file.metadata().unwrap().len(),
            (12 + header.len() + 17 * 1024 * 1024 + 16) as u64
        );
        let hub = OperationHub::new(1, 2).unwrap();
        let token = hub.grant_encrypted_input(1, input).unwrap().wire();
        assert!(hub
            .read_encrypted_header(2, token, MAX_HEADER_BYTES, |_| panic!("foreign copy"))
            .is_err());
        assert!(hub
            .read_encrypted_header(1, token, 12, |_| panic!("short copy"))
            .is_err());
        assert_eq!(
            hub.read_encrypted_header(1, token, MAX_HEADER_BYTES, |bytes| assert_eq!(
                &bytes[12..],
                header
            ))
            .unwrap(),
            12 + header.len()
        );
        assert!(hub.abandon_encrypted_input(2, token).is_err());
        hub.abandon_encrypted_input(1, token).unwrap();
        assert!(hub
            .read_encrypted_header(1, token, MAX_HEADER_BYTES, |_| panic!("stale copy"))
            .is_err());
        assert_eq!(hub.snapshot().live_resources, 0);
    }

    #[test]
    fn authenticated_input_rejects_oversized_header_input_and_truncated_tag() {
        assert!(EncryptedInput::open(source(b"{}", 15), [0; 16], 1024).is_err());
        assert!(EncryptedInput::open(source(b"{}", 16), [0; 16], 1).is_err());
        assert!(
            EncryptedInput::open(source(&vec![b' '; 16 * 1024 + 1], 16), [0; 16], 1024 * 1024)
                .is_err()
        );
        let mut wrong = source(b"{}", 16);
        wrong.rewind().unwrap();
        wrong.write_all(b"BADMAGIC").unwrap();
        assert!(EncryptedInput::open(wrong, [0; 16], 1024).is_err());
    }
}
