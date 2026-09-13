//! Compatibility lookup for callers supplying their own PATH environment.
use std::ffi::OsStr;
use std::path::PathBuf;

/// Host implicit executable suffixes, in search order.
/// Windows reads PATHEXT (defaulting when absent or non-Unicode), trims empty
/// entries and ASCII-lowercases suffixes. Other hosts have no implicit suffixes.
pub fn candidate_extensions() -> Vec<String> {
    #[cfg(windows)]
    {
        std::env::var_os("PATHEXT")
            .and_then(|value| value.into_string().ok())
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string())
            .split(';')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Resolve using only the supplied PATH and the host suffix list.
/// See [`find_on_path_using`] for explicit-path and file-probing semantics.
pub fn find_on_supplied_path(name: &str, path_value: &OsStr) -> Option<PathBuf> {
    find_on_path_using(name, path_value, &candidate_extensions())
}

/// Trust names containing either slash as explicit paths without probing them.
/// Otherwise skip empty PATH entries and, per directory, try the bare name then
/// every supplied suffix, even for already-dotted names. Matches must be files;
/// this does not check execute permission and is not an authorization check.
/// Never reads the ambient PATH or PATHEXT.
pub fn find_on_path_using(
    name: &str,
    path_value: &OsStr,
    extensions: &[String],
) -> Option<PathBuf> {
    if name.contains('/') || name.contains('\\') {
        return Some(PathBuf::from(name));
    }
    for dir in std::env::split_paths(path_value) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        for extension in extensions {
            let candidate = dir.join(format!("{name}{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
