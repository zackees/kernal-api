use kernal_api::platform::fs::is_lock_contention;
use std::io::{Error, ErrorKind};

#[test]
fn normalized_contention_is_distinct_from_other_failures() {
    assert!(is_lock_contention(&Error::from(ErrorKind::WouldBlock)));
    for kind in [
        ErrorKind::NotFound,
        ErrorKind::PermissionDenied,
        ErrorKind::Interrupted,
    ] {
        assert!(!is_lock_contention(&Error::from(kind)));
    }
}

#[test]
fn windows_codes_are_interpreted_only_on_windows() {
    for code in [32, 33] {
        assert_eq!(
            is_lock_contention(&Error::from_raw_os_error(code)),
            cfg!(windows)
        );
    }
}
