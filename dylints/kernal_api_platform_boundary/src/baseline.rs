//! The temporary exact-occurrence migration baseline (#152).
//!
//! After its `#` header, each line of `baseline.txt` names one existing violation as
//! `path<TAB>kind<TAB>construct<TAB>ordinal`: the repository-relative file,
//! the violation kind, the normalized construct, and the construct's
//! zero-based position among identical `(kind, construct)` hits in that file.
//! Line numbers are deliberately absent, so unrelated edits do not churn it.
//!
//! The baseline only shrinks. A hit without an entry is a new violation
//! (including a second copy in an already-listed file, which takes the next
//! ordinal, and a moved file, whose path changes); an entry without a hit is
//! stale and must be deleted; a repeated entry is rejected. The migration is
//! complete when the file is empty, and then it and this module are deleted.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::scan::Violation;

pub const BASELINE: &str = include_str!("baseline.txt");

/// This lint's manifest, relative to the repository root. It locates the
/// root at run time without trusting the checkout's spelling.
const LINT_MANIFEST: &str = "dylints/kernal_api_platform_boundary/Cargo.toml";

pub type Key = (String, String, String, usize);

/// The repository that owns `file`: the nearest ancestor holding this lint.
pub fn repository_root_of(file: &Path) -> Option<PathBuf> {
    let file = file.canonicalize().ok()?;
    file.ancestors()
        .skip(1)
        .find(|dir| dir.join(LINT_MANIFEST).is_file())
        .map(Path::to_path_buf)
}

/// Pair each violation with its baseline key. `violations` may come from any
/// number of files; ordinals are assigned per file in source order.
pub fn keyed<'a>(violations: &'a [Violation], root: &Path) -> Vec<(&'a Violation, Key)> {
    let mut sorted: Vec<&Violation> = violations.iter().collect();
    sorted.sort_by(|a, b| (&a.file, a.start, a.end).cmp(&(&b.file, b.start, b.end)));
    let mut seen: BTreeMap<(String, String, String), usize> = BTreeMap::new();
    sorted
        .into_iter()
        .map(|violation| {
            let file = violation
                .file
                .canonicalize()
                .unwrap_or_else(|_| violation.file.clone());
            let path = file
                .strip_prefix(root)
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            let id = (
                path,
                violation.kind.slug().to_owned(),
                violation.construct.clone(),
            );
            let ordinal = seen.entry(id.clone()).or_default();
            let key = (id.0, id.1, id.2, *ordinal);
            *ordinal += 1;
            (violation, key)
        })
        .collect()
}

/// Parse baseline text, rejecting malformed and repeated entries.
pub fn parse(text: &str) -> Result<BTreeSet<Key>, String> {
    let mut entries = BTreeSet::new();
    let mut total = None;
    for (number, line) in text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.is_empty())
    {
        if let Some(comment) = line.strip_prefix('#') {
            if let Some(value) = comment.trim().strip_prefix("total = ") {
                total = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| format!("baseline line {}: bad total", number + 1))?,
                );
            }
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let [path, kind, construct, ordinal] = fields[..] else {
            return Err(format!(
                "baseline line {}: expected four tab-separated fields",
                number + 1
            ));
        };
        let ordinal = ordinal
            .parse()
            .map_err(|_| format!("baseline line {}: bad ordinal `{ordinal}`", number + 1))?;
        let key = (
            path.to_owned(),
            kind.to_owned(),
            construct.to_owned(),
            ordinal,
        );
        if !entries.insert(key) {
            return Err(format!(
                "baseline line {}: duplicate entry `{line}`",
                number + 1
            ));
        }
    }
    match total {
        Some(total) if total == entries.len() => Ok(entries),
        Some(total) => Err(format!(
            "baseline `# total = {total}` but it lists {} entries",
            entries.len()
        )),
        None => Err("baseline has no `# total = N` header".to_owned()),
    }
}

#[cfg(test)]
pub fn format_key(key: &Key) -> String {
    format!("{}\t{}\t{}\t{}", key.0, key.1, key.2, key.3)
}

