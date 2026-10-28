//! Which tools exist and which may run. Four layers: family, capability,
//! permission and tool. The first release ships read-only Files tools only.

use super::tools::{self, Limits};
use super::workspace::{ToolError, Workspace};
use crate::session::{EffectiveToolConfig, Permission};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Files,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Read,
}

#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    pub family: Family,
    pub capability: Capability,
    pub description: &'static str,
    /// JSON Schema of the arguments, as Ollama expects it.
    pub parameters: Value,
}

impl Family {
    pub fn label(self) -> &'static str {
        match self {
            Family::Files => "Files",
        }
    }
}

impl Capability {
    pub fn label(self) -> &'static str {
        match self {
            Capability::Read => "Read",
        }
    }
}

/// The key a capability has in `EffectiveToolConfig::capabilities`.
pub fn capability_key(family: Family, capability: Capability) -> String {
    let f = match family {
        Family::Files => "files",
    };
    let c = match capability {
        Capability::Read => "read",
    };
    format!("{f}.{c}")
}

/// The most a capability may be set to. Read-only local capabilities may be
/// `Auto`; anything that writes, runs or reaches the network never is.
pub fn max_permission(_family: Family, capability: Capability) -> Permission {
    match capability {
        Capability::Read => Permission::Auto,
    }
}

/// The built-in tools.
pub fn specs() -> Vec<ToolSpec> {
    let read = |name, description, parameters| ToolSpec {
        name,
        family: Family::Files,
        capability: Capability::Read,
        description,
        parameters,
    };
    vec![
        read(
            "list_dir",
            "List the entries of a directory in the workspace (at most 200, sorted by name). Directories end with /.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Directory relative to the workspace root. Defaults to the root."}
                }
            }),
        ),
        read(
            "read_file",
            "Read a text file from the workspace, with line numbers. Returns at most 200 lines by default (1000 max) and says which offset to use to continue.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File relative to the workspace root."},
                    "offset": {"type": "integer", "description": "First line to return, starting at 1."},
                    "limit": {"type": "integer", "description": "Number of lines to return."}
                },
                "required": ["path"]
            }),
        ),
        read(
            "find_files",
            "Find files and directories whose path matches a glob such as *.rs or src/*.toml. A pattern without a slash matches at any depth.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "Glob pattern."},
                    "path": {"type": "string", "description": "Directory to search in. Defaults to the workspace root."}
                },
                "required": ["pattern"]
            }),
        ),
        read(
            "search_text",
            "Search text files for a regular expression. Returns path:line: text for each match (at most 200).",
            json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "Regular expression."},
                    "path": {"type": "string", "description": "File or directory to search. Defaults to the workspace root."},
                    "case_insensitive": {"type": "boolean", "description": "Ignore letter case."}
                },
                "required": ["pattern"]
            }),
        ),
    ]
}

/// The permission a spec currently has in a session's configuration. A
/// capability that was never configured runs at its maximum.
pub fn permission_of(config: &EffectiveToolConfig, spec: &ToolSpec) -> Permission {
    config
        .capabilities
        .get(&capability_key(spec.family, spec.capability))
        .copied()
        .unwrap_or_else(|| max_permission(spec.family, spec.capability))
}

/// Whether the tool is switched on for the session.
pub fn tool_enabled(config: &EffectiveToolConfig, spec: &ToolSpec) -> bool {
    config.tools.get(spec.name).copied().unwrap_or(true)
}

/// Checked at call time: the tool must be on and its capability allowed.
pub fn check(config: &EffectiveToolConfig, spec: &ToolSpec) -> Result<(), ToolError> {
    if !tool_enabled(config, spec) {
        return Err(ToolError::Off(spec.name.to_string()));
    }
    match permission_of(config, spec) {
        Permission::Off => Err(ToolError::Off(spec.name.to_string())),
        Permission::Ask => Err(ToolError::NeedsApproval(spec.name.to_string())),
        Permission::Auto => Ok(()),
    }
}

/// The tools the model is told about: those that would be allowed to run.
pub fn offered(config: &EffectiveToolConfig) -> Vec<ToolSpec> {
    specs()
        .into_iter()
        .filter(|s| check(config, s).is_ok())
        .collect()
}

/// `tools` for an Ollama chat request.
pub fn ollama_definitions(config: &EffectiveToolConfig) -> Value {
    Value::Array(
        offered(config)
            .into_iter()
            .map(|s| {
                json!({
                    "type": "function",
                    "function": {
                        "name": s.name,
                        "description": s.description,
                        "parameters": s.parameters,
                    }
                })
            })
            .collect(),
    )
}

