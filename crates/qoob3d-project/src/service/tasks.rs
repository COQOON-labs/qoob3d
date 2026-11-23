//! Task endpoints (AG-SV-04, ADR-0054): the task inbox of
//! `qoob3d_core::tasks` over HTTP. The review gate is evaluated here, from
//! the service's own part and check states, never from what a client says.
//! Creating and changing tasks is serialised (AG-HD-07).

use std::sync::PoisonError;
use std::time::{SystemTime, UNIX_EPOCH};

use qoob3d_core::tasks::{self, Change, Role, Task, TaskError, TaskKind, TaskState, Tasks};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Answer, Failure, Service, body};
use crate::worker::ErrorKind;

#[derive(Deserialize)]
struct CreateBody {
    title: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    parts: Vec<String>,
    #[serde(default)]
    checks: Vec<String>,
}

#[derive(Deserialize)]
struct UpdateBody {
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    note: Option<String>,
    /// Replace the bound parts and checks (AG-TK-05).
    #[serde(default)]
    parts: Option<Vec<String>>,
    #[serde(default)]
    checks: Option<Vec<String>>,
    role: String,
}

impl From<TaskError> for Failure {
    fn from(error: TaskError) -> Self {
        match error {
            TaskError::UnknownTask { .. } => Self::not_found(error),
            TaskError::Gate { .. } => Self::new(400, ErrorKind::Gate, error),
            _ => Self::bad_request(error),
        }
    }
}

/// Today (UTC) as `YYYY-MM-DD`: days since 1970 to a civil date (Howard
/// Hinnant's `civil_from_days`).
fn today() -> String {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let z = seconds / 86_400 + 719_468;
    let (era, doe) = (z / 146_097, z % 146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// A task of `/tasks`; with `body` and `log` for `/task/<id>`.
fn view(task: &Task, full: bool) -> Value {
    let mut v = json!(task);
    if !full && let Some(fields) = v.as_object_mut() {
        fields.remove("body");
        fields.remove("log");
    }
    v
}

impl Service {
    /// `GET /tasks`.
    pub(super) fn tasks(&self) -> Answer {
        let tasks = Tasks::load(&self.dir)?;
        Ok(tasks.all().iter().map(|t| view(t, false)).collect())
    }

    /// `GET /task/<id>`.
    pub(super) fn task(&self, id: &str) -> Answer {
        let tasks = Tasks::load(&self.dir)?;
        let task = tasks.get(id).ok_or_else(|| TaskError::UnknownTask { id: id.to_owned() })?;
        Ok(view(task, true))
    }

    /// `POST /task`.
    pub(super) fn task_create(&self, bytes: &[u8]) -> Answer {
        let request: CreateBody = body(bytes)?;
        let kind = match request.kind.as_deref() {
            None => TaskKind::Task,
            Some(name) => TaskKind::from_name(name).ok_or_else(|| {
                Failure::bad_request(format!(
                    "unknown kind {name:?}: use task, requirement or idea"
                ))
            })?,
        };
        let _writing = self.task_writes.lock().unwrap_or_else(PoisonError::into_inner);
        let task = tasks::create(
            &self.dir,
            &request.title,
            kind,
            &request.body,
            &request.parts,
            &request.checks,
            &today(),
        )?;
        Ok(view(&task, true))
    }

    /// `POST /task/<id>`: `role` is required, so nobody is taken for the
    /// human by default.
    pub(super) fn task_update(&self, id: &str, bytes: &[u8]) -> Answer {
        let request: UpdateBody = body(bytes)?;
        let role = match request.role.as_str() {
            "agent" => Role::Agent,
            "human" => Role::Human,
            other => {
                return Err(Failure::bad_request(format!(
                    "unknown role {other:?}: use agent or human"
                )));
            }
        };
        let state = request
            .state
            .as_deref()
            .map(|name| {
                TaskState::from_name(name)
                    .ok_or_else(|| Failure::bad_request(format!("unknown state {name:?}")))
            })
            .transpose()?;
        let (gate, load_errors) = self.gate_input();
        // AG-HD-09: a project that does not load has no part states; say so
        // instead of calling every part missing.
        if role == Role::Agent && state == Some(TaskState::Review) && !load_errors.is_empty() {
            let message = format!(
                "task {id} cannot go to review: the project does not load: {}",
                load_errors.join("; ")
            );
            return Err(Failure::new(400, ErrorKind::Gate, message));
        }
        let change = Change {
            parts: request.parts.as_deref(),
            checks: request.checks.as_deref(),
            state,
            note: request.note.as_deref(),
        };
        let _writing = self.task_writes.lock().unwrap_or_else(PoisonError::into_inner);
        let task = tasks::apply(&self.dir, id, role, &change, &today(), &gate)?;
        Ok(view(&task, true))
    }
}