/// Compare the repository's actual violations against the baseline: both new
/// and stale entries are errors, so the file tracks the debt exactly.
#[cfg(test)]
pub fn check(actual: &[Key], text: &str) -> Result<(), String> {
    let baseline = parse(text)?;
    let actual_set: BTreeSet<Key> = actual.iter().cloned().collect();
    let new: Vec<String> = actual_set.difference(&baseline).map(format_key).collect();
    let stale: Vec<String> = baseline.difference(&actual_set).map(format_key).collect();
    let mut errors = Vec::new();
    if !new.is_empty() {
        errors.push(format!(
            "{} new boundary violations (move the code behind the platform facade; never add them to the baseline):\n{}",
            new.len(),
            new.join("\n")
        ));
    }
    if !stale.is_empty() {
        errors.push(format!(
            "{} stale baseline entries (the violation is gone; delete these lines):\n{}",
            stale.len(),
            stale.join("\n")
        ));
    }
    let sorted: Vec<String> = baseline.iter().map(format_key).collect();
    let listed: Vec<&str> = text
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    if errors.is_empty() && sorted.iter().map(String::as_str).ne(listed) {
        errors.push("baseline.txt is not sorted".to_owned());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `super::check` with the `# total` header the body implies.
    fn check(actual: &[Key], body: &str) -> Result<(), String> {
        let total = body.lines().filter(|line| !line.is_empty()).count();
        super::check(actual, &format!("# total = {total}\n{body}"))
    }

    #[test]
    fn the_total_must_match() {
        let text = "# total = 2\nsrc/a.rs\thost_cfg\tcfg(unix)\t0\n";
        let error = super::check(&[key("src/a.rs", "cfg(unix)", 0)], text).unwrap_err();
        assert!(error.contains("total = 2"), "{error}");
        assert!(super::check(&[], "").unwrap_err().contains("no `# total"));
    }

    fn key(path: &str, construct: &str, ordinal: usize) -> Key {
        (
            path.to_owned(),
            "host_cfg".to_owned(),
            construct.to_owned(),
            ordinal,
        )
    }

    #[test]
    fn exact_match_passes() {
        let actual = [
            key("src/a.rs", "cfg(unix)", 0),
            key("src/a.rs", "cfg(unix)", 1),
        ];
        let text = "src/a.rs\thost_cfg\tcfg(unix)\t0\nsrc/a.rs\thost_cfg\tcfg(unix)\t1\n";
        assert_eq!(check(&actual, text), Ok(()));
    }

    #[test]
    fn a_second_copy_in_a_listed_file_is_new() {
        let actual = [
            key("src/a.rs", "cfg(unix)", 0),
            key("src/a.rs", "cfg(unix)", 1),
        ];
        let error = check(&actual, "src/a.rs\thost_cfg\tcfg(unix)\t0\n").unwrap_err();
        assert!(error.contains("1 new"), "{error}");
    }

    #[test]
    fn a_moved_file_is_new_and_its_old_entry_stale() {
        let actual = [key("src/b.rs", "cfg(unix)", 0)];
        let error = check(&actual, "src/a.rs\thost_cfg\tcfg(unix)\t0\n").unwrap_err();
        assert!(
            error.contains("1 new") && error.contains("1 stale"),
            "{error}"
        );
    }

    #[test]
    fn fixed_violations_must_leave_the_baseline() {
        let error = check(&[], "src/a.rs\thost_cfg\tcfg(unix)\t0\n").unwrap_err();
        assert!(error.contains("1 stale"), "{error}");
    }

    #[test]
    fn duplicates_and_disorder_are_rejected() {
        let line = "src/a.rs\thost_cfg\tcfg(unix)\t0\n";
        let error =
            check(&[key("src/a.rs", "cfg(unix)", 0)], &format!("{line}{line}")).unwrap_err();
        assert!(error.contains("duplicate"), "{error}");
        let actual = [
            key("src/a.rs", "cfg(unix)", 0),
            key("src/b.rs", "cfg(unix)", 0),
        ];
        let error = check(
            &actual,
            "src/b.rs\thost_cfg\tcfg(unix)\t0\nsrc/a.rs\thost_cfg\tcfg(unix)\t0\n",
        )
        .unwrap_err();
        assert!(error.contains("not sorted"), "{error}");
    }

    #[test]
    fn ordinals_follow_source_order_per_file() {
        let violation = |file: &str, start| Violation {
            file: PathBuf::from(file),
            start,
            end: start + 1,
            line: 1,
            column: 1,
            kind: crate::scan::Kind::HostCfg,
            construct: "cfg(unix)".to_owned(),
        };
        let violations = [
            violation("/r/src/a.rs", 9),
            violation("/r/src/b.rs", 1),
            violation("/r/src/a.rs", 2),
        ];
        let keys: Vec<Key> = keyed(&violations, Path::new("/r"))
            .into_iter()
            .map(|(_, key)| key)
            .collect();
        assert_eq!(
            keys,
            [
                key("src/a.rs", "cfg(unix)", 0),
                key("src/a.rs", "cfg(unix)", 1),
                key("src/b.rs", "cfg(unix)", 0)
            ]
        );
    }
}
