//! Task endpoints (AG-SV-04, ADR-0054): the task inbox of
//! `qoob3d_core::tasks` over HTTP. The review gate is evaluated here, from
//! the service's own part and check states, never from what a client says.

use std::time::{SystemTime, UNIX_EPOCH};

use qoob3d_core::tasks::{self, Role, Task, TaskError, TaskKind, TaskState, Tasks};
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
        let gate = self.gate_input();
        let note = request.note.as_deref();
        let task = tasks::update(&self.dir, id, role, state, note, &today(), &gate)?;
        Ok(view(&task, true))
    }
}
