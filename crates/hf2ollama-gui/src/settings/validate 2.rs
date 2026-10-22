//! Pure validation of settings fields. No egui in here.

use std::path::Path;

const URL_HINT: &str =
    "Enter an address starting with http:// or https://, like http://localhost:11434";

/// A server address: `http://` or `https://`, a host, an optional port.
/// Returns the trimmed value without a trailing slash.
pub fn validate_ollama_url(raw: &str) -> Result<String, String> {
    validate_url(raw)
}

/// Same rules as the Ollama address.
pub fn validate_hf_endpoint(raw: &str) -> Result<String, String> {
    validate_url(raw)
}

fn validate_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    if url.is_empty() {
        return Err(URL_HINT.to_string());
    }
    let lower = url.to_ascii_lowercase();
    let rest = if lower.starts_with("http://") {
        &url["http://".len()..]
    } else if lower.starts_with("https://") {
        &url["https://".len()..]
    } else {
        return Err(URL_HINT.to_string());
    };
    if rest.chars().any(char::is_whitespace) {
        return Err("The address cannot contain spaces.".to_string());
    }
    if rest.contains('?') || rest.contains('#') {
        return Err("Leave out the query and fragment: use only host, port and path.".to_string());
    }
    let authority = rest.split('/').next().unwrap_or("");
    if authority.contains('@') {
        return Err("Do not put a user name or password in the address.".to_string());
    }
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let (host, after) = inner
            .split_once(']')
            .ok_or_else(|| "The address has an unclosed [".to_string())?;
        (host, after.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        }
    };
    if host.is_empty() {
        return Err("The address needs a host name, like localhost.".to_string());
    }
    if let Some(p) = port {
        match p.parse::<u16>() {
            Ok(n) if n >= 1 => {}
            _ => return Err("The port must be a number from 1 to 65535.".to_string()),
        }
    }
    Ok(url.trim_end_matches('/').to_string())
}

/// The read-only part of the staging check (no writes): an absolute path
/// without `..`, naming a directory or a missing child of one. Cheap enough
/// to run on every keystroke.
pub fn check_staging_dir(path: &Path) -> Result<(), String> {
    use std::path::Component;
    let text = path.to_string_lossy();
    if text.trim().is_empty() {
        return Err("Enter a full path to a folder.".to_string());
    }
    if text.starts_with('~') {
        return Err("A leading ~ is not expanded: enter a full path.".to_string());
    }
    if !path.is_absolute() {
        return Err("Enter a full path, starting at the root of a drive or /.".to_string());
    }
    if path.components().any(|c| c == Component::ParentDir) {
        return Err("The path cannot contain .. : enter a full path.".to_string());
    }
    match std::fs::metadata(path) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err("That path is not a folder (it is a file).".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Absolute and not a root, so a parent exists in the path.
            let parent = path.parent().unwrap_or(path);
            match std::fs::metadata(parent) {
                Ok(m) if m.is_dir() => Ok(()),
                Ok(_) => Err("The parent of that path is not a folder.".to_string()),
                Err(_) => Err("The parent folder does not exist.".to_string()),
            }
        }
        Err(e) => Err(format!("Cannot read that path: {e}")),
    }
}

/// `check_staging_dir`, plus a write probe: the folder (or, for a missing
/// one, its parent) must accept a new file. Run it on commit only.
pub fn validate_staging_dir(path: &Path) -> Result<(), String> {
    check_staging_dir(path)?;
    if path.is_dir() {
        writable(path)
    } else {
        writable(path.parent().unwrap_or(path))
    }
}

/// Creates and removes a uniquely named file to find out whether `dir`
/// accepts writes; nothing is left behind either way.
fn writable(dir: &Path) -> Result<(), String> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let probe = dir.join(format!(
        ".hf2ollama-write-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(_) => Err("That folder is not writable.".to_string()),
    }
}

/// Simultaneous downloads stay within 1 to 3.
pub fn clamp_max_parallel(n: u8) -> u8 {
    n.clamp(1, 3)
}

/// Largest value accepted: 100 000 GB (100 TB) is beyond any real machine.
const MAX_GB: f32 = 100_000.0;

/// A positive, finite number of gigabytes (a comma is accepted as decimal point).
pub fn parse_gb(raw: &str) -> Result<f32, String> {
    let text = raw.trim().replace(',', ".");
    if text.is_empty() {
        return Err("Enter a number of GB.".to_string());
    }
    let n: f32 = text
        .parse()
        .map_err(|_| "Enter a number, like 16.".to_string())?;
    if !n.is_finite() || n <= 0.0 {
        return Err("Enter a number greater than 0.".to_string());
    }
    if n > MAX_GB {
        return Err("That is more memory than any machine has.".to_string());
    }
    Ok(n)
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
        // An empty path is refused.
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

    fn full_path_hint(e: &str) -> bool {
        e.contains("full path")
    }

    #[test]
    fn relative_staging_path_is_rejected() {
        for raw in ["foo", "./foo", "../x", "a/b"] {
            let p = PathBuf::from(raw);
            let e = validate_staging_dir(&p).unwrap_err();
            assert!(full_path_hint(&e), "{raw}: {e}");
            assert!(check_staging_dir(&p).is_err(), "{raw}");
        }
    }

    #[test]
    fn dot_dot_component_is_rejected_even_when_absolute() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("..").join("x");
        assert!(validate_staging_dir(&p).unwrap_err().contains(".."));
        assert!(check_staging_dir(&p).is_err());
        let p = dir.path().join("a").join("..");
        assert!(check_staging_dir(&p).is_err());
    }

    #[test]
    fn empty_and_tilde_staging_paths_are_rejected() {
        for raw in ["", "~", "~/models"] {
            let p = PathBuf::from(raw);
            let e = validate_staging_dir(&p).unwrap_err();
            assert!(check_staging_dir(&p).is_err(), "{raw:?}");
            if raw.starts_with('~') {
                assert!(e.contains('~') && full_path_hint(&e), "{raw}: {e}");
            }
        }
    }

    #[test]
    fn absolute_writable_directory_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(check_staging_dir(dir.path()), Ok(()));
        assert_eq!(validate_staging_dir(dir.path()), Ok(()));
    }

    #[test]
    fn absolute_nonexistent_with_writable_parent_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("new-staging");
        assert_eq!(check_staging_dir(&p), Ok(()));
        assert_eq!(validate_staging_dir(&p), Ok(()));
        assert!(!p.exists(), "validation creates nothing");
    }

    #[test]
    fn file_path_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f.txt");
        std::fs::write(&f, "x").unwrap();
        assert!(check_staging_dir(&f).unwrap_err().contains("not a folder"));
        assert!(validate_staging_dir(&f).is_err());
    }
}
