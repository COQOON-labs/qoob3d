//! Checks in scripts (docs/requirements/agent.md AG-CK-01/02, ADR-0054):
//! `check("name", || { … })` with `expect`, `expect_near`, `expect_between`,
//! and `part("Name")`, the built shape of a project part.
//!
//! Checks run in plain runs (`PartMode::All`); declaring and building one
//! part only record their names. A failed expectation is recorded and the
//! check goes on; a script error ends the check and is recorded with it.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use rhai::{Dynamic, Engine, FnPtr, NativeCallContext};
use serde_json::{Value, json};

use super::api::{Rr, num, runtime};
use super::{PartMode, RunOptions, RunOutput, ScriptError, Shape, read_brep, script_error};

/// What checks and `part("Name")` work on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckOptions {
    /// BREP files of the built parts `part("Name")` returns; other names
    /// are parts this run built itself, or errors.
    pub built: BTreeMap<String, PathBuf>,
    /// Run only the checks with these names; `None` = all.
    pub only: Option<Vec<String>>,
}

/// One recorded expectation (AG-CK-02).
#[derive(Debug, Clone, PartialEq)]
pub struct Expectation {
    pub text: String,
    pub passed: bool,
    /// The measured number, or the condition of `expect`.
    pub actual: Value,
    /// The wanted number, `[min, max]` of `expect_between`, or `true`.
    pub expected: Value,
    /// Tolerance of `expect_near`, otherwise null.
    pub tol: Value,
}

/// A check declared with `check(name, closure)`.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckResult {
    pub name: String,
    /// False if the closure did not run (declaring, or not selected).
    pub ran: bool,
    pub expectations: Vec<Expectation>,
    /// The script error that ended the check.
    pub error: Option<ScriptError>,
    /// Names asked from `part("Name")`, found or not.
    pub parts_read: Vec<String>,
}

impl CheckResult {
    /// The check ran without error and every expectation holds.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.ran && self.error.is_none() && self.expectations.iter().all(|e| e.passed)
    }
}

pub(super) fn register(engine: &mut Engine, output: &Rc<RefCell<RunOutput>>, options: &RunOptions) {
    // True while the closure of the last entry of `output.checks` runs.
    let active = Rc::new(Cell::new(false));
    register_part(engine, output, &active, options.check.built.clone());
    register_check(engine, output, &active, options);
    register_expectations(engine, output, &active);
}

fn register_part(
    engine: &mut Engine,
    output: &Rc<RefCell<RunOutput>>,
    active: &Rc<Cell<bool>>,
    files: BTreeMap<String, PathBuf>,
) {
    let (out, active) = (Rc::clone(output), Rc::clone(active));
    let loaded: RefCell<BTreeMap<String, Shape>> = RefCell::default();
    engine.register_fn("part", move |name: &str| -> Rr<Shape> {
        if active.get()
            && let Some(check) = out.borrow_mut().checks.last_mut()
        {
            check.parts_read.push(name.to_owned());
        }
        if let Some(shape) = loaded.borrow().get(name) {
            return Ok(shape.clone());
        }
        if let Some(file) = files.get(name) {
            let shape = read_brep(file).map_err(|e| runtime(format!("part {name:?}: {e}")))?;
            loaded.borrow_mut().insert(name.to_owned(), shape.clone());
            return Ok(shape);
        }
        let out = out.borrow();
        match out.shapes.iter().rev().find(|(n, _)| n == name) {
            Some((_, shape)) if out.parts.iter().any(|p| p.name == name) => Ok(shape.clone()),
            _ => Err(runtime(format!("part {name:?} is not built and current"))),
        }
    });
}

fn register_check(
    engine: &mut Engine,
    output: &Rc<RefCell<RunOutput>>,
    active: &Rc<Cell<bool>>,
    options: &RunOptions,
) {
    let (out, active) = (Rc::clone(output), Rc::clone(active));
    let run_all = options.parts == PartMode::All;
    let only = options.check.only.clone();
    engine.register_fn("check", move |ctx: NativeCallContext, name: &str, body: FnPtr| -> Rr<()> {
        if active.get() {
            return Err(runtime(format!("check {name:?}: checks cannot be nested")));
        }
        if out.borrow().checks.iter().any(|c| c.name == name) {
            return Err(runtime(format!("check {name:?} is declared twice")));
        }
        let ran = run_all && only.as_ref().is_none_or(|names| names.iter().any(|n| n == name));
        out.borrow_mut().checks.push(CheckResult {
            name: name.to_owned(),
            ran,
            expectations: Vec::new(),
            error: None,
            parts_read: Vec::new(),
        });
        if !ran {
            return Ok(());
        }
        active.set(true);
        let result: Rr<Dynamic> = body.call_within_context(&ctx, ());
        active.set(false);
        if let Err(e) = result
            && let Some(check) = out.borrow_mut().checks.last_mut()
        {
            check.error = Some(script_error(*e));
        }
        Ok(())
    });
}

fn register_expectations(
    engine: &mut Engine,
    output: &Rc<RefCell<RunOutput>>,
    active: &Rc<Cell<bool>>,
) {
    let (out, active) = (Rc::clone(output), Rc::clone(active));
    let record = Rc::new(move |function: &str, expectation: Expectation| -> Rr<()> {
        if !active.get() {
            return Err(runtime(format!("{function} is only available inside check(...)")));
        }
        if let Some(check) = out.borrow_mut().checks.last_mut() {
            check.expectations.push(expectation);
        }
        Ok(())
    });

    let rec = Rc::clone(&record);
    engine.register_fn("expect", move |cond: bool, text: &str| {
        let expectation = Expectation {
            text: text.to_owned(),
            passed: cond,
            actual: json!(cond),
            expected: json!(true),
            tol: Value::Null,
        };
        rec("expect", expectation)
    });

    let rec = Rc::clone(&record);
    engine.register_fn(
        "expect_near",
        move |actual: Dynamic, expected: Dynamic, tol: Dynamic, text: &str| {
            let actual = num(&actual, "expect_near: actual")?;
            let expected = num(&expected, "expect_near: expected")?;
            let tol = num(&tol, "expect_near: tol")?;
            let expectation = Expectation {
                text: text.to_owned(),
                // NaN compares false: it is never near.
                passed: (actual - expected).abs() <= tol,
                actual: json!(actual),
                expected: json!(expected),
                tol: json!(tol),
            };
            rec("expect_near", expectation)
        },
    );

    engine.register_fn(
        "expect_between",
        move |actual: Dynamic, min: Dynamic, max: Dynamic, text: &str| {
            let actual = num(&actual, "expect_between: actual")?;
            let min = num(&min, "expect_between: min")?;
            let max = num(&max, "expect_between: max")?;
            let expectation = Expectation {
                text: text.to_owned(),
                passed: actual >= min && actual <= max,
                actual: json!(actual),
                expected: json!([min, max]),
                tol: Value::Null,
            };
            record("expect_between", expectation)
        },
    );
}
