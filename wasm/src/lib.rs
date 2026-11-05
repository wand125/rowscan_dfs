//! Browser worker ABI. Replies belong to this module until the next call.
use rowscan_dfs::{BrowserSearch, Options, Puzzle};
use serde_json::{json, Value};
use std::cell::RefCell;

struct Saved {
    search: BrowserSearch,
    puzzle: Puzzle,
    values: Vec<u16>,
    budget: u64,
    max: u64,
}
#[derive(Default)]
struct Worker {
    saved: Option<Saved>,
    reply: Vec<u8>,
}
thread_local! { static WORKER: RefCell<Worker> = RefCell::new(Worker::default()); }

fn uint(options: &Value, name: &str, default: u64, maximum: u64) -> Result<u64, String> {
    let Some(value) = options.get(name) else {
        return Ok(default);
    };
    value
        .as_u64()
        .filter(|&n| n <= maximum)
        .ok_or_else(|| format!("{name} must be an integer in 0..={maximum}"))
}

fn decode(url: &str) -> Result<(Puzzle, Vec<u16>), String> {
    let (mut p, values) = Puzzle::decode_url(url)?;
    // Existing browser prepare contract numbers pairs by increasing URL label.
    let mut sorted = values.clone();
    sorted.sort_unstable();
    for v in &mut p.cells {
        if *v != 0 {
            *v = (sorted.binary_search(&values[*v as usize - 1]).unwrap() + 1) as u16;
        }
    }
    Ok((p, sorted))
}

fn options(text: &str) -> Result<Value, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("invalid options JSON: {e}"))?;
    if !value.is_object() {
        return Err("options JSON must be an object".into());
    }
    Ok(value)
}

fn settings(p: &Puzzle, v: &Value) -> Result<(u8, Options, u64, u64), String> {
    let direction = match v.get("direction") {
        None => p.recommended_direction(),
        Some(Value::String(s)) if s == "recommended" || s == "race" => p.recommended_direction(),
        Some(n) if n.as_i64() == Some(-1) => p.recommended_direction(),
        Some(n) => n
            .as_u64()
            .filter(|&n| n < 8)
            .ok_or("direction must be 0..7, -1, recommended or race")? as u8,
    };
    let diag_shortcut = match v.get("diag_shortcut") {
        None => false,
        Some(v) => v.as_bool().ok_or("diag_shortcut must be a boolean")?,
    };
    let options = Options {
        cut: uint(v, "cut", 25, usize::MAX as u64)? as usize,
        memo_bits: uint(v, "memo_bits", 24, 27)? as usize,
        diag_shortcut,
        ..Options::none()
    };
    let requested = uint(v, "race_budget", 0, u64::MAX)?;
    let budget = if requested == 1 {
        2 * (p.width * p.height).pow(2) as u64
    } else {
        requested
    };
    let max = uint(v, "max", 0, u64::MAX)?;
    Ok((direction, options, budget, max))
}

fn prepare(url: &str, text: &str) -> Result<Value, String> {
    let v = options(text)?;
    let (p, values) = decode(url)?;
    settings(&p, &v)?;
    Ok(json!({"width":p.width,"height":p.height,"pairs":p.pairs,
        "recommended":p.recommended_direction(),"value_map":values}))
}

fn report(saved: &Saved) -> Value {
    let (out, progress, elapsed) = saved.search.report();
    let answer = out.first_solution.map(
        |links| json!({"width":saved.puzzle.width,"height":saved.puzzle.height,"links":links,"has_empty":out.first_solution_has_empty}),
    );
    json!({"width":saved.puzzle.width,"height":saved.puzzle.height,
        "recommended":saved.puzzle.recommended_direction(),"value_map":saved.values,
        "direction":out.direction,"steps":out.steps,"solutions":out.solutions,
        "first_has_empty":out.first_solution_has_empty,
        "first_solution_has_empty":out.first_solution_has_empty,
        "elapsed_ms":elapsed,"memo_hits":out.memo_hits,"memo_stores":out.memo_stores,
        "memo_slots":out.memo_slots,"answer":answer,"progress":progress.progress,
        "finished":progress.finished,"race_budget":saved.budget,
        "est_total":if progress.estimated_total.is_finite() {Some(progress.estimated_total)} else {None}})
}

