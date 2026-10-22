//! Validation of Ollama model names: `[namespace/]model[:tag]`.

use crate::error::{Error, Result};

const MAX_TOTAL: usize = 200;
const MAX_PART: usize = 64;

/// Accepts `[namespace/]model[:tag]`. Each part is 1 to 64 characters of
/// lowercase ASCII letters, digits, `_`, `-` or `.`, and neither starts nor
/// ends with `.` or `-`.
pub fn validate_model_name(name: &str) -> Result<()> {
    let bad = || Error::InvalidName(name.to_string());
    if name.len() > MAX_TOTAL {
        return Err(bad());
    }
    let (path, tag) = match name.split_once(':') {
        Some((path, tag)) => (path, Some(tag)),
        None => (name, None),
    };
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() > 2 {
        return Err(bad());
    }
    if segments.into_iter().chain(tag).all(valid_part) {
        Ok(())
    } else {
        Err(bad())
    }
}

fn valid_part(part: &str) -> bool {
    let edge_ok = |c: Option<char>| c.is_some_and(|c| c != '.' && c != '-');
    part.len() <= MAX_PART
        && part
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.'))
        && edge_ok(part.chars().next())
        && edge_ok(part.chars().next_back())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invalid(name: &str) -> Error {
        Error::InvalidName(name.to_string())
    }

    #[test]
    fn validate_model_name_rejects_empty_uppercase_spaces() {
        for bad in ["", "Foo", "foo bar", " foo", "foo ", "foo:Bar", "NS/foo"] {
            assert_eq!(validate_model_name(bad), Err(invalid(bad)), "{bad:?}");
        }
    }

    #[test]
    fn validate_model_name_accepts_name_colon_tag() {
        for good in [
            "foo",
            "foo:latest",
            "my-model_1.5:q4-k-m",
            "ns/foo",
            "a/b:c",
            "qwen2.5-coder:1.5b",
        ] {
            assert_eq!(validate_model_name(good), Ok(()), "{good:?}");
        }
    }

    #[test]
    fn validate_model_name_rejects_malformed_structure() {
        for bad in [
            "-x", "x-", ".x", "x.", "x:", ":x", "a::b", "a:b:c", "/x", "x/", "a/b/c", "a//b", "ü",
            "a:-b", "a/-b:c",
        ] {
            assert_eq!(validate_model_name(bad), Err(invalid(bad)), "{bad:?}");
        }
    }

    #[test]
    fn validate_model_name_enforces_length_limits() {
        let p64 = "a".repeat(64);
        assert_eq!(validate_model_name(&p64), Ok(()));
        let p65 = "a".repeat(65);
        assert_eq!(validate_model_name(&p65), Err(invalid(&p65)));
        let long = format!("{p64}/{p64}:{p64}");
        assert!(long.len() <= 200);
        assert_eq!(validate_model_name(&long), Ok(()));
        let too_long = format!("{p64}/{p64}:{p64}{p64}");
        assert!(too_long.len() > 200);
        assert_eq!(validate_model_name(&too_long), Err(invalid(&too_long)));
    }
}
