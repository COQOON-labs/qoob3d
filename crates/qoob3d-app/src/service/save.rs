//! `POST /save` (AG-SV-07, ADR-0054): a Git commit of the project files,
//! only when every part builds and every check passes. Git is the `git`
//! executable, run in the project folder; `.qoob3d/` and `out/` are never
//! added, whatever `.gitignore` says. The project folder must be the top
//! level of its repository, and the commit takes the same paths as the add,
//! so nothing else that is staged rides along (AG-HD-08).

use std::path::Path;
use std::process::{Command, Stdio};

use qoob3d_core::project::layout;
use serde::Deserialize;
use serde_json::json;

use super::jobs::Work;
use super::{Answer, Failure, Service, body, unknown_job};
use crate::worker::ErrorKind;

/// Never part of a save: service data and generated output.
const EXCLUDED: [&str; 2] = [layout::DATA_DIR, layout::OUT_DIR];

#[derive(Deserialize)]
struct SaveBody {
    message: String,
}

/// `git <args>` in `dir`: trimmed stdout, or the failure with stderr.
fn git(dir: &Path, args: &[&str]) -> Result<String, Failure> {
    let failed = |text: String| {
        Failure::new(400, ErrorKind::Git, format!("git {}: {text}", args.first().unwrap_or(&"")))
    };
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| failed(e.to_string()))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned());
    }
    let (stdout, stderr) =
        (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    // "nothing to commit" is on stdout.
    Err(failed(format!("{} {}", stderr.trim(), stdout.trim()).trim().to_owned()))
}

impl Service {
    /// `POST /save`: runs all checks, then commits; `{"commit": hash}`.
    pub(super) fn save(&self, bytes: &[u8]) -> Answer {
        let request: SaveBody = body(bytes)?;
        if request.message.trim().is_empty() {
            return Err(Failure::bad_request("a save needs a message"));
        }
        // Fails outside a repository, before any check runs.
        git(&self.dir, &["status", "--porcelain"])?;
        // AG-HD-08: the top level of a repository is the folder that holds
        // its `.git`; inside a larger repository the commit would land there.
        if !self.dir.join(".git").exists() {
            let message = "the project folder is not the top level of its Git repository; run \
                           `git init` in the project folder";
            return Err(Failure::new(400, ErrorKind::Git, message));
        }

        let id = self.enqueue(Work::Check(None), 0);
        self.wait(id, None);
        {
            let st = self.lock();
            let job = st.jobs.get(&id).ok_or_else(|| unknown_job(id))?;
            if let Some(e) = &job.error {
                return Err(Failure::from(e.clone()));
            }
            let result = job.result();
            let mut red: Vec<String> = job.failures().iter().map(|f| format!("part {f}")).collect();
            let checks = result["checks"].as_array().into_iter().flatten();
            red.extend(
                checks
                    .filter(|c| c["state"] != "passed")
                    .map(|c| format!("check {} failed", c["name"])),
            );
            if !red.is_empty() || !matches!(job.state, super::jobs::JobState::Done) {
                let message = format!("not saved, the project is not green: {}", red.join("; "));
                let mut failure = Failure::new(400, ErrorKind::Gate, message);
                failure.result = result;
                return Err(failure);
            }
        }

        // `git add` fails on an exclude pathspec that names an ignored path
        // (as in a project made by `init`); an ignored folder is not added
        // anyway, so its exclusion is left out there.
        let mut add = vec!["add", "--all", "--", "."];
        // Ignored paths are listed as `!! <folder>/`.
        let listed = git(
            &self.dir,
            &["status", "--porcelain", "--ignored", "--", EXCLUDED[0], EXCLUDED[1]],
        )?;
        let ignored = |folder: &str| {
            listed
                .lines()
                .any(|l| l.strip_prefix("!! ").is_some_and(|p| p.trim_end_matches('/') == folder))
        };
        let excludes = EXCLUDED.map(|folder| format!(":(exclude){folder}"));
        add.extend(
            EXCLUDED.iter().zip(&excludes).filter(|(f, _)| !ignored(f)).map(|(_, e)| e.as_str()),
        );
        git(&self.dir, &add)?;
        let mut commit = vec!["commit", "--quiet", "--message", &request.message, "--", "."];
        commit.extend(excludes.iter().map(String::as_str));
        git(&self.dir, &commit)?;
        let commit = git(&self.dir, &["log", "-1", "--format=%H"])?;
        Ok(json!({"commit": commit}))
    }
}
