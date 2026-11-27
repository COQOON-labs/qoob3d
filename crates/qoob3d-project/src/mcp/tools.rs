//! The tools of the MCP server (AG-MC-02): their schemas for `tools/list`
//! and `tools/call`. Most tools forward to the project service over HTTP
//! and hand its JSON envelope through; guide, reference, files and render
//! are answered here in the same envelope.

use std::time::{Duration, Instant};

use qoob3d_core::project::layout;
use serde_json::{Map, Value, json};

use super::resources::{GUIDE, api_reference};
use super::{INVALID_PARAMS, RpcError, Server, files, render};
use crate::client::{self, Client, START_TIMEOUT, segment, service_command_from_env};

/// Requests that do not wait for workers.
const QUICK: Duration = Duration::from_secs(30);
/// Requests that build first and answer when done.
const LONG: Duration = Duration::from_secs(3600);
/// Margin over `wait_s` for the answer of a waiting request.
const WAIT_MARGIN: Duration = Duration::from_secs(30);
/// `wait_s` of build, check and run_script when the caller gives none.
const DEFAULT_WAIT_S: f64 = 60.0;

/// JSON type of a tool argument.
#[derive(Clone, Copy)]
enum Kind {
    Text,
    Number,
    Integer,
    Flag,
    Names,
    /// A job id: integer or string.
    Id,
    /// A task state the agent may set (AG-TK-02): never `done`, `rejected`.
    AgentState,
    /// A box `{min: [x, y, z], max: [x, y, z]}`.
    Region,
}