impl Worker {
    fn solve(&mut self, url: &str, text: &str) -> Result<Value, String> {
        let (p, values) = decode(url)?;
        let v = options(text)?;
        let (direction, options, budget, max) = settings(&p, &v)?;
        // Validation precedes replacement; a new valid solve releases the old table.
        self.saved = None;
        let direct_race = v.get("direction").and_then(Value::as_str) == Some("race");
        let probe_budget = if budget == 0 && direct_race {
            2 * (p.width * p.height).pow(2) as u64
        } else {
            budget
        };
        let make = |direction| Saved {
            search: BrowserSearch::new(p.clone(), direction, options),
            puzzle: p.clone(),
            values: values.clone(),
            budget: probe_budget,
            max,
        };
        let mut saved = if direct_race {
            // Keep only the best candidate so far; release each loser immediately.
            // The browser scheduler normally does this across separate workers.
            let mut best: Option<Saved> = None;
            for candidate_direction in 0..8 {
                let mut candidate = make(candidate_direction);
                candidate.search.run_budget(probe_budget, max);
                let (_, a, _) = candidate.search.report();
                let better = best.as_ref().is_none_or(|best| {
                    let (_, b, _) = best.search.report();
                    b.finished
                        .cmp(&a.finished)
                        .then_with(|| a.estimated_total.total_cmp(&b.estimated_total))
                        .then_with(|| (a.direction != direction).cmp(&(b.direction != direction)))
                        .then_with(|| a.direction.cmp(&b.direction))
                        .is_lt()
                });
                if better {
                    best = Some(candidate);
                }
            }
            let mut best = best.unwrap();
            if budget == 0 {
                best.search.run_budget(0, max);
            }
            best
        } else {
            make(direction)
        };
        if !direct_race {
            saved.search.run_budget(budget, max);
        }

        let result = report(&saved);
        if result["finished"] == false {
            self.saved = Some(saved);
        }
        Ok(result)
    }
    fn resume(&mut self, additional: u64) -> Result<Value, String> {
        let mut saved = self.saved.take().ok_or("no saved search to resume")?;
        saved.search.run_budget(additional, saved.max);
        let result = report(&saved);
        if result["finished"] == false {
            self.saved = Some(saved);
        }
        Ok(result)
    }
    fn reply(&mut self, result: Result<Value, String>) -> *const u8 {
        let value = result.unwrap_or_else(|error| json!({"error":error}));
        let bytes = serde_json::to_vec(&value).unwrap();
        self.reply.clear();
        self.reply
            .extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        self.reply.extend_from_slice(&bytes);
        self.reply.as_ptr()
    }
}

// Input buffers are allocated/freed by the caller with Emscripten malloc/free.
unsafe fn input<'a>(ptr: *const u8, len: usize) -> Result<&'a str, String> {
    if ptr.is_null() {
        return Err("null input pointer".into());
    }
    std::str::from_utf8(std::slice::from_raw_parts(ptr, len))
        .map_err(|_| "input must be UTF-8".into())
}

/// # Safety
/// Both input ranges must remain readable for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn dfs_prepare(
    url: *const u8,
    url_len: usize,
    opts: *const u8,
    opts_len: usize,
) -> *const u8 {
    WORKER.with(|w| {
        let result = input(url, url_len)
            .and_then(|url| input(opts, opts_len).and_then(|opts| prepare(url, opts)));
        w.borrow_mut().reply(result)
    })
}
/// # Safety
/// Both input ranges must remain readable for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn dfs_solve(
    url: *const u8,
    url_len: usize,
    opts: *const u8,
    opts_len: usize,
) -> *const u8 {
    WORKER.with(|w| {
        let mut w = w.borrow_mut();
        let result = input(url, url_len)
            .and_then(|url| input(opts, opts_len).and_then(|opts| w.solve(url, opts)));
        w.reply(result)
    })
}
#[no_mangle]
pub extern "C" fn dfs_resume() -> *const u8 {
    WORKER.with(|w| {
        let mut w = w.borrow_mut();
        let result = w.resume(0);
        w.reply(result)
    })
}
/// Resume the saved search for at most `additional` more branch steps (zero
/// exhausts it). An unfinished search stays saved for the next call, so a
/// scheduler can poll `progress` and `est_total` between slices.
#[no_mangle]
pub extern "C" fn dfs_resume_budget(additional: u32) -> *const u8 {
    WORKER.with(|w| {
        let mut w = w.borrow_mut();
        let result = w.resume(u64::from(additional));
        w.reply(result)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn continuation_and_validation() {
        let url = include_str!("../../tests/fixtures/urls.txt")
            .lines()
            .find(|l| l.starts_with("ex3 "))
            .unwrap()
            .split_once(' ')
            .unwrap()
            .1;
        for memo in [0, 12] {
            let mut w = Worker::default();
            assert!(w.resume(0).is_err());
            let start = w
                .solve(
                    url,
                    &json!({"direction":6,"cut":1,"memo_bits":memo,"race_budget":10}).to_string(),
                )
                .unwrap();
            assert_eq!(start["steps"], 10);
            assert_eq!(start["finished"], false);
            // A budgeted slice keeps the unfinished search saved and reports progress.
            let slice = w.resume(5).unwrap();
            assert_eq!(slice["steps"], 15);
            assert_eq!(slice["finished"], false);
            assert!(slice["progress"].as_f64().unwrap() < 1.0);
            let resumed = w.resume(0).unwrap();
            assert_eq!(resumed["finished"], true);
            assert_eq!(resumed["progress"], 1.0);
            assert!(w.resume(0).is_err());
            let plain = w
                .solve(
                    url,
                    &json!({"direction":6,"cut":1,"memo_bits":memo}).to_string(),
                )
                .unwrap();
            for key in ["steps", "solutions", "memo_hits", "memo_stores", "answer"] {
                assert_eq!(resumed[key], plain[key], "{key}");
            }
        }
        let mut w = Worker::default();
        for opts in [
            "[]",
            "null",
            "{",
            r#"{"direction":8}"#,
            r#"{"memo_bits":29}"#,
            r#"{"cut":-1}"#,
            r#"{"diag_shortcut":1}"#,
            r#"{"max":1.5}"#,
        ] {
            assert!(w.solve("p?numlin/3/1/1g1", opts).is_err(), "{opts}");
        }
        let result = w
            .solve("p?numlin/3/1/1g1", r#"{"memo_bits":0,"race_budget":1}"#)
            .unwrap();
        assert_eq!(result["race_budget"], 18);
        assert_eq!(result["solutions"], 1);
        assert_eq!(result["answer"]["links"], json!([2, 10, 8]));
        assert!(w.resume(0).is_err());
    }
}
