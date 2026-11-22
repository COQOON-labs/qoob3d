//! Size limits of declaration discovery (docs/requirements/agent.md
//! AG-HD-02): a file that doubles a string forty times at its top level gets
//! a clear load error with its line. Discovery runs in a worker (ADR-0056),
//! whose memory limit stops what the measurements below do not see.
//!
//! Rhai's own size limits check what a function call returns or changes. An
//! assignment into a container (`m[k] = v`) grows it unchecked, and Rhai has
//! no hook that sees the variables of a running script. So the script runs
//! in stages: with a budget of operations that doubles from
//! `STAGE_OPERATIONS`, from the start each time (Rhai cannot resume a run),
//! and after every stage the variables are measured. Doubling keeps the
//! total cost below twice the last stage, linear in the operations of the
//! script (ADR-0056); a loop can pile up unseen what fits between two
//! measurements, which the worker's memory limit bounds. A script that fits
//! the first stage costs nothing extra.
//! A measurement notices the growth after the statement that caused it;
//! `locate` finds that statement by running the script with fewer
//! operations.

use std::cell::Cell;
use std::rc::Rc;

use rhai::{Array, Dynamic, Engine, EvalAltResult, ImmutableString, Map, Position, Scope};

use super::ScriptError;

const MAX_STRING_BYTES: usize = 65_536;
const MAX_ARRAY_ELEMENTS: usize = 10_000;
const MAX_MAP_ENTRIES: usize = 1_000;
pub(super) const MAX_OPERATIONS: u64 = 1_000_000;
/// Operations of the first stage; every further stage doubles the budget.
pub(super) const STAGE_OPERATIONS: u64 = 4096;
/// Operations of one statement at most whose position `locate` looks for.
const STATEMENT_OPERATIONS: u64 = 16;

/// What is too large, in the words of Rhai's own size errors.
#[derive(Debug, Clone, Copy)]
pub(super) struct Oversize(&'static str);

/// What `Engine::run_with_scope` returns.
pub(super) type Outcome = Result<(), Box<EvalAltResult>>;

/// Array elements, map entries and string bytes of a value, nested ones
/// included (counted as Rhai counts them).
fn sizes(value: &Dynamic) -> [usize; 3] {
    if value.is_int() || value.is_float() {
        return [0; 3];
    }
    let add = |total: [usize; 3], item: &Dynamic| {
        let s = sizes(item);
        [total[0] + s[0], total[1] + s[1], total[2] + s[2]]
    };
    if let Some(array) = value.read_lock::<Array>() {
        array.iter().fold([array.len(), 0, 0], add)
    } else if let Some(map) = value.read_lock::<Map>() {
        map.values().fold([0, map.len(), 0], add)
    } else if let Some(text) = value.read_lock::<ImmutableString>() {
        [0, 0, text.len()]
    } else {
        [0; 3]
    }
}

/// What is too large among the variables a run left behind.
pub(super) fn found(scope: &Scope) -> Option<Oversize> {
    scope.iter_raw().find_map(|(_, _, value)| {
        let [elements, entries, bytes] = sizes(value);
        if bytes > MAX_STRING_BYTES {
            Some(Oversize("Length of string"))
        } else if elements > MAX_ARRAY_ELEMENTS {
            Some(Oversize("Size of array/BLOB"))
        } else if entries > MAX_MAP_ENTRIES {
            Some(Oversize("Size of object map"))
        } else {
            None
        }
    })
}

/// Sets the limits of declaration discovery. Returns the counter of the
/// operations a run has started.
pub(super) fn apply(engine: &mut Engine) -> Rc<Cell<u64>> {
    engine.set_max_string_size(MAX_STRING_BYTES);
    engine.set_max_array_size(MAX_ARRAY_ELEMENTS);
    engine.set_max_map_size(MAX_MAP_ENTRIES);
    // The built-in shortcut for `a += b` skips the size check of arrays.
    engine.set_fast_operators(false);
    let operations = Rc::new(Cell::new(0));
    let count = Rc::clone(&operations);
    engine.on_progress(move |n| {
        count.set(n);
        None
    });
    operations
}

/// The run stopped because its budget of operations was used up.
pub(super) fn out_of_operations(outcome: &Outcome) -> bool {
    fn stopped(error: &EvalAltResult) -> bool {
        match error {
            EvalAltResult::ErrorTooManyOperations(_) => true,
            EvalAltResult::ErrorInFunctionCall(_, _, inner, _)
            | EvalAltResult::ErrorInModule(_, inner, _) => stopped(inner),
            _ => false,
        }
    }
    outcome.as_ref().is_err_and(|error| stopped(error))
}

/// The `Limit` error for something that `found` reports after a run of
/// `over` operations but not after one of `clean` operations. `rerun(n)`
/// runs the script again with at most `n` operations and says whether it is
/// found there. The longest run that finds nothing stops inside the
/// statement that crosses the limit, which gives the position. That
/// statement is usually among the last ones, so the search starts at the
/// end.
pub(super) fn locate(
    what: Oversize,
    (mut clean, mut over): (u64, u64),
    rerun: impl Fn(u64) -> (Outcome, bool),
) -> ScriptError {
    let stopped_at = |outcome: Outcome| outcome.err().map_or(Position::NONE, |e| e.position());
    let mut position = Position::NONE;
    let mut step = 1;
    while over - clean > 1 {
        let middle = over.saturating_sub(step).max(clean + (over - clean) / 2);
        step = step.saturating_mul(2);
        let (outcome, found) = rerun(middle);
        if found {
            over = middle;
        } else {
            (clean, position, step) = (middle, stopped_at(outcome), u64::MAX);
        }
    }
    // The last operation of a statement may have no position (indexing);
    // the ones before it belong to the same statement.
    let mut back = clean;
    while position.is_none() && back > 1 && clean - back < STATEMENT_OPERATIONS {
        back -= 1;
        position = stopped_at(rerun(back).0);
    }
    let (line, column) = super::line_column(position);
    ScriptError::Limit { line, column, message: format!("{} too large", what.0) }
}
