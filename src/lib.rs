#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

/// Marker proving that version 0.0.0 is a name reservation, not a usable API.
pub const RESERVATION_ONLY: bool = true;

#[cfg(test)]
mod tests {
    use super::RESERVATION_ONLY;

    #[test]
    fn zero_release_is_explicitly_a_reservation() {
        assert!(RESERVATION_ONLY);
    }
}
