//! Browser-only continuation, with separate native solve and race loops.
use super::*;

impl<C: CutState> Candidate<C> {
    fn run_browser<const DIAG: bool, const MEMO: bool>(
        self,
        budget: u64,
        original: &Puzzle,
        max: u64,
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
                if board.steps >= budget
                    && ((good && board.remaining != 0)
                        || board.choices.iter().any(|f| f.taken == 0))
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
                if max != 0 && out.solutions >= max {
                    // This is a capped result, not a failed/exhausted subtree.
                    // Do not finish or store open memo entries.
                    return Self {
                        board,
                        out,
                        memo,
                        good,
                        started: true,
                        finished: true,
                        backtracking: false,
                    };
                }
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
}

enum State {
    Plain(Candidate<NoCut>),
    Cut(Candidate<CutCapacity>),
    WidePlain(Candidate<wide::Many<NoCut>>),
    WideCut(Candidate<wide::Many<CutCapacity>>),
}

/// A single browser worker's saved board, undo stack, memo frontiers and counts.
pub struct BrowserSearch {
    puzzle: Puzzle,
    options: Options,
    state: Option<State>,
    elapsed: f64,
}

impl BrowserSearch {
    pub fn new(puzzle: Puzzle, direction: u8, options: Options) -> Self {
        assert!(direction < 8 && options.memo_bits <= 28 && options.threads == 1);
        let start = Instant::now();
        let state = match (options.cut != 0, puzzle.pairs > 63) {
            (false, false) => State::Plain(Candidate::new(&puzzle, direction, &options)),
            (true, false) => State::Cut(Candidate::new(&puzzle, direction, &options)),
            (false, true) => State::WidePlain(Candidate::new(&puzzle, direction, &options)),
            (true, true) => State::WideCut(Candidate::new(&puzzle, direction, &options)),
        };
        Self {
            puzzle,
            options,
            state: Some(state),
            elapsed: start.elapsed().as_secs_f64(),
        }
    }

    /// Run at most `additional` more branch steps; zero means exhaust.
    /// A positive max stops once that many solutions have been found.
    pub fn run_budget(&mut self, additional: u64, max: u64) {
        fn advance<C: CutState>(
            c: Candidate<C>,
            p: &Puzzle,
            o: &Options,
            additional: u64,
            max: u64,
        ) -> Candidate<C> {
            let budget = if additional == 0 {
                u64::MAX
            } else {
                c.board.steps.saturating_add(additional)
            };
            match (o.diag_shortcut, o.memo_bits != 0) {
                (false, false) => c.run_browser::<false, false>(budget, p, max),
                (false, true) => c.run_browser::<false, true>(budget, p, max),
                (true, false) => c.run_browser::<true, false>(budget, p, max),
                (true, true) => c.run_browser::<true, true>(budget, p, max),
            }
        }
        let start = Instant::now();
        self.state = Some(match self.state.take().unwrap() {
            State::Plain(c) => {
                State::Plain(advance(c, &self.puzzle, &self.options, additional, max))
            }
            State::Cut(c) => State::Cut(advance(c, &self.puzzle, &self.options, additional, max)),
            State::WidePlain(c) => {
                State::WidePlain(advance(c, &self.puzzle, &self.options, additional, max))
            }
            State::WideCut(c) => {
                State::WideCut(advance(c, &self.puzzle, &self.options, additional, max))
            }
        });
        self.elapsed += start.elapsed().as_secs_f64();
    }

    pub fn report(&self) -> (Outcome, CandidateReport, f64) {
        fn report<C: CutState>(c: &Candidate<C>) -> (Outcome, CandidateReport) {
            let out = Outcome {
                direction: c.out.direction,
                steps: c.board.steps,
                contradictions: c.board.contradictions,
                threads: 1,
                solutions: c.out.solutions,
                first_solution_has_empty: c.out.first_solution_has_empty,
                first_solution: c.out.first_solution.clone(),
                memo_hits: c.memo.as_ref().map_or(0, |m| m.hits),
                memo_stores: c.memo.as_ref().map_or(0, |m| m.stores),
                memo_slots: c.memo.as_ref().map_or(0, Memo::slots),
                ..Outcome::default()
            };
            (out, c.report())
        }
        let (out, progress) = match self.state.as_ref().unwrap() {
            State::Plain(c) => report(c),
            State::Cut(c) => report(c),
            State::WidePlain(c) => report(c),
            State::WideCut(c) => report(c),
        };
        (out, progress, self.elapsed * 1000.0)
    }
}
