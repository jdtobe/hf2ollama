//! Validation of Ollama model names: `[namespace/]model[:tag]`.

use crate::error::{Error, Result};

const MAX_TOTAL: usize = 200;
const MAX_PART: usize = 64;

/// Accepts `[namespace/]model[:tag]`. Each part is 1 to 64 characters of
/// lowercase ASCII letters, digits, `_`, `-` or `.`, and neither starts nor
/// ends with `.` or `-`.
pub fn validate_model_name(name: &str) -> Result<()> {
    todo!("{name}")
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
