//! Desktop status-area presence without exposing toolkit or D-Bus types.

use crate::native_system_tray;

/// A bounded square ARGB32 icon and application identity.
#[derive(Clone, Debug)]
pub struct TrayOptions {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) size: u16,
    pub(crate) argb: Vec<u8>,
}

impl TrayOptions {
    /// Validate all data before contacting the desktop. Icons are at most 128².
    pub fn new(id: &str, title: &str, size: u16, argb: Vec<u8>) -> Result<Self, TrayError> {
        if id.is_empty()
            || id.len() > 255
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || title.is_empty()
            || title.len() > 255
            || title.contains('\0')
            || !(1..=128).contains(&size)
            || argb.len() != usize::from(size).pow(2) * 4
        {
            return Err(TrayError::InvalidOptions);
        }
        Ok(Self {
            id: id.into(),
            title: title.into(),
            size,
            argb,
        })
    }
}

/// Registration is optional; callers retain their existing visible fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayError {
    InvalidOptions,
    Unsupported,
    Unavailable,
}

/// A deliberate user action. Native callbacks never open windows themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    Activate,
    OpenDashboard,
    Quit,
}

/// Owned tray registration. Dropping it unregisters the item.
pub struct TrayHandle {
    native: native_system_tray::Handle,
}
impl TrayHandle {
    /// Register on the existing facade async runtime without blocking the UI thread.
    /// Fails within two seconds; unsupported desktops retain a caller-owned fallback.
    pub async fn register(options: TrayOptions) -> Result<Self, TrayError> {
        native_system_tray::register(options)
            .await
            .map(|native| Self { native })
    }
    /// Whether a desktop host can currently display the item.
    pub fn is_online(&self) -> bool {
        self.native.is_online()
    }
    /// Take one buffered action; the callback queue is bounded to 16 actions.
    pub fn try_event(&self) -> Option<TrayEvent> {
        self.native.try_event()
    }
    /// Update the displayed status; icon dimensions and identity stay unchanged.
    pub async fn set_title(&self, title: &str) -> Result<(), TrayError> {
        if title.is_empty() || title.len() > 255 || title.contains('\0') {
            return Err(TrayError::InvalidOptions);
        }
        self.native.set_title(title).await
    }
}
