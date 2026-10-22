use crate::model::RepoFile;

/// One downloadable GGUF choice: a single file or a complete shard set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GgufOption {
    pub name: String,
    pub quant: Option<String>,
    pub files: Vec<RepoFile>,
    pub total_bytes: u64,
}

/// Whether and how a repository can be imported into Ollama.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compat {
    Gguf { options: Vec<GgufOption> },
    Safetensors { arch: String },
    Unsupported { reason: String },
}

/// Classify a repository's file list.
pub fn classify(_files: &[RepoFile], _arch: Option<&str>, _supported: &[String]) -> Compat {
    todo!()
}

/// Architectures bundled with the app that Ollama can import from safetensors.
pub fn bundled_supported() -> Vec<String> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str, size: u64) -> RepoFile {
        RepoFile {
            path: path.to_string(),
            size,
            sha256: None,
        }
    }

    fn supported() -> Vec<String> {
        vec!["MistralForCausalLM".to_string()]
    }

    #[test]
    fn single_gguf_file_is_importable() {
        let c = classify(&[f("model.gguf", 10)], None, &supported());
        match c {
            Compat::Gguf { options } => {
                assert_eq!(options.len(), 1);
                assert_eq!(options[0].total_bytes, 10);
                assert_eq!(options[0].files.len(), 1);
            }
            other => panic!("expected Gguf, got {other:?}"),
        }
    }

    #[test]
    fn quant_parsed_from_name() {
        let c = classify(&[f("model-Q4_K_M.gguf", 10)], None, &supported());
        match c {
            Compat::Gguf { options } => {
                assert_eq!(options[0].quant, Some("Q4_K_M".to_string()));
            }
            other => panic!("expected Gguf, got {other:?}"),
        }
    }

    #[test]
    fn shards_group_into_one_option() {
        let files = [
            f("m-00003-of-00003.gguf", 30),
            f("m-00001-of-00003.gguf", 10),
            f("m-00002-of-00003.gguf", 20),
        ];
        let c = classify(&files, None, &supported());
        match c {
            Compat::Gguf { options } => {
                assert_eq!(options.len(), 1);
                assert_eq!(options[0].total_bytes, 60);
                let paths: Vec<&str> = options[0].files.iter().map(|x| x.path.as_str()).collect();
                assert_eq!(
                    paths,
                    [
                        "m-00001-of-00003.gguf",
                        "m-00002-of-00003.gguf",
                        "m-00003-of-00003.gguf"
                    ]
                );
            }
            other => panic!("expected Gguf, got {other:?}"),
        }
    }

    #[test]
    fn safetensors_with_supported_arch() {
        let files = [
            f("config.json", 1),
            f("tokenizer.json", 1),
            f("model.safetensors", 100),
        ];
        let c = classify(&files, Some("MistralForCausalLM"), &supported());
        assert_eq!(
            c,
            Compat::Safetensors {
                arch: "MistralForCausalLM".to_string()
            }
        );
    }

    #[test]
    fn safetensors_with_unknown_arch_is_unsupported() {
        let files = [
            f("config.json", 1),
            f("tokenizer.json", 1),
            f("model.safetensors", 100),
        ];
        let c = classify(&files, Some("FooForCausalLM"), &supported());
        assert!(matches!(c, Compat::Unsupported { .. }));
    }

    #[test]
    fn safetensors_missing_config_is_unsupported() {
        let files = [f("tokenizer.json", 1), f("model.safetensors", 100)];
        let c = classify(&files, Some("MistralForCausalLM"), &supported());
        assert!(matches!(c, Compat::Unsupported { .. }));
    }

    #[test]
    fn pytorch_bin_only_is_unsupported() {
        let files = [f("config.json", 1), f("pytorch_model.bin", 100)];
        match classify(&files, Some("MistralForCausalLM"), &supported()) {
            Compat::Unsupported { reason } => assert!(reason.contains("PyTorch")),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn empty_file_list_is_unsupported_not_panic() {
        match classify(&[], None, &supported()) {
            Compat::Unsupported { reason } => assert!(reason.contains("no model weight files")),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn readme_only_is_unsupported() {
        match classify(&[f("README.md", 5)], None, &supported()) {
            Compat::Unsupported { reason } => assert!(reason.contains("no model weight files")),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn gguf_wins_over_safetensors() {
        let files = [
            f("config.json", 1),
            f("tokenizer.json", 1),
            f("model.safetensors", 100),
            f("model-Q8_0.gguf", 50),
        ];
        let c = classify(&files, Some("MistralForCausalLM"), &supported());
        assert!(matches!(c, Compat::Gguf { .. }));
    }

    #[test]
    fn bundled_list_contains_mistral() {
        assert!(bundled_supported().contains(&"MistralForCausalLM".to_string()));
    }
}
