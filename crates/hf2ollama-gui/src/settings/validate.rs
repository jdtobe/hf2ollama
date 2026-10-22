//! Pure validation of settings fields. No egui in here.

#![allow(dead_code)] // stubs until the Settings state uses them

use std::path::Path;

/// A server address: `http://` or `https://`, a host, an optional port.
/// Returns the trimmed value without a trailing slash.
pub fn validate_ollama_url(raw: &str) -> Result<String, String> {
    let _ = raw;
    Err("not implemented".to_string())
}

/// Same rules as the Ollama address.
pub fn validate_hf_endpoint(raw: &str) -> Result<String, String> {
    let _ = raw;
    Err("not implemented".to_string())
}

/// An existing writable directory, or a missing path whose parent exists and
/// is a writable directory. A file path is never accepted.
pub fn validate_staging_dir(path: &Path) -> Result<(), String> {
    let _ = path;
    Ok(())
}

/// Simultaneous downloads stay within 1 to 3.
pub fn clamp_max_parallel(n: u8) -> u8 {
    n
}

/// A positive, finite number of gigabytes (a comma is accepted as decimal point).
pub fn parse_gb(raw: &str) -> Result<f32, String> {
    let _ = raw;
    Ok(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn ollama_url_accepts_http_and_https_with_host() {
        for (raw, want) in [
            ("http://localhost:11434", "http://localhost:11434"),
            ("https://box.lan:8443", "https://box.lan:8443"),
            ("http://127.0.0.1", "http://127.0.0.1"),
            ("http://[::1]:11434", "http://[::1]:11434"),
            ("HTTP://Example.com", "HTTP://Example.com"),
            ("http://h/prefix", "http://h/prefix"),
        ] {
            assert_eq!(validate_ollama_url(raw).as_deref(), Ok(want), "{raw}");
        }
    }

    #[test]
    fn urls_are_trimmed_and_lose_trailing_slashes() {
        assert_eq!(
            validate_ollama_url("  http://localhost:11434/  ").as_deref(),
            Ok("http://localhost:11434")
        );
        assert_eq!(
            validate_hf_endpoint("https://huggingface.co//").as_deref(),
            Ok("https://huggingface.co")
        );
    }

    #[test]
    fn urls_reject_empty_whitespace_wrong_scheme_and_missing_host() {
        for raw in [
            "",
            "   ",
            "ftp://host",
            "localhost:11434",
            "http://",
            "https:///path",
            "http://:11434",
            "http://ho st",
            "http://host:abc",
            "http://host:0",
            "http://host:70000",
            "http://user:pw@host",
            "http://host?x=1",
            "http://host#frag",
            "http://[::1",
            "javascript:alert(1)",
        ] {
            assert!(validate_ollama_url(raw).is_err(), "{raw:?} was accepted");
            assert!(validate_hf_endpoint(raw).is_err(), "{raw:?} was accepted");
        }
    }

    #[test]
    fn url_errors_are_readable() {
        let e = validate_ollama_url("ftp://x").unwrap_err();
        assert!(e.contains("http://") && e.contains("https://"), "{e}");
        let e = validate_ollama_url("").unwrap_err();
        assert!(e.to_lowercase().contains("enter"), "{e}");
    }

    #[test]
    fn staging_dir_must_be_writable_directory() {
        let dir = tempfile::tempdir().unwrap();
        // An existing directory is fine.
        assert_eq!(validate_staging_dir(dir.path()), Ok(()));
        // A missing folder under an existing parent is fine (it is created later).
        assert_eq!(validate_staging_dir(&dir.path().join("new")), Ok(()));
        // A missing parent is not.
        let e = validate_staging_dir(&dir.path().join("a").join("b")).unwrap_err();
        assert!(e.contains("does not exist"), "{e}");
        // A file is never a staging folder.
        let file = dir.path().join("file.txt");
        std::fs::write(&file, "x").unwrap();
        let e = validate_staging_dir(&file).unwrap_err();
        assert!(e.contains("not a folder"), "{e}");
        // Nor is a path below a file.
        assert!(validate_staging_dir(&file.join("sub")).is_err());
        // Empty and relative-less paths are refused.
        assert!(validate_staging_dir(&PathBuf::new()).is_err());
        // Probing leaves nothing behind.
        let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(left.len(), 1, "only file.txt: {left:?}");
    }

    #[cfg(unix)]
    #[test]
    fn staging_dir_read_only_directory_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let ro = dir.path().join("ro");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Root ignores permission bits: nothing to assert then.
        let root_can_write = std::fs::write(ro.join("probe"), "x").is_ok();
        if !root_can_write {
            let e = validate_staging_dir(&ro).unwrap_err();
            assert!(e.contains("not writable"), "{e}");
            // A new folder inside it cannot be created either.
            let e = validate_staging_dir(&ro.join("child")).unwrap_err();
            assert!(e.contains("not writable"), "{e}");
        }
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn max_parallel_cannot_leave_1_to_3() {
        assert_eq!(clamp_max_parallel(0), 1);
        assert_eq!(clamp_max_parallel(1), 1);
        assert_eq!(clamp_max_parallel(2), 2);
        assert_eq!(clamp_max_parallel(3), 3);
        assert_eq!(clamp_max_parallel(4), 3);
        assert_eq!(clamp_max_parallel(u8::MAX), 3);
    }

    #[test]
    fn parse_gb_accepts_positive_numbers() {
        assert_eq!(parse_gb("16"), Ok(16.0));
        assert_eq!(parse_gb(" 24.5 "), Ok(24.5));
        assert_eq!(parse_gb("0.5"), Ok(0.5));
        assert_eq!(parse_gb("8,5"), Ok(8.5));
    }

    #[test]
    fn parse_gb_rejects_empty_zero_negative_and_non_finite() {
        for raw in [
            "", "  ", "0", "0.0", "-1", "-0.5", "abc", "NaN", "nan", "inf", "-inf", "infinity",
            "1e40", "1e9", "100001", "1.2.3", "16GB",
        ] {
            assert!(parse_gb(raw).is_err(), "{raw:?} was accepted");
        }
        assert_eq!(parse_gb("100000"), Ok(100000.0));
    }
}
