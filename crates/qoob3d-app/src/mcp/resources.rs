//! What the server carries in its binary (AG-MC-05, AG-MC-06): the script
//! API reference, the workflow guide and the example scripts, so they match
//! the version that runs.

use serde_json::{Value, json};

use super::{INVALID_PARAMS, RpcError};

pub(super) const GUIDE: &str = include_str!("guide.md");
const SCRIPT_API: &str = include_str!("../../../../docs/script-api.md");
/// `(name, script)` of the examples, run by `qoob3d-kernel/tests/script_examples.rs`.
const EXAMPLES: [(&str, &str); 4] = [
    ("crane_column.rhai", include_str!("../../../../docs/script-api/examples/crane_column.rhai")),
    (
        "crane_column_revolve.rhai",
        include_str!("../../../../docs/script-api/examples/crane_column_revolve.rhai"),
    ),
    ("railing.rhai", include_str!("../../../../docs/script-api/examples/railing.rhai")),
    ("stair.rhai", include_str!("../../../../docs/script-api/examples/stair.rhai")),
];
const REFERENCE_URI: &str = "qoob3d://script-api";
const GUIDE_URI: &str = "qoob3d://guide";
const EXAMPLE_URI: &str = "qoob3d://examples/";
/// JSON-RPC code of MCP for a resource that does not exist.
const RESOURCE_NOT_FOUND: i64 = -32002;

/// The reference split at its headings; code blocks stay with their section.
fn sections() -> Vec<String> {
    let mut sections = vec![String::new()];
    let mut code = false;
    for line in SCRIPT_API.lines() {
        if line.starts_with("```") {
            code = !code;
        } else if !code && line.starts_with('#') {
            sections.push(String::new());
        }
        if let Some(section) = sections.last_mut() {
            section.push_str(line);
            section.push('\n');
        }
    }
    sections
}

/// The whole reference, or the sections whose heading or text contains
/// `search` (case-insensitive); `None` if nothing matches.
pub(super) fn api_reference(search: Option<&str>) -> Option<String> {
    let Some(needle) = search.map(str::to_lowercase).filter(|s| !s.trim().is_empty()) else {
        return Some(SCRIPT_API.to_owned());
    };
    let found: Vec<String> =
        sections().into_iter().filter(|s| s.to_lowercase().contains(&needle)).collect();
    (!found.is_empty()).then(|| found.concat())
}

/// `resources/list`.
pub(super) fn list() -> Value {
    let mut resources = vec![
        json!({"uri": REFERENCE_URI, "name": "Script API reference",
            "description": "Every function of the Rhai CAD API with examples.",
            "mimeType": "text/markdown"}),
        json!({"uri": GUIDE_URI, "name": "Workflow guide",
            "description": "Project layout, the work loop, rules and errors.",
            "mimeType": "text/markdown"}),
    ];
    resources.extend(EXAMPLES.iter().map(|(name, _)| {
        json!({"uri": format!("{EXAMPLE_URI}{name}"), "name": name,
            "description": "Example part script.", "mimeType": "text/plain"})
    }));
    json!({"resources": resources})
}

/// `resources/read`.
pub(super) fn read(params: &Value) -> Result<Value, RpcError> {
    let uri =
        params["uri"].as_str().ok_or((INVALID_PARAMS, "resources/read needs a uri".into()))?;
    let example = uri
        .strip_prefix(EXAMPLE_URI)
        .and_then(|name| EXAMPLES.iter().find(|(n, _)| *n == name))
        .map(|(_, script)| ("text/plain", *script));
    let (mime, text) = match uri {
        REFERENCE_URI => ("text/markdown", SCRIPT_API),
        GUIDE_URI => ("text/markdown", GUIDE),
        _ => example.ok_or((RESOURCE_NOT_FOUND, format!("unknown resource {uri}")))?,
    };
    Ok(json!({"contents": [{"uri": uri, "mimeType": mime, "text": text}]}))
}