/// Runs one tool call, after checking that it is allowed.
pub fn call(
    ws: &Workspace,
    config: &EffectiveToolConfig,
    name: &str,
    args: &Value,
    limits: &Limits,
) -> Result<String, ToolError> {
    let spec = specs()
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| ToolError::UnknownTool(name.to_string()))?;
    check(config, &spec)?;
    // A model that sends no arguments means "defaults".
    let empty = json!({});
    let args = if args.is_null() { &empty } else { args };
    if !args.is_object() {
        return Err(ToolError::InvalidArguments(
            "arguments must be an object".into(),
        ));
    }
    match name {
        "list_dir" => tools::list_dir(ws, args, limits),
        "read_file" => tools::read_file(ws, args, limits),
        "find_files" => tools::find_files(ws, args, limits),
        "search_text" => tools::search_text(ws, args, limits),
        other => Err(ToolError::UnknownTool(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn cfg() -> EffectiveToolConfig {
        EffectiveToolConfig::default()
    }

    fn names(v: &[ToolSpec]) -> Vec<&'static str> {
        v.iter().map(|s| s.name).collect()
    }

    #[test]
    fn the_first_release_has_exactly_the_four_read_tools() {
        let all = specs();
        assert_eq!(
            names(&all),
            ["list_dir", "read_file", "find_files", "search_text"]
        );
        assert!(all
            .iter()
            .all(|s| s.family == Family::Files && s.capability == Capability::Read));
    }

    #[test]
    fn an_unconfigured_session_allows_read_tools_automatically() {
        assert_eq!(offered(&cfg()).len(), 4);
        assert_eq!(
            capability_key(Family::Files, Capability::Read),
            "files.read"
        );
        assert_eq!(
            max_permission(Family::Files, Capability::Read),
            Permission::Auto
        );
    }

    #[test]
    fn a_single_tool_can_be_switched_off_without_losing_the_others() {
        let mut c = cfg();
        c.tools.insert("search_text".into(), false);
        assert_eq!(names(&offered(&c)), ["list_dir", "read_file", "find_files"]);
        let spec = specs()
            .into_iter()
            .find(|s| s.name == "search_text")
            .unwrap();
        assert_eq!(check(&c, &spec), Err(ToolError::Off("search_text".into())));
    }

    #[test]
    fn a_whole_capability_can_be_turned_off_or_set_to_ask() {
        let mut c = cfg();
        c.capabilities.insert("files.read".into(), Permission::Off);
        assert!(offered(&c).is_empty());
        c.capabilities.insert("files.read".into(), Permission::Ask);
        assert!(offered(&c).is_empty(), "no approval flow exists yet");
        let spec = &specs()[0];
        assert_eq!(
            check(&c, spec),
            Err(ToolError::NeedsApproval("list_dir".into()))
        );
        c.capabilities.insert("files.read".into(), Permission::Auto);
        assert_eq!(offered(&c).len(), 4);
    }

    #[test]
    fn ollama_definitions_describe_only_the_allowed_tools() {
        let mut c = cfg();
        c.tools.insert("find_files".into(), false);
        let defs = ollama_definitions(&c);
        let list = defs.as_array().unwrap();
        assert_eq!(list.len(), 3);
        for d in list {
            assert_eq!(d["type"], "function");
            assert!(d["function"]["name"].is_string());
            assert!(d["function"]["description"].as_str().unwrap().len() > 10);
            assert_eq!(d["function"]["parameters"]["type"], "object");
        }
        let read = list
            .iter()
            .find(|d| d["function"]["name"] == "read_file")
            .unwrap();
        assert_eq!(read["function"]["parameters"]["required"][0], "path");
    }

    #[test]
    fn call_runs_allowed_tools_and_refuses_the_rest_at_call_time() {
        let root = TempDir::new().unwrap();
        std::fs::write(root.path().join("a.txt"), "hello\n").unwrap();
        let ws = Workspace::open(root.path()).unwrap();
        let l = Limits::default();
        let out = call(&ws, &cfg(), "read_file", &json!({"path": "a.txt"}), &l).unwrap();
        assert_eq!(out, "1| hello");
        assert_eq!(
            call(&ws, &cfg(), "list_dir", &Value::Null, &l).unwrap(),
            "a.txt"
        );
        assert!(matches!(
            call(&ws, &cfg(), "write_file", &json!({}), &l),
            Err(ToolError::UnknownTool(_))
        ));
        assert!(matches!(
            call(&ws, &cfg(), "read_file", &json!("a.txt"), &l),
            Err(ToolError::InvalidArguments(_))
        ));
        // Turned off after the model was told about it: still refused.
        let mut c = cfg();
        c.tools.insert("read_file".into(), false);
        assert!(matches!(
            call(&ws, &c, "read_file", &json!({"path": "a.txt"}), &l),
            Err(ToolError::Off(_))
        ));
    }

    #[test]
    fn a_model_cannot_escape_the_workspace_through_any_tool() {
        let outside = TempDir::new().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "classified").unwrap();
        let root = TempDir::new().unwrap();
        let ws = Workspace::open(root.path()).unwrap();
        let l = Limits::default();
        let hostile = [
            (
                "read_file",
                json!({"path": outside.path().join("secret.txt").to_str().unwrap()}),
            ),
            ("read_file", json!({"path": "../secret.txt"})),
            (
                "list_dir",
                json!({"path": outside.path().to_str().unwrap()}),
            ),
            ("list_dir", json!({"path": "../"})),
            ("find_files", json!({"pattern": "*", "path": ".."})),
            ("search_text", json!({"pattern": "classified", "path": "/"})),
            ("read_file", json!({"path": "a\0b"})),
        ];
        for (tool, args) in hostile {
            let r = call(&ws, &cfg(), tool, &args, &l);
            assert!(r.is_err(), "{tool} {args} was allowed: {r:?}");
        }
    }
}
