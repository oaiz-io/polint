//! Platform-independent check that a path stays inside the repository.
//!
//! Sidecars report repository-relative paths by contract, and the contract is
//! the same on every platform. [`std::path::Path::is_absolute`] is not: `/x` is
//! absolute on Unix and merely drive-relative on Windows, and `C:\x` is the
//! reverse. Asking the host would make a path that escapes on one platform
//! acceptable on another, so the shapes are checked directly.

/// True when `path` names a location from some root rather than from the
/// repository, or walks out of it.
pub fn escapes_repository(path: &str) -> bool {
    is_rooted(path) || walks_out(path)
}

fn is_rooted(path: &str) -> bool {
    if path.starts_with('/') || path.starts_with('\\') {
        return true;
    }
    // `C:`, `C:\x` and `C:x` all resolve against a drive rather than against
    // the repository. The rule is deliberately broad: it also refuses a Unix
    // file legitimately named `a:b`, which costs one dropped row with a
    // counter, where letting a drive-qualified path through costs a fact that
    // describes a file the scan does not own.
    let mut characters = path.chars();
    if let (Some(first), Some(':')) = (characters.next(), characters.next())
        && first.is_ascii_alphabetic()
    {
        return true;
    }
    std::path::Path::new(path).is_absolute()
}

fn walks_out(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized == ".."
        || normalized.starts_with("../")
        || normalized.contains("/../")
        || normalized.ends_with("/..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_relative_paths_are_accepted() {
        for path in [
            "src/app.ts",
            "packages/web/src/index.tsx",
            "a..b/c",
            "dir/..hidden",
            "",
        ] {
            assert!(!escapes_repository(path), "{path} should be accepted");
        }
    }

    #[test]
    fn a_leading_separator_is_rejected_on_every_platform() {
        // `/etc/passwd` is absolute on Unix and drive-relative on Windows.
        // Neither is repository-relative.
        assert!(escapes_repository("/etc/passwd"));
        assert!(escapes_repository("\\\\server\\share\\x"));
        assert!(escapes_repository("\\windows\\system32"));
    }

    #[test]
    fn a_drive_qualified_path_is_rejected_on_every_platform() {
        assert!(escapes_repository("C:/Windows"));
        assert!(escapes_repository("c:\\Windows"));
        assert!(escapes_repository("D:relative"));
    }

    #[test]
    fn a_single_letter_before_a_colon_is_refused_even_where_it_is_a_legal_name() {
        // `a:b/c.ts` is a legal Unix path and an unusual one. Refusing it costs
        // a dropped row; accepting the drive-qualified shape it is
        // indistinguishable from costs a fact about a file outside the scan.
        assert!(escapes_repository("a:b/c.ts"));
        // More than one character before the colon is not a drive.
        assert!(!escapes_repository("ab:c/d.ts"));
    }

    #[test]
    fn a_parent_walk_is_rejected_in_every_position_and_separator() {
        assert!(escapes_repository(".."));
        assert!(escapes_repository("../outside"));
        assert!(escapes_repository("src/../../etc/passwd"));
        assert!(escapes_repository("src\\..\\..\\etc"));
        assert!(escapes_repository("src/.."));
    }
}