/// `(name, kind, required, description)`.
type Arg = (&'static str, Kind, bool, &'static str);
/// `(name, description, arguments)`.
type Tool = (&'static str, &'static str, &'static [Arg]);

const WAIT: Arg = (
    "wait_s",
    Kind::Number,
    false,
    "Seconds to wait for the result (default 60). If it takes longer the answer is {job, \
     state}; poll it with the tool job.",
);

const TOOLS: [Tool; 19] = [
    (
        "project_status",
        "Parts with build state (current, outdated, failed, never_built), checks with state \
         (passed, failed, stale, never_run), jobs and workers of the project.",
        &[],
    ),
    (
        "guide",
        "The workflow guide: project layout, the work loop, rules, errors. Read it first.",
        &[],
    ),
    (
        "api_reference",
        "The script API reference (Rhai CAD functions with examples). Read the sections you \
         need before writing a script.",
        &[(
            "search",
            Kind::Text,
            false,
            "Return only the sections whose heading or text contains this string \
             (case-insensitive), e.g. \"fillet\". Without it: the whole reference.",
        )],
    ),
    (
        "task_list",
        "The tasks, requirements and ideas of the human: id, title, kind, state, parts, checks.",
        &[(
            "state",
            Kind::Text,
            false,
            "Only tasks in this state: open, in_progress, blocked, review, done or rejected.",
        )],
    ),
    (
        "task_get",
        "One task with its body (the human's words) and its log, and `project`: the part and \
         check names of the project with their states.",
        &[("id", Kind::Text, true, "Task id, e.g. T-0001.")],
    ),
    (
        "task_create",
        "Create a task file in tasks/ (state open).",
        &[
            ("title", Kind::Text, true, "Short title; the file name is derived from it."),
            ("kind", Kind::Text, false, "task (default), requirement or idea."),
            ("body", Kind::Text, false, "Markdown text of the task."),
            ("parts", Kind::Names, false, "Parts that must be built and current for review."),
            ("checks", Kind::Names, false, "Checks that must have passed for review."),
        ],
    ),
    (
        "task_update",
        "Change the state of a task, bind parts and checks to it and/or add a note to its log, \
         as the agent. Allowed: open -> in_progress, in_progress -> blocked (note = your \
         question), blocked -> in_progress, in_progress -> review (gate: at least one part and \
         one check bound, parts current, checks passed; an idea needs a note only), review -> \
         in_progress. Only the human sets done or rejected.",
        &[
            ("id", Kind::Text, true, "Task id, e.g. T-0001."),
            ("state", Kind::AgentState, false, "The new state."),
            ("note", Kind::Text, false, "Log entry; required for blocked (the question)."),
            (
                "parts",
                Kind::Names,
                false,
                "Replaces the parts bound to the task (the ones you built for it); only while \
                 the task is open or in_progress.",
            ),
            (
                "checks",
                Kind::Names,
                false,
                "Replaces the checks bound to the task; only while it is open or in_progress.",
            ),
        ],
    ),
    (
        "file_list",
        "Project files the file tools may read: qoob3d.toml and .rhai, .toml, .md files under \
         parts/, lib/, values/, checks/, tasks/. `imports` lists the reference files there \
         (.step, .stp, .stl, .3mf) with their size; scripts read them with import_step and \
         import_mesh.",
        &[("dir", Kind::Text, false, "Only this folder, e.g. \"parts\" or \"lib/util\".")],
    ),
    (
        "file_read",
        "Read a project file (at most 1 MiB).",
        &[("path", Kind::Text, true, "Path relative to the project, e.g. parts/box.rhai.")],
    ),
    (
        "file_write",
        "Create or replace a project file (at most 1 MiB): qoob3d.toml or a .rhai, .toml, .md \
         file under parts/, lib/, values/, checks/. Task files are not writable. A part or \
         check file answers with what it declares (declared: parts, checks) or its load_error, \
         a lib/ file with the load_errors of all part and check files.",
        &[
            ("path", Kind::Text, true, "Path relative to the project, e.g. parts/box.rhai."),
            ("content", Kind::Text, true, "The whole new content of the file."),
        ],
    ),
    (
        "build",
        "Build parts and their dependencies in worker processes. Result per part: state (built, \
         cached, failed), volume, bbox, valid, error with file, line, column.",
        &[("parts", Kind::Names, false, "Part names; all parts without it."), WAIT],
    ),
    (
        "job",
        "State, progress and result of a job that build, check or run_script returned.",
        &[
            ("id", Kind::Id, true, "The job id."),
            ("cancel", Kind::Flag, false, "true: cancel the job."),
        ],
    ),
    (
        "run_script",
        "Run a throwaway script in a worker to answer a question: part(\"Name\") returns a built \
         part, value(\"id\") a value. Nothing is written. Result: the shapes given to show() \
         with volume, area, bbox, center, valid, solids, and the printed lines.",
        &[("source", Kind::Text, true, "The Rhai script."), WAIT],
    ),
    (
        "measure",
        "Volume, area, bbox, center, valid and solids of part a; with b also the smallest \
         distance and the common volume (an overlap is a finding, not an error).",
        &[("a", Kind::Text, true, "Part name."), ("b", Kind::Text, false, "Second part name.")],
    ),
    (
        "check",
        "Build stale parts, then run the checks of checks/*.rhai. Result per check: state, \
         every expectation with text, actual, expected, tol, and a script error if any.",
        &[("names", Kind::Names, false, "Check names; all checks without it."), WAIT],
    ),
    (
        "render",
        "Render built parts (stale ones are built first), or the shapes a throwaway script \
         shows, as a PNG image, orthographic, camera fitted to what is rendered or to a region. \
         Returns the image, its path under out/renders/, the bbox of the rendered parts and per \
         view where the camera looks along and which axis is up.",
        &[
            ("view", Kind::Text, false, "iso (default), front, back, left, right, top or bottom."),
            (
                "views",
                Kind::Names,
                false,
                "2 to 6 views, e.g. [\"iso\", \"front\", \"top\"]: one contact sheet image with \
                 a label per view instead of `view`; width and height are the size of one view.",
            ),
            ("parts", Kind::Names, false, "Part names; all parts without it."),
            (
                "source",
                Kind::Text,
                false,
                "A throwaway script instead of parts: its show(shape, name) shapes are rendered \
                 (part(\"Name\") and value(\"id\") work as in run_script). Nothing is written.",
            ),
            (
                "region",
                Kind::Region,
                false,
                "Zoom: frame this box {min: [x, y, z], max: [x, y, z]} (mm) instead of everything.",
            ),
            ("width", Kind::Integer, false, "Image width in pixels (default 1024)."),
            ("height", Kind::Integer, false, "Image height in pixels (default 768)."),
        ],
    ),
    (
        "export",
        "Write out/<part>.<format> for printing or exchange (parts are built first if needed), \
         STL and 3MF in each part's print pose (option pose of part). With kit: one file \
         out/<kit>.<format> with all parts, each count times, arranged on the bed. With \
         project: one Bambu Studio project out/<project>.3mf with all parts on plates, each in \
         the filament of its colour (option color of part). The answer lists per file the \
         size, triangles, whether the mesh is watertight, and warnings.",
        &[
            ("parts", Kind::Names, true, "Part names."),
            ("format", Kind::Text, true, "stl, 3mf (one object per part) or step."),
            ("kit", Kind::Text, false, "File name of one file with all parts (stl or 3mf)."),
            ("bed_mm", Kind::Number, false, "Bed width for arranging a kit (default 256)."),
            ("project", Kind::Text, false, "File name of a Bambu Studio project (format 3mf)."),
            (
                "multicolor",
                Kind::Flag,
                false,
                "For project: parts with print_with print inside their host as one multi-colour \
                 object (AMS). Default false: every part its own object, one colour per plate.",
            ),
            (
                "printer",
                Kind::Text,
                false,
                "For project: printer id (bed and presets), default bambu-lab-p1s; a wrong \
                 id answers with the known ones.",
            ),
        ],
    ),
    ("values", "All target values with source and status, and the ids that are assumptions.", &[]),
    (
        "save_state",
        "Run all checks and, if every part builds and every check passes, commit the project \
         files with Git. Refused with kind gate otherwise.",
        &[("message", Kind::Text, true, "The commit message.")],
    ),
];

fn schema((_, kind, _, description): &Arg) -> Value {
    let mut property = match kind {
        Kind::Text => json!({"type": "string"}),
        Kind::Number => json!({"type": "number"}),
        Kind::Integer => json!({"type": "integer"}),
        Kind::Flag => json!({"type": "boolean"}),
        Kind::Names => json!({"type": "array", "items": {"type": "string"}}),
        Kind::Id => json!({"type": ["integer", "string"]}),
        Kind::AgentState => {
            json!({"type": "string", "enum": ["in_progress", "blocked", "review"]})
        }
        Kind::Region => {
            let corner =
                json!({"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3});
            json!({"type": "object", "properties": {"min": corner, "max": corner}, "required": ["min", "max"]})
        }
    };
    property["description"] = json!(description);
    property
}

/// `tools/list`.
pub(super) fn list() -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .map(|(name, description, args)| {
            let properties: Map<String, Value> =
                args.iter().map(|arg| (arg.0.to_owned(), schema(arg))).collect();
            let required: Vec<&str> = args.iter().filter(|a| a.2).map(|a| a.0).collect();
            json!({
                "name": name,
                "description": description,
                "inputSchema": {"type": "object", "properties": properties, "required": required},
            })
        })
        .collect();
    json!({"tools": tools})
}

/// What a tool answers.
enum Reply {
    /// The envelope of the service, handed through.
    Envelope(Value),
    Result(Value),
    /// A result and a PNG as base64.
    Image(Value, String),
}

/// A tool call refused here: `kind` and `message` of the envelope's error.
struct Refusal(&'static str, String);

fn bad_request(message: String) -> Refusal {
    Refusal("bad_request", message)
}

/// The arguments `keys` that are present, as a request body.
fn pick(args: &Value, keys: &[&str]) -> Value {
    let present = keys.iter().filter(|k| !args[**k].is_null());
    Value::Object(present.map(|k| ((*k).to_owned(), args[*k].clone())).collect())
}

impl Server {
    /// `tools/call`: one text content with the envelope `{ok, duration_s,
    /// warnings, result|error}`, `isError = !ok`; `render` adds the image.
    pub(super) fn call(&self, params: &Value) -> Result<Value, RpcError> {
        let name = params["name"].as_str().unwrap_or_default();
        let Some((_, _, declared)) = TOOLS.iter().find(|t| t.0 == name) else {
            return Err((INVALID_PARAMS, format!("unknown tool {name:?}")));
        };
        let none = json!({});
        let args = if params["arguments"].is_null() { &none } else { &params["arguments"] };
        let start = Instant::now();
        let missing = declared.iter().find(|a| a.2 && args[a.0].is_null());
        let reply = match missing {
            Some(arg) => Err(bad_request(format!("{name}: the argument {:?} is missing", arg.0))),
            None => self.dispatch(name, args),
        };
        let done = |result: Value| {
            json!({"ok": true, "duration_s": start.elapsed().as_secs_f64(), "warnings": [],
                "result": result})
        };
        let (envelope, image) = match reply {
            Ok(Reply::Envelope(envelope)) => (envelope, None),
            Ok(Reply::Result(result)) => (done(result), None),
            Ok(Reply::Image(result, png)) => (done(result), Some(png)),
            Err(Refusal(kind, message)) => (
                json!({"ok": false, "duration_s": start.elapsed().as_secs_f64(), "warnings": [],
                    "result": null, "error": {"kind": kind, "message": message}}),
                None,
            ),
        };
        let mut content = vec![json!({"type": "text", "text": envelope.to_string()})];
        if let Some(data) = image {
            content.push(json!({"type": "image", "data": data, "mimeType": "image/png"}));
        }
        Ok(json!({"content": content, "isError": envelope["ok"] != json!(true)}))
    }

    fn dispatch(&self, name: &str, args: &Value) -> Result<Reply, Refusal> {
        let dir = self.project.dir();
        let text = |key: &str| args[key].as_str().unwrap_or_default();
        let local = |result: Result<Value, String>| result.map(Reply::Result).map_err(bad_request);
        match name {
            "project_status" => self.service("GET", "/status", None, QUICK),
            "values" => self.service("GET", "/values", None, QUICK),
            "guide" => Ok(Reply::Result(json!({"text": GUIDE}))),
            "api_reference" => match api_reference(args["search"].as_str()) {
                Some(reference) => Ok(Reply::Result(json!({"text": reference}))),
                None => Err(Refusal(
                    "not_found",
                    format!("no section of the script API reference contains {:?}", text("search")),
                )),
            },
            "task_list" => self.task_list(args["state"].as_str()),
            "task_get" => self.task_get(text("id")),
            "task_create" => {
                let body = pick(args, &["title", "kind", "body", "parts", "checks"]);
                self.service("POST", "/task", Some(body), QUICK)
            }
            "task_update" => {
                // Whatever the caller sends: over MCP the role is the agent.
                let mut body = pick(args, &["state", "note", "parts", "checks"]);
                body["role"] = json!("agent");
                self.service("POST", &format!("/task/{}", segment(text("id"))), Some(body), QUICK)
            }
            "file_list" => local(files::list(dir, args["dir"].as_str())),
            "file_read" => local(files::read(dir, text("path"))),
            "file_write" => match args["content"].as_str() {
                Some(content) => self.file_write(text("path"), content),
                None => Err(bad_request("file_write: content must be a string".to_owned())),
            },
            "build" => self.waiting("/build", pick(args, &["parts", "wait_s"])),
            "check" => self.waiting("/check", pick(args, &["names", "wait_s"])),
            "run_script" => self.waiting("/run", pick(args, &["source", "wait_s"])),
            "job" => {
                let id = args["id"].as_str().map_or_else(|| args["id"].to_string(), str::to_owned);
                let path = format!("/job/{}", segment(&id));
                if args["cancel"] == json!(true) {
                    self.service("POST", &format!("{path}/cancel"), None, QUICK)
                } else {
                    self.service("GET", &path, None, QUICK)
                }
            }
            "measure" => self.service("POST", "/measure", Some(pick(args, &["a", "b"])), LONG),
            "export" => self.service(
                "POST",
                "/export",
                Some(pick(
                    args,
                    &["parts", "format", "kit", "bed_mm", "project", "multicolor", "printer"],
                )),
                LONG,
            ),
            "save_state" => self.service("POST", "/save", Some(pick(args, &["message"])), LONG),
            "render" => {
                let (mut result, png) =
                    render::render(dir, args).map_err(|message| Refusal("render", message))?;
                // A script's shapes are not parts: their boxes are in run_script.
                if args["source"].is_null() {
                    result["bbox"] = self.bbox(&args["parts"])?;
                }
                Ok(Reply::Image(result, png))
            }
            _ => Err(bad_request(format!("the tool {name:?} is not implemented"))),
        }
    }

    /// One request to the project service, started on demand (AG-MC-01).
    fn service(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        timeout: Duration,
    ) -> Result<Reply, Refusal> {
        let unreachable = |e: anyhow::Error| Refusal(client::error_kind(&e), format!("{e:#}"));
        let client = Client::connect(&self.project, &service_command_from_env(), START_TIMEOUT)
            .map_err(unreachable)?;
        client.call(method, path, body.as_ref(), timeout).map(Reply::Envelope).map_err(unreachable)
    }

    /// A request with `wait_s` (build, check, run): the default wait, and an
    /// HTTP timeout that outlasts it.
    fn waiting(&self, path: &str, mut body: Value) -> Result<Reply, Refusal> {
        let wait_s = body["wait_s"].as_f64().unwrap_or(DEFAULT_WAIT_S);
        body["wait_s"] = json!(wait_s);
        let wait = Duration::try_from_secs_f64(wait_s).unwrap_or(Duration::ZERO);
        self.service("POST", path, Some(body), wait + WAIT_MARGIN)
    }

    /// `file_write`; a part or check file comes back with what it declares
    /// (`declared: {parts, checks}`) or its `load_error`, a `lib/` file with
    /// the `load_errors` of all files, once the service has discovered them
    /// (ADR-0055). So `/status` shows the new parts afterwards. A script in a
    /// sub folder of `parts/` or `checks/` gets a `note` instead.
    fn file_write(&self, path: &str, content: &str) -> Result<Reply, Refusal> {
        let mut result = files::write(self.project.dir(), path, content).map_err(bad_request)?;
        if path == layout::CONFIG_FILE {
            // Finding 3.5: an agent restarted the service to apply new limits.
            result["note"] = json!(
                "[limits], [mesh] and [cache] apply from the next job; [workers] and [service] \
                 from the next start of the service"
            );
            return Ok(Reply::Result(result));
        }
        let folder = path.split('/').next().unwrap_or_default();
        if !path.ends_with(".rhai") || !["parts", "checks", "lib"].contains(&folder) {
            return Ok(Reply::Result(result));
        }
        if folder != "lib" && path.matches('/').count() > 1 {
            // Sub folders are not searched (AG-PJ-01).
            result["note"] = json!(
                "not discovered: only parts/*.rhai and checks/*.rhai (directly in the folder) \
                 are part and check files"
            );
            return Ok(Reply::Result(result));
        }
        let files = if folder == "lib" { Value::Null } else { json!([path]) };
        // The file is written either way; a service that cannot say what it
        // declares is reported next to it.
        let answer = match self.waiting("/discover", json!({"files": files})) {
            Ok(Reply::Envelope(envelope)) if envelope["ok"] == json!(true) => {
                envelope["result"].clone()
            }
            Ok(Reply::Envelope(envelope)) => {
                result["discovery_error"] = envelope["error"].clone();
                return Ok(Reply::Result(result));
            }
            Err(Refusal(kind, message)) => {
                result["discovery_error"] = json!({"kind": kind, "message": message});
                return Ok(Reply::Result(result));
            }
            Ok(_) => return Ok(Reply::Result(result)),
        };
        let known = answer["files"].as_array().cloned().unwrap_or_default();
        if folder == "lib" {
            let errors: Vec<&Value> =
                known.iter().filter(|f| f["state"] == "failed").map(|f| &f["error"]).collect();
            result["load_errors"] = json!(errors);
        } else if let Some(f) = known.first() {
            match f["state"].as_str() {
                Some("declared") => {
                    result["declared"] = json!({"parts": f["parts"], "checks": f["checks"]});
                }
                Some("failed") => result["load_error"] = f["error"].clone(),
                _ => {}
            }
        }
        if !answer["discovering"].as_array().is_none_or(Vec::is_empty) {
            result["discovering"] = answer["discovering"].clone();
        }
        Ok(Reply::Result(result))
    }

    /// `GET /task/<id>` plus `project`: names and states of the parts and
    /// checks from `/status`, so a new task needs no status call (AG-MC-10).
    fn task_get(&self, id: &str) -> Result<Reply, Refusal> {
        let mut reply = self.service("GET", &format!("/task/{}", segment(id)), None, QUICK)?;
        if let Reply::Envelope(envelope) = &mut reply
            && envelope["ok"] == json!(true)
            && let Reply::Envelope(status) = self.service("GET", "/status", None, QUICK)?
        {
            let states = |key: &str| -> Vec<Value> {
                let list = status["result"][key].as_array().into_iter().flatten();
                list.map(|e| json!({"name": e["name"], "state": e["state"]})).collect()
            };
            envelope["result"]["project"] =
                json!({"parts": states("parts"), "checks": states("checks")});
        }
        Ok(reply)
    }

    /// The bounding box `{min, max, size}` of `parts` (all if null) from
    /// their build results (AG-MC-09); the render has just built them.
    fn bbox(&self, parts: &Value) -> Result<Value, Refusal> {
        let body = json!({"parts": parts, "wait_s": DEFAULT_WAIT_S});
        let Reply::Envelope(built) = self.waiting("/build", body)? else {
            return Ok(Value::Null);
        };
        let selected = |name: &String| parts.as_array().is_none_or(|p| p.contains(&json!(name)));
        let results = built["result"]["parts"].as_object().into_iter().flatten();
        let boxes: Vec<&Value> =
            results.filter(|(name, _)| selected(name)).map(|(_, part)| &part["bbox"]).collect();
        let corner = |key: &str, pick: fn(f64, f64) -> f64| -> Option<Vec<f64>> {
            let axis = |i: usize| {
                boxes.iter().map(|b| b[key][i].as_f64()).reduce(|a, b| Some(pick(a?, b?)))?
            };
            (0..3).map(axis).collect()
        };
        let (Some(min), Some(max)) = (corner("min", f64::min), corner("max", f64::max)) else {
            return Ok(Value::Null);
        };
        let size: Vec<f64> = min.iter().zip(&max).map(|(lo, hi)| hi - lo).collect();
        Ok(json!({"min": min, "max": max, "size": size}))
    }

    /// `GET /tasks`, filtered by `state` here.
    fn task_list(&self, state: Option<&str>) -> Result<Reply, Refusal> {
        let mut reply = self.service("GET", "/tasks", None, QUICK)?;
        if let (Reply::Envelope(envelope), Some(state)) = (&mut reply, state)
            && let Some(tasks) = envelope["result"].as_array_mut()
        {
            tasks.retain(|t| t["state"] == state);
        }
        Ok(reply)
    }
}
