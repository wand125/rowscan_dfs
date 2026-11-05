//! Budgeted direction selection. Only this loop has a budget check.
use super::*;
#[cfg(feature = "browser")]
#[path = "browser.rs"]
pub mod browser;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct CandidateReport {
    pub direction: u8,
    pub steps: u64,
    pub progress: f64,
    pub estimated_total: f64,
    pub finished: bool,
}

#[derive(Debug)]
pub struct RaceOutcome {
    /// Includes the selected candidate's race work and solutions.
    pub outcome: Outcome,
    pub candidates: Vec<CandidateReport>,
    pub budget: u64,
    /// Sum over every candidate, including the winner.
    pub race_steps: u64,
    pub race_seconds: f64,
    pub continuation_seconds: f64,
    pub total_seconds: f64,
}

pub(super) struct Candidate<C: CutState> {
    pub board: Board<C>,
    pub out: Outcome,
    pub memo: Option<Memo>,
    pub good: bool,
    pub started: bool,
    pub finished: bool,
    pub backtracking: bool,
}

// The binary fraction of consumed alternatives. Exhaustion is represented
// explicitly: an empty open path alone also describes a fresh search.
fn progress(path: impl Iterator<Item = u8>, finished: bool) -> f64 {
    if finished {
        return 1.0;
    }
    let mut weight = 0.5;
    let mut value = 0.0;
    for taken in path {
        value += f64::from(taken) * weight;
        weight *= 0.5;
        if weight == 0.0 {
            break;
        }
    }
    value.min(1.0)
}

// A failed cut scan can consume the final budget step before the sibling
// branch in the same loop iteration. Save that phase rather than overshoot.
pub(super) fn resume_alternative<C: CutState, const DIAG: bool>(board: &mut Board<C>) -> bool {
    let frame = board.choices.last().unwrap();
    let (checkpoint, c, zero) = (frame.trail_len, frame.cell as usize, frame.zero);
    debug_assert_eq!(frame.taken, 0);
    board.rollback_to_impl::<false>(checkpoint);
    board.dirty.fill(0);
    board.dirty_pairs = 0;
    board.choices.last_mut().unwrap().taken = 1;
    board.log(MARK, 0, 0);
    board.choose(c, zero, true);
    propagate_optional!(board, DIAG)
}

impl<C: CutState> Candidate<C> {
    fn new(puzzle: &Puzzle, direction: u8, options: &Options) -> Self {
        let mut board = Board::<C>::new(&puzzle.transformed(direction));
        if C::ENABLED {
            board.pruning.data_mut().interval = options.cut;
        }
        let memo =
            (options.memo_bits != 0).then(|| Memo::new(options.memo_bits, board.shared.height));
        Self {
            board,
            memo,
            out: Outcome {
                direction,
                threads: 1,
                ..Outcome::default()
            },
            good: false,
            started: false,
            finished: false,
            backtracking: false,
        }
    }
    fn report(&self) -> CandidateReport {
        // In the saved sibling phase, the first subtree is exhausted and its
        // consumed suffix has already been popped. Credit that half before the
        // next choose() step flips the frame's stored alternative index.
        let last = self.board.choices.len().wrapping_sub(1);
        let progress = progress(
            self.board.choices.iter().enumerate().map(|(i, f)| {
                if self.backtracking && i == last {
                    1
                } else {
                    f.taken
                }
            }),
            self.finished,
        );
        CandidateReport {
            direction: self.out.direction,
            steps: self.board.steps,
            progress,
            estimated_total: if self.finished {
                self.board.steps as f64
            } else if progress > 0.0 {
                self.board.steps as f64 / progress
            } else {
                f64::INFINITY
            },
            finished: self.finished,
        }
    }
    fn run_budget<const DIAG: bool, const MEMO: bool>(
        self,
        budget: u64,
        original: &Puzzle,
    ) -> Self {
        self.run_budget_control::<DIAG, MEMO>(budget, original, None)
    }
    fn run_budget_control<const DIAG: bool, const MEMO: bool>(
        self,
        budget: u64,
        original: &Puzzle,
        stop: Option<&AtomicBool>,
    ) -> Self {
        if self.finished || (self.backtracking && self.board.steps >= budget) {
            return self;
        }
        let Self {
            mut board,
            mut out,
            mut memo,
            good: saved_good,
            started,
            backtracking,
            ..
        } = self;
        search_loop!(
            board,
            out,
            DIAG,
            good,
            original,
            {
                good = saved_good;
                if backtracking {
                    good = resume_alternative::<C, DIAG>(&mut board);
                }
                if !started {
                    for y in 1..=board.shared.height {
                        for x in 1..=board.shared.width {
                            let c = y * board.shared.stride + x;
                            if active(board.state(c)) {
                                board.endpoint(c);
                            }
                        }
                    }
                    good = propagate_optional!(board, DIAG);
                }
            },
            {},
            {},
            {
                if MEMO && memo.as_mut().unwrap().observe(&board, out.solutions, good) {
                    board.fail();
                    good = false;
                }
            },
            {},
            {},
            {
                // Let an exhausted final branch finish its bookkeeping without
                // charging an extra step. Otherwise pause before the next branch.
                if stop.is_some_and(|s| s.load(Ordering::Relaxed))
                    || (board.steps >= budget
                        && ((good && board.remaining != 0)
                            || board.choices.iter().any(|f| f.taken == 0)))
                {
                    return Self {
                        board,
                        out,
                        memo,
                        good,
                        started: true,
                        finished: false,
                        backtracking: false,
                    };
                }
            },
            {
                if MEMO {
                    memo.as_mut().unwrap().finish(out.solutions);
                }
                if let Some(stop) = stop {
                    stop.store(true, Ordering::Relaxed);
                }
                return Self {
                    board,
                    out,
                    memo,
                    good,
                    started: true,
                    finished: true,
                    backtracking: false,
                };
            },
            {
                if board.steps >= budget && board.choices.last().unwrap().taken == 0 {
                    return Self {
                        board,
                        out,
                        memo,
                        good,
                        started: true,
                        finished: false,
                        backtracking: true,
                    };
                }
            }
        )
    }
    pub fn into_outcome(mut self) -> Outcome {
        self.out.steps = self.board.steps;
        self.out.contradictions = self.board.contradictions;
        if let Some(memo) = self.memo {
            self.out.memo_hits = memo.hits;
            self.out.memo_stores = memo.stores;
            self.out.memo_slots = memo.slots();
        }
        #[cfg(feature = "stats")]
        {
            self.out.stats = self.board.stats;
        }
        self.out
    }
}

