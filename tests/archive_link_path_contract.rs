use kernal_api::platform::fs::resolve_archive_link_target;
use std::path::Path;

#[test]
fn relative_targets_resolve_but_parent_escapes_and_unsafe_link_names_do_not() {
    let root = Path::new("sdk");
    let link = root.join("usr/lib");
    assert_eq!(
        resolve_archive_link_target(root, &link, Path::new("../lib64")),
        Some(root.join("lib64"))
    );
    assert_eq!(
        resolve_archive_link_target(root, &link, Path::new("../../outside")),
        None
    );
    assert_eq!(
        resolve_archive_link_target(root, &root.join("../outside"), Path::new("inside")),
        None
    );
    assert_eq!(
        resolve_archive_link_target(root, &link, Path::new("/absolute")),
        None
    );
}
