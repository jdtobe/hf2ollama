//! What each agent tool is held to, for the Individual tools list: a short
//! summary beside the tool and the full set in a small popover. The values
//! are the app-wide ones from Settings, Tools; a model with a small context
//! window gets smaller results than these (see `Limits::for_window`).

use crate::settings::{size_text, time_text};
use roost_core::agent::ask;
use roost_core::agent::tools::{Limits, BINARY_MAX_BYTES};

fn bytes(n: usize) -> String {
    size_text(n as u64)
}

/// One short phrase for the row, or `None` for a tool without limits.
pub fn summary(name: &str, l: &Limits) -> Option<String> {
    Some(match name {
        "list_dir" => format!("{} entries", l.list_entries),
        "read_file" => format!("{} lines", l.read_max_lines),
        "read_binary" => format!("{} per call", size_text(BINARY_MAX_BYTES)),
        "find_files" => format!("{} paths", l.find_results),
        "search_text" => format!("{} matches", l.search_matches),
        "search_bytes" => format!("{} per file", size_text(l.bytes_file_bytes)),
        "write_file" => format!("{} per write", bytes(l.write_bytes)),
        "edit_file" => format!("{} per file", bytes(l.edit_file_bytes)),
        "run_command" => time_text(l.run_timeout.as_secs()),
        "ask_user" => format!("{} per run", ask::MAX_PER_RUN),
        _ => return None,
    })
}

/// The tool's limits as (what, value) pairs for the popover.
pub fn facts(name: &str, l: &Limits) -> Vec<(&'static str, String)> {
    let n = |v: usize| v.to_string();
    match name {
        "list_dir" => vec![
            ("Entries per call", n(l.list_entries)),
            ("Output per call", bytes(l.output_bytes)),
        ],
        "read_file" => vec![
            ("Lines when none are asked for", n(l.read_default_lines)),
            ("Most lines per call", n(l.read_max_lines)),
            ("Characters per line", n(l.line_chars)),
            ("Largest file opened", size_text(l.read_file_bytes)),
            ("Output per call", bytes(l.output_bytes)),
        ],
        "read_binary" => vec![
            ("Bytes per call", size_text(BINARY_MAX_BYTES)),
            ("Output per call", bytes(l.output_bytes)),
        ],
        "find_files" => vec![
            ("Paths per call", n(l.find_results)),
            ("Folders deep", n(l.depth)),
            ("Entries looked at", n(l.examined)),
            ("Output per call", bytes(l.output_bytes)),
        ],
        "search_text" => vec![
            ("Matches per call", n(l.search_matches)),
            ("Matches from one file", n(l.per_file_matches)),
            ("Folders deep", n(l.depth)),
            ("Entries looked at", n(l.examined)),
            ("Characters per line", n(l.line_chars)),
            ("Largest file searched", size_text(l.search_file_bytes)),
            ("Output per call", bytes(l.output_bytes)),
        ],
        "search_bytes" => vec![
            ("Largest file searched", size_text(l.bytes_file_bytes)),
            ("Output per call", bytes(l.output_bytes)),
        ],
        "write_file" => vec![
            ("Written per call", bytes(l.write_bytes)),
            ("Largest file changed", bytes(l.edit_file_bytes)),
            ("Undo copies kept", bytes(l.undo_bytes)),
        ],
        "edit_file" => vec![
            ("Largest file changed", bytes(l.edit_file_bytes)),
            ("Undo copies kept", bytes(l.undo_bytes)),
        ],
        "run_command" => vec![
            ("Stopped after", time_text(l.run_timeout.as_secs())),
            ("Output kept", bytes(l.run_output_bytes)),
        ],
        "ask_user" => vec![
            ("Questions per run", ask::MAX_PER_RUN.to_string()),
            (
                "Options per question",
                format!("{} to {}", ask::MIN_OPTIONS, ask::MAX_OPTIONS),
            ),
            (
                "Typed answer",
                format!("{} characters", ask::MAX_ANSWER_CHARS),
            ),
        ],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roost_core::agent::registry;

    #[test]
    fn every_registered_tool_has_a_summary_and_limits() {
        let l = Limits::default();
        for spec in registry::specs() {
            assert!(summary(spec.name, &l).is_some(), "{}", spec.name);
            assert!(!facts(spec.name, &l).is_empty(), "{}", spec.name);
        }
        assert!(summary("nope", &l).is_none());
        assert!(facts("nope", &l).is_empty());
    }

    #[test]
    fn the_values_follow_the_settings() {
        let l = Limits {
            list_entries: 123,
            search_matches: 45,
            run_timeout: std::time::Duration::from_secs(120),
            ..Limits::default()
        };
        assert_eq!(summary("list_dir", &l).as_deref(), Some("123 entries"));
        assert_eq!(summary("search_text", &l).as_deref(), Some("45 matches"));
        assert_eq!(summary("run_command", &l).as_deref(), Some("2 minutes"));
        assert!(facts("list_dir", &l)
            .iter()
            .any(|(k, v)| *k == "Entries per call" && v == "123"));
        assert!(facts("run_command", &l)
            .iter()
            .any(|(k, v)| *k == "Stopped after" && v == "2 minutes"));
    }
}