fn select(reports: &[CandidateReport], recommended: u8) -> usize {
    reports
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            b.finished
                .cmp(&a.finished)
                .then_with(|| a.estimated_total.total_cmp(&b.estimated_total))
                .then_with(|| (a.direction != recommended).cmp(&(b.direction != recommended)))
                .then_with(|| a.direction.cmp(&b.direction))
        })
        .unwrap()
        .0
}

// B is cumulative per candidate. Extend only when every remaining-work
// estimate exceeds B/f (f=.02), keeping the budget below 2% of remaining work.
fn extend(reports: &[CandidateReport], budget: u64) -> bool {
    budget < 10_000_000
        && !reports.iter().any(|r| r.finished)
        && reports
            .iter()
            .all(|r| r.estimated_total - r.steps as f64 > budget as f64 / 0.02)
}

fn run<C: CutState + Send, const DIAG: bool>(
    puzzle: &Puzzle,
    requested: u64,
    options: &Options,
    forced: Option<u8>,
) -> RaceOutcome {
    let start = Instant::now();
    let mut budget = if requested == 1 { 10_000 } else { requested };
    // Lazy creation bounds concurrent allocations, while paused candidates keep
    // their own tables until selection (at most eight tables in total).
    let mut candidates: Vec<Option<Candidate<C>>> = (0..8).map(|_| None).collect();
    loop {
        let next = AtomicUsize::new(0);
        let stop = AtomicBool::new(false);
        let slots: Vec<_> = candidates
            .into_iter()
            .enumerate()
            .map(|(d, c)| std::sync::Mutex::new((d as u8, c)))
            .collect();
        std::thread::scope(|scope| {
            for _ in 0..options.threads.min(8) {
                let slots = &slots;
                let next = &next;
                let stop = &stop;
                scope.spawn(move || loop {
                    if forced.is_none() && stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= 8 {
                        break;
                    }
                    let mut slot = slots[index].lock().unwrap();
                    let c = slot
                        .1
                        .take()
                        .unwrap_or_else(|| Candidate::new(puzzle, slot.0, options));
                    slot.1 = Some(if options.memo_bits == 0 {
                        c.run_budget_control::<DIAG, false>(
                            budget,
                            puzzle,
                            forced.is_none().then_some(stop),
                        )
                    } else {
                        c.run_budget_control::<DIAG, true>(
                            budget,
                            puzzle,
                            forced.is_none().then_some(stop),
                        )
                    });
                });
            }
        });
        candidates = slots
            .into_iter()
            .map(|s| s.into_inner().unwrap().1)
            .collect();
        let reports: Vec<_> = candidates
            .iter()
            .enumerate()
            .map(|(d, c)| {
                c.as_ref().map_or(
                    CandidateReport {
                        direction: d as u8,
                        steps: 0,
                        progress: 0.0,
                        estimated_total: f64::INFINITY,
                        finished: false,
                    },
                    Candidate::report,
                )
            })
            .collect();
        if requested != 1 || !extend(&reports, budget) {
            break;
        }
        budget *= 10;
    }
    let reports: Vec<_> = candidates
        .iter()
        .enumerate()
        .map(|(d, c)| {
            c.as_ref().map_or(
                CandidateReport {
                    direction: d as u8,
                    steps: 0,
                    progress: 0.0,
                    estimated_total: f64::INFINITY,
                    finished: false,
                },
                Candidate::report,
            )
        })
        .collect();
    let chosen = forced.map_or_else(
        || select(&reports, puzzle.recommended_direction()),
        usize::from,
    );
    let winner = candidates[chosen].take().unwrap();
    drop(candidates); // Release every losing table before starting stealing.
    let race_seconds = start.elapsed().as_secs_f64();
    let continuation = Instant::now();
    let mut outcome = if winner.finished {
        winner.into_outcome()
    } else if options.threads > 1 {
        parallel::resume::<C, DIAG>(winner, puzzle, options)
    } else if options.memo_bits == 0 {
        winner
            .run_budget::<DIAG, false>(u64::MAX, puzzle)
            .into_outcome()
    } else {
        winner
            .run_budget::<DIAG, true>(u64::MAX, puzzle)
            .into_outcome()
    };
    outcome.threads = options.threads;
    RaceOutcome {
        race_steps: reports.iter().map(|r| r.steps).sum(),
        candidates: reports,
        budget,
        outcome,
        race_seconds,
        continuation_seconds: continuation.elapsed().as_secs_f64(),
        total_seconds: start.elapsed().as_secs_f64(),
    }
}

