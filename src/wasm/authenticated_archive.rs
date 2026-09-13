//! Native registry experiment only; no generated guest operations yet.
use super::*;
use crate::archive::authenticated_staging::{Authenticated, Authentication};
use std::io;

const ARCHIVE_KIND: u8 = 6;
const EXTRACT_RIGHT: u8 = 1;

impl OperationHub {
    fn register_authenticated_archive(
        &self,
        store: u64,
        archive: Authenticated,
    ) -> Result<OpaqueToken, HubError> {
        // A file authenticated under a different quota cannot be imported to
        // bypass this logical sketch's storage ceiling.
        if !archive.belongs_to(&self.staging_budget) {
            return Err(HubError::WrongRights);
        }
        let token = self.create_resource_value(
            store,
            ARCHIVE_KIND,
            EXTRACT_RIGHT,
            false,
            ResourceValue::AuthenticatedArchive(archive),
        )?;
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        state
            .resources
            .get_mut(&token)
            .ok_or(HubError::Closed)?
            .reserved = false;
        Ok(token)
    }

    fn extract_authenticated_archive(
        &self,
        store: u64,
        token: OpaqueToken,
        destination: &std::path::Path,
        limits: crate::archive::ExtractionLimits,
    ) -> io::Result<()> {
        let invalid = |error: HubError| io::Error::other(format!("archive resource: {error:?}"));
        let mut state = self.state.lock().map_err(|_| invalid(HubError::Closed))?;
        let slot = state
            .resources
            .get_mut(&token)
            .ok_or_else(|| invalid(HubError::Invalid))?;
        Self::validate_resource(slot, store, ARCHIVE_KIND, EXTRACT_RIGHT).map_err(invalid)?;
        let ResourceValue::AuthenticatedArchive(archive) =
            std::mem::replace(&mut slot.value, ResourceValue::Synthetic)
        else {
            return Err(invalid(HubError::WrongKind));
        };
        let notifications =
            Self::close_resource_with_terminal_locked(&mut state, token, Terminal::Closed)
                .map_err(invalid)?;
        drop(state);
        for notify in notifications {
            notify.notify_one();
        }
        // All filesystem work is outside the hub mutex. The consumed file
        // retains the storage charge until the extractor returns.
        archive.extract(destination, limits)
    }
}

fn authenticated(hub: &OperationHub) -> Authenticated {
    let mut pending =
        Authentication::begin(&[0; 16], &[0; 12], &[], 16, &hub.staging_budget).unwrap();
    assert_eq!(hub.snapshot().live_resources, 0);
    pending
        .update(&[
            0x03, 0x88, 0xda, 0xce, 0x60, 0xb6, 0xa3, 0x92, 0xf3, 0x28, 0xc2, 0xb9, 0x71, 0xb2,
            0xfe, 0x78,
        ])
        .unwrap();
    pending
        .authenticate(&[
            0xab, 0x6e, 0x47, 0xd4, 0x2c, 0xec, 0x13, 0xbd, 0xf5, 0x3a, 0x67, 0xb2, 0x12, 0x57,
            0xbd, 0xdf,
        ])
        .unwrap()
}

#[test]
fn authenticated_archive_registry_rejects_foreign_owners_and_stale_tokens() {
    let hub = OperationHub::new(8, 2).unwrap();
    let foreign = OperationHub::new(8, 2).unwrap();
    let token = hub
        .register_authenticated_archive(1, authenticated(&hub))
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    let limits = crate::archive::ExtractionLimits::default();
    assert!(hub
        .extract_authenticated_archive(2, token, &output, limits)
        .is_err());
    assert!(foreign
        .extract_authenticated_archive(1, token, &output, limits)
        .is_err());
    assert!(!output.exists());
    assert_eq!(hub.staging_budget.used(), 16);
    assert_eq!(hub.snapshot().live_resources, 1);
    hub.close_resource(token).unwrap();
    assert_eq!(hub.staging_budget.used(), 0);
    let replacement = hub
        .register_authenticated_archive(1, authenticated(&hub))
        .unwrap();
    assert_ne!(token, replacement);
    assert!(hub
        .extract_authenticated_archive(1, token, &output, limits)
        .is_err());
    assert_eq!(hub.staging_budget.used(), 16);
    // The authenticated NIST plaintext is not a ZIP; extraction failure must
    // still consume the live generation and release its file/reservation.
    assert!(hub
        .extract_authenticated_archive(1, replacement, &output, limits)
        .is_err());
    assert_eq!(hub.staging_budget.used(), 0);
    assert_eq!(hub.snapshot().live_resources, 0);
}

#[test]
fn authenticated_archive_registry_rejection_and_teardown_release_storage() {
    let hub = OperationHub::new(8, 2).unwrap();
    let foreign = OperationHub::new(8, 2).unwrap();
    assert_eq!(
        hub.register_authenticated_archive(1, authenticated(&foreign)),
        Err(HubError::WrongRights)
    );
    assert_eq!(foreign.staging_budget.used(), 0);
    let full = OperationHub::new(8, 0).unwrap();
    assert_eq!(
        full.register_authenticated_archive(1, authenticated(&full)),
        Err(HubError::Quota)
    );
    assert_eq!(full.staging_budget.used(), 0);
    for terminal in [
        Terminal::Cancelled,
        Terminal::Trapped,
        Terminal::OwnerExited,
    ] {
        let hub = OperationHub::new(8, 2).unwrap();
        let token = hub
            .register_authenticated_archive(1, authenticated(&hub))
            .unwrap();
        assert_eq!(hub.staging_budget.used(), 16);
        hub.close_all(terminal);
        assert_eq!(hub.staging_budget.used(), 0);
        assert_eq!(hub.snapshot().live_resources, 0);
        assert!(hub.close_resource(token).is_err());
        assert_eq!(
            hub.register_authenticated_archive(1, authenticated(&hub)),
            Err(HubError::Closed)
        );
        assert_eq!(hub.staging_budget.used(), 0);
    }
}
