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
pub fn classify(files: &[RepoFile], arch: Option<&str>, supported: &[String]) -> Compat {
    let options = gguf_options(files);
    if !options.is_empty() {
        return Compat::Gguf { options };
    }

    let has = |pred: &dyn Fn(&str) -> bool| files.iter().any(|f| pred(&base_name(&f.path)));
    if has(&|n| n.ends_with(".safetensors")) {
        let has_config = has(&|n| n == "config.json");
        let has_tokenizer = has(&|n| n == "tokenizer.json" || n == "tokenizer.model");
        if !has_config || !has_tokenizer {
            return unsupported(
                "This repository has safetensors weights but is missing config.json or a tokenizer file, so Ollama cannot import it.",
            );
        }
        return match arch {
            Some(a) if supported.iter().any(|s| s == a) => Compat::Safetensors {
                arch: a.to_string(),
            },
            Some(a) => unsupported(&format!(
                "The model architecture {a} is not one Ollama can import from safetensors."
            )),
            None => unsupported(
                "The model architecture could not be determined, so Ollama cannot import these safetensors.",
            ),
        };
    }

    if has(&|n| n.ends_with(".bin") || n.ends_with(".pt") || n.ends_with(".pth")) {
        return unsupported(
            "This repository only has PyTorch weights, which Ollama cannot import. Look for a GGUF or safetensors version.",
        );
    }

    unsupported("This repository has no model weight files.")
}

fn unsupported(reason: &str) -> Compat {
    Compat::Unsupported {
        reason: reason.to_string(),
    }
}

fn base_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// Split `name-00002-of-00005.gguf` into (`name`, 2, 5); None if not a shard.
fn parse_shard(file_name: &str) -> Option<(String, u32, u32)> {
    let stem = file_name.get(..file_name.len().checked_sub(5)?)?;
    let (rest, total) = stem.rsplit_once("-of-")?;
    let (prefix, index) = rest.rsplit_once('-')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(index) || !digits(total) {
        return None;
    }
    Some((prefix.to_string(), index.parse().ok()?, total.parse().ok()?))
}

fn gguf_options(files: &[RepoFile]) -> Vec<GgufOption> {
    // Groups keyed by full path of the set (directory plus shard-less name).
    let mut groups: Vec<(String, Vec<(u32, RepoFile)>)> = Vec::new();
    for file in files {
        let name = base_name(&file.path);
        if !name.to_ascii_lowercase().ends_with(".gguf") {
            continue;
        }
        let (key, index) = match parse_shard(&name) {
            Some((prefix, index, total)) => (format!("{prefix}\u{0}{total}"), index),
            None => (file.path.clone(), 0),
        };
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push((index, file.clone())),
            None => groups.push((key, vec![(index, file.clone())])),
        }
    }
    groups
        .into_iter()
        .map(|(_, mut members)| {
            members.sort_by_key(|(i, _)| *i);
            let files: Vec<RepoFile> = members.into_iter().map(|(_, f)| f).collect();
            let total_bytes = files.iter().map(|f| f.size).sum();
            let first = base_name(&files[0].path);
            let name = match parse_shard(&first) {
                Some((prefix, _, _)) => prefix,
                None => first.get(..first.len() - 5).unwrap_or(&first).to_string(),
            };
            let quant = parse_quant(&name);
            GgufOption {
                name,
                quant,
                files,
                total_bytes,
            }
        })
        .collect()
}

/// Find a quantization tag such as `Q4_K_M`, `IQ3_XS`, `F16` or `BF16` in a name.
fn parse_quant(name: &str) -> Option<String> {
    let upper = name.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    for i in 0..bytes.len() {
        if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
            continue;
        }
        let Some(rest) = upper.get(i..) else {
            continue;
        };
        let starts_quant = |p: &str| {
            rest.strip_prefix(p)
                .is_some_and(|r| r.bytes().next().is_some_and(|b| b.is_ascii_digit()))
        };
        if starts_quant("IQ") || starts_quant("Q") || starts_quant("F") || starts_quant("BF") {
            let is_float = rest.starts_with('F') || rest.starts_with("BF");
            if is_float
                && !(rest.starts_with("F16") || rest.starts_with("F32") || rest.starts_with("BF16"))
            {
                continue;
            }
            let len = rest
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .count();
            let tag = name.get(i..i + len)?.trim_end_matches('_');
            return Some(tag.to_string());
        }
    }
    None
}

/// Architectures bundled with the app that Ollama can import from safetensors.
pub fn bundled_supported() -> Vec<String> {
    serde_json::from_str(include_str!("supported_architectures.json")).unwrap_or_default()
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
    fn non_ascii_names_do_not_panic() {
        let c = classify(&[f("modèle-é.gguf", 1)], None, &supported());
        assert!(matches!(c, Compat::Gguf { .. }));
    }

    #[test]
    fn bundled_list_contains_mistral() {
        assert!(bundled_supported().contains(&"MistralForCausalLM".to_string()));
    }
}
