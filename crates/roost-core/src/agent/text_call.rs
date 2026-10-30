//! Spotting a reply that is a tool call written out as text.
//!
//! Some models, or their chat templates, do not produce real tool calls: the
//! reply is the JSON of a call (`{"name": "list_dir", "arguments": {}}`) and
//! nothing runs. Roost does not run such text; it says what happened.

use serde_json::Value;

/// The names of the tools, in order and without repeats, when the whole reply
/// is nothing but calls to tools in `known` written as JSON (one object, an
/// array of objects, or several objects one after another, optionally inside
/// a code fence). Anything else, including a reply with prose around the
/// JSON, gives an empty list.
pub fn tool_calls_written_as_text(reply: &str, known: &[&str]) -> Vec<String> {
    let body = unfence(reply.trim());
    if !body.starts_with('{') && !body.starts_with('[') {
        return Vec::new();
    }
    let mut names: Vec<String> = Vec::new();
    for value in serde_json::Deserializer::from_str(body).into_iter::<Value>() {
        let Ok(value) = value else {
            return Vec::new();
        };
        let items = match value {
            Value::Array(items) if !items.is_empty() => items,
            other => vec![other],
        };
        for item in items {
            let Some(name) = call_name(&item).filter(|n| known.contains(n)) else {
                return Vec::new();
            };
            if !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// `item` is `{"name": ..., "arguments": {...}}` (or `parameters`).
fn call_name(item: &Value) -> Option<&str> {
    let object = item.as_object()?;
    let args = object
        .get("arguments")
        .or_else(|| object.get("parameters"))?;
    if !args.is_object() {
        return None;
    }
    object.get("name")?.as_str()
}

/// The text inside a surrounding ``` fence, if there is one.
fn unfence(text: &str) -> &str {
    let Some(inner) = text.strip_prefix("```") else {
        return text;
    };
    let inner = inner.strip_suffix("```").unwrap_or(inner);
    // Drop the language tag on the first line.
    match inner.split_once('\n') {
        Some((tag, rest)) if !tag.trim_start().starts_with(['{', '[']) => rest.trim(),
        _ => inner.trim(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN: &[&str] = &["list_dir", "search_text", "read_file"];

    fn names(text: &str) -> Vec<String> {
        tool_calls_written_as_text(text, KNOWN)
    }

    #[test]
    fn a_reply_that_is_one_call_is_recognised() {
        assert_eq!(
            names(r#"{"name": "list_dir", "arguments": {}}"#),
            ["list_dir"]
        );
        assert_eq!(
            names(r#"  {"name":"read_file","parameters":{"path":"a"}}  "#),
            ["read_file"]
        );
    }

    #[test]
    fn several_calls_in_a_row_in_an_array_or_in_a_fence_are_recognised() {
        let two = r#"{"name": "list_dir", "arguments": {}} {"name": "read_file", "arguments": {"path": "a"}}"#;
        assert_eq!(names(two), ["list_dir", "read_file"]);
        let array = r#"[{"name":"list_dir","arguments":{}},{"name":"list_dir","arguments":{"path":"src"}}]"#;
        assert_eq!(names(array), ["list_dir"], "no repeats");
        let fenced =
            "```json\n{\"name\": \"search_text\", \"arguments\": {\"pattern\": \"x\"}}\n```";
        assert_eq!(names(fenced), ["search_text"]);
    }

    #[test]
    fn anything_else_is_not_taken_for_a_call() {
        // Prose around the JSON, an unknown tool, a normal JSON answer, a
        // call whose arguments are not an object, and plain text.
        for text in [
            r#"Sure, I will run {"name": "list_dir", "arguments": {}}"#,
            r#"{"name": "launch_missiles", "arguments": {}}"#,
            r#"{"name": "list_dir", "arguments": {}} and then I will stop"#,
            r#"{"files": ["a", "b"], "count": 2}"#,
            r#"{"name": "list_dir", "arguments": "src"}"#,
            r#"{"name": "list_dir"}"#,
            "The folder has two files.",
            "",
            "{not json",
            "[]",
        ] {
            assert!(names(text).is_empty(), "{text}");
        }
    }
}