/// Race all eight directions, preserving the winner's search and memo state.
/// Budget 1 adapts from 10^4 through 10^7; larger budgets are fixed step caps.
/// `outcome.steps` includes the winner's race steps; `race_steps` includes all eight.
pub fn solve_race(puzzle: &Puzzle, budget: u64, options: &Options) -> RaceOutcome {
    assert!(budget >= 1, "race budget must be positive");
    assert!(options.memo_bits <= 28);
    assert!((1..=256).contains(&options.threads));
    assert!((1..=64).contains(&puzzle.width) && (1..=64).contains(&puzzle.height));
    assert!((1..=254).contains(&puzzle.pairs));
    assert_eq!(puzzle.cells.len(), puzzle.width * puzzle.height);
    if puzzle.pairs > 63 {
        return match (options.cut != 0, options.diag_shortcut) {
            (false, false) => run::<wide::Many<NoCut>, false>(puzzle, budget, options, None),
            (false, true) => run::<wide::Many<NoCut>, true>(puzzle, budget, options, None),
            (true, false) => run::<wide::Many<CutCapacity>, false>(puzzle, budget, options, None),
            (true, true) => run::<wide::Many<CutCapacity>, true>(puzzle, budget, options, None),
        };
    }
    match (options.cut != 0, options.diag_shortcut) {
        (false, false) => run::<NoCut, false>(puzzle, budget, options, None),
        (false, true) => run::<NoCut, true>(puzzle, budget, options, None),
        (true, false) => run::<CutCapacity, false>(puzzle, budget, options, None),
        (true, true) => run::<CutCapacity, true>(puzzle, budget, options, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometric_progress_and_selection() {
        assert_eq!(progress([].into_iter(), false), 0.0);
        assert_eq!(progress([0, 1, 1].into_iter(), false), 0.375);
        assert_eq!(progress([1, 1, 1].into_iter(), false), 0.875);
        assert_eq!(progress([].into_iter(), true), 1.0);
        assert_eq!(progress([1; 1100].into_iter(), false), 1.0);
        let mut reports: Vec<_> = (0..8)
            .map(|direction| CandidateReport {
                direction,
                steps: 10_000,
                progress: 0.0,
                estimated_total: f64::INFINITY,
                finished: false,
            })
            .collect();
        assert_eq!(select(&reports, 6), 6);
        assert!(extend(&reports, 10_000));
        assert!(!extend(&reports, 10_000_000));
        reports[3].estimated_total = 510_000.0;
        assert_eq!(select(&reports, 6), 3);
        assert!(!extend(&reports, 10_000));
        reports[3].estimated_total += 1.0;
        assert!(extend(&reports, 10_000));
        reports[2].finished = true;
        assert_eq!(select(&reports, 6), 2);
        assert!(!extend(&reports, 10_000));
        reports[1].finished = true;
        assert_eq!(select(&reports, 6), 1);
        assert_eq!(select(&reports, 2), 2);
    }

    #[test]
    fn ex3_forced_winners_and_saved_memo() {
        let p = Puzzle::parse(include_str!("../reference/ex3.txt")).unwrap();
        for memo_bits in [0, 16] {
            let options = Options {
                cut: 25,
                memo_bits,
                ..Options::none()
            };
            for direction in 0..8 {
                let serial = solve_with(&p, direction, &options);
                for threads in [1, 4] {
                    let result = run::<CutCapacity, false>(
                        &p,
                        10_000,
                        &Options { threads, ..options },
                        Some(direction),
                    );
                    assert_eq!(result.outcome.direction, direction);
                    assert_eq!(result.outcome.solutions, serial.solutions);
                    if threads == 1 || memo_bits == 0 {
                        assert_eq!(result.outcome.steps, serial.steps);
                    }
                    if threads == 1 {
                        assert_eq!(result.outcome.memo_hits, serial.memo_hits);
                        assert_eq!(result.outcome.memo_stores, serial.memo_stores);
                        assert_eq!(result.outcome.contradictions, serial.contradictions);
                        assert_eq!(result.outcome.first_solution, serial.first_solution);
                    }
                }
                let mut c = Candidate::<CutCapacity>::new(&p, direction, &options);
                let mut previous = 0.0;
                for budget in [1, 2, 10, 100, 1000, 10_000, 100_000, 1_000_000] {
                    c = if memo_bits == 0 {
                        c.run_budget::<false, false>(budget, &p)
                    } else {
                        c.run_budget::<false, true>(budget, &p)
                    };
                    assert!(c.board.steps <= budget);
                    let report = c.report();
                    assert!(report.progress >= previous && report.progress <= 1.0);
                    previous = report.progress;
                }
                assert!(c.finished);
                assert_eq!(c.report().progress, 1.0);
                let out = c.into_outcome();
                assert_eq!(out.steps, serial.steps);
                assert_eq!(out.solutions, serial.solutions);
                assert_eq!(out.memo_hits, serial.memo_hits);
                assert_eq!(out.memo_stores, serial.memo_stores);
            }
        }
    }

    #[test]
    fn progress_at_every_cut_step_is_monotone() {
        let p = Puzzle::parse(include_str!("../reference/ex3.txt")).unwrap();
        let options = Options {
            cut: 1,
            ..Options::none()
        };
        let mut c = Candidate::<CutCapacity>::new(&p, 0, &options);
        let mut previous = 0.0;
        loop {
            let budget = c.board.steps + 1;
            c = c.run_budget::<false, false>(budget, &p);
            let current = c.report().progress;
            assert!(
                current >= previous,
                "budget {budget}: {previous} -> {current}; sibling phase {}",
                c.backtracking
            );
            previous = current;
            if c.finished {
                break;
            }
        }
    }

    #[test]
    fn solutions_found_before_pause_survive() {
        let p = Puzzle::parse(include_str!(
            "../tests/fixtures/end_check_across_interior.txt"
        ))
        .unwrap();
        for memo_bits in [0, 12] {
            let options = Options {
                memo_bits,
                ..Options::none()
            };
            for direction in 0..8 {
                let expected = solve_with(&p, direction, &options);
                let mut c = Candidate::<NoCut>::new(&p, direction, &options);
                loop {
                    let next = c.board.steps + 1;
                    c = if memo_bits == 0 {
                        c.run_budget::<false, false>(next, &p)
                    } else {
                        c.run_budget::<false, true>(next, &p)
                    };
                    assert!(!c.finished, "fixture must pause after its first solution");
                    if c.out.solutions > 0 {
                        break;
                    }
                }
                assert!(expected.solutions > 1);
                let result = parallel::resume::<NoCut, false>(
                    c,
                    &p,
                    &Options {
                        threads: 4,
                        ..options
                    },
                );
                assert_eq!(result.solutions, expected.solutions);
                if memo_bits == 0 {
                    assert_eq!(result.steps, expected.steps);
                }
                assert_eq!(result.first_solution, expected.first_solution);
            }
        }
    }
}
