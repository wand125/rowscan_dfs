//! Cooperative donation: only a board's owner reads or clones its mutable state.
use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Condvar, Mutex,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
struct Policy {
    global: bool,
    rank: usize,
    minimum: usize,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            global: false,
            rank: 0,
            minimum: 2,
        }
    }
}

struct Job<C: CutState> {
    board: Board<C>,
    // Ancestor frames are retained for undo data, but cannot be searched.
    floor: usize,
    root: bool,
}
struct State<C: CutState> {
    busy: Vec<bool>,
    inbox: Vec<Option<Job<C>>>,
    requester: Vec<Option<usize>>,
    active: usize,
    cancelled: bool,
}
struct Scheduler<C: CutState> {
    state: Mutex<State<C>>,
    wake: Condvar,
    flags: Vec<AtomicBool>,
    demand: AtomicUsize,
    policy: Policy,
    first: Mutex<Option<(bool, Vec<u8>)>>,
    log: bool,
}

impl<C: CutState> Scheduler<C> {
    fn new(board: Board<C>, count: usize, policy: Policy, log: bool) -> Self {
        let mut inbox: Vec<_> = (0..count).map(|_| None).collect();
        inbox[0] = Some(Job {
            board,
            floor: 0,
            root: true,
        });
        let mut busy = vec![false; count];
        busy[0] = true;
        Self {
            state: Mutex::new(State {
                busy,
                inbox,
                requester: vec![None; count],
                active: 1,
                cancelled: false,
            }),
            wake: Condvar::new(),
            flags: (0..count).map(|_| AtomicBool::new(false)).collect(),
            demand: AtomicUsize::new(count - 1),
            policy,
            first: Mutex::new(None),
            log,
        }
    }
    fn next(&self, worker: usize, finished: bool) -> Option<Job<C>> {
        let mut state = self.state.lock().unwrap();
        if finished {
            state.busy[worker] = false;
            state.active -= 1;
            self.demand.fetch_add(1, Ordering::Relaxed);
            state.requester[worker] = None;
            self.flags[worker].store(false, Ordering::Relaxed);
            self.wake.notify_all();
        }
        let mut victim = worker;
        loop {
            debug_assert_eq!(
                state.active,
                state.busy.iter().filter(|&&busy| busy).count()
            );
            debug_assert!(state
                .requester
                .iter()
                .flatten()
                .all(|&thief| !state.busy[thief]));
            if state.cancelled {
                return None;
            }
            if let Some(job) = state.inbox[worker].take() {
                return Some(job);
            }
            if state.active == 0 {
                self.wake.notify_all();
                return None;
            }
            self.post_request(&mut state, worker, &mut victim);
            state = self
                .wake
                .wait_timeout(state, Duration::from_millis(1))
                .unwrap()
                .0;
        }
    }
    fn post_request(&self, state: &mut State<C>, worker: usize, victim: &mut usize) {
        // A reserved recipient is waiting for its inbox, not for another victim.
        if state.busy[worker] || self.policy.global || state.requester.contains(&Some(worker)) {
            return;
        }
        for _ in 0..state.busy.len() {
            *victim = (*victim + 1) % state.busy.len();
            if state.busy[*victim] && state.requester[*victim].is_none() {
                state.requester[*victim] = Some(worker);
                self.flags[*victim].store(true, Ordering::Relaxed);
                break;
            }
        }
    }
    fn requested(&self, worker: usize) -> bool {
        if self.policy.global {
            self.demand.load(Ordering::Relaxed) != 0
        } else {
            self.flags[worker].load(Ordering::Relaxed)
        }
    }
    fn recipient(&self, worker: usize, eligible: bool) -> Option<usize> {
        let mut state = self.state.lock().unwrap();
        if state.cancelled {
            return None;
        }
        let thief = if self.policy.global {
            if !eligible {
                return None;
            }
            state.busy.iter().position(|&busy| !busy)
        } else {
            self.flags[worker].store(false, Ordering::Relaxed);
            state.requester[worker].take()
        };
        if !eligible {
            self.wake.notify_all();
            return None;
        }
        let thief = thief?;
        debug_assert!(!state.busy[thief]);
        // Count an in-flight clone as active, before releasing the lock.
        state.busy[thief] = true;
        state.active += 1;
        self.demand.fetch_sub(1, Ordering::Relaxed);
        Some(thief)
    }
    fn deliver(&self, worker: usize, job: Job<C>) {
        let mut state = self.state.lock().unwrap();
        debug_assert!(state.busy[worker] && state.inbox[worker].is_none());
        state.inbox[worker] = Some(job);
        self.wake.notify_all();
    }
    fn publish(&self, out: &Outcome) {
        let mut first = self.first.lock().unwrap();
        if first.is_none() {
            *first = Some((
                out.first_solution_has_empty.unwrap(),
                out.first_solution.as_ref().unwrap().clone(),
            ));
        }
    }
    fn cancel(&self) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancelled = true;
        self.wake.notify_all();
    }
}

// Keep the non-race worker's sole call site and code-generation boundary.
// Race thieves use the same body under a separate specialization.
macro_rules! worker_search {
    ($name:ident) => {
    fn $name<const DIAG: bool, const MEMO: bool>(
        mut self,
        direction: u8,
        original: &Puzzle,
        resume: (usize, bool),
        mut memo: Option<Memo>,
        scheduler: &Scheduler<C>,
        worker: usize,
    ) -> Outcome {
        let (floor, root) = resume;
        let mut out = Outcome {
            direction,
            ..Outcome::default()
        };
        search_loop!(
            self,
            out,
            DIAG,
            good,
            original,
            {
                if root {
                    for y in 1..=self.shared.height {
                        for x in 1..=self.shared.width {
                            let c = y * self.shared.stride + x;
                            if active(self.state(c)) {
                                self.endpoint(c);
                            }
                        }
                    }
                }
                good = propagate_optional!(self, DIAG);
            },
            {
                if self.choices.len() == floor {
                    self.choices.clear();
                }
            },
            {
                scheduler.publish(&out);
            },
            {
                if scheduler.requested(worker) {
                    let frame = self.donation_frame(floor, scheduler.policy);
                    if let Some(thief) = scheduler.recipient(worker, frame.is_some()) {
                        let frame = frame.unwrap();
                        let (job, seconds) = self.donate(frame, memo.as_mut());
                        out.steals += 1;
                        out.clone_seconds += seconds;
                        if scheduler.log {
                            eprintln!("[steal] thief {thief} took from worker {worker} (frame {frame}, floor {})", job.floor);
                        }
                        scheduler.deliver(thief, job);
                    }
                }
                if MEMO && memo.as_mut().unwrap().observe(&self, out.solutions, good) {
                    self.fail();
                    good = false;
                }
            },
            {},
            {
                if MEMO {
                    let memo = memo.as_mut().unwrap();
                    memo.finish(out.solutions);
                    out.memo_hits = memo.hits;
                    out.memo_stores = memo.stores;
                    out.memo_slots = memo.slots();
                }
            }
        )
    }
    };
}

impl<C: CutState> Board<C> {
    fn donation_frame(&self, floor: usize, policy: Policy) -> Option<usize> {
        let mut pending = self
            .choices
            .iter()
            .enumerate()
            .skip(floor)
            .filter(|(_, frame)| frame.taken == 0);
        let frame = pending.nth(policy.rank)?.0;
        // Count all pending alternatives, including those before selected rank.
        (policy.rank + 1 + pending.count() >= policy.minimum).then_some(frame)
    }
    fn donate(&mut self, frame: usize, memo: Option<&mut Memo>) -> (Job<C>, f64) {
        let start = Instant::now();
        let mut copy = self.snapshot();
        let clone_seconds = start.elapsed().as_secs_f64();
        let checkpoint = copy.choices[frame].trail_len;
        copy.rollback_to_impl::<false>(checkpoint);
        copy.choices.truncate(frame + 1);
        copy.steps = 0;
        copy.contradictions = 0;
        #[cfg(feature = "stats")]
        {
            copy.stats = Stats::default();
        }
        copy.dirty.fill(0);
        copy.dirty_pairs = 0;
        copy.choices[frame].taken = 1;
        let (c, zero) = (copy.choices[frame].cell as usize, copy.choices[frame].zero);
        copy.log(MARK, 0, 0);
        copy.choose(c, zero, true);
        self.choices[frame].taken = 1;
        if let Some(memo) = memo {
            memo.taint_open();
        }
        (
            Job {
                board: copy,
                floor: frame + 1,
                root: false,
            },
            clone_seconds,
        )
    }
    worker_search!(search_job);
    worker_search!(search_race_job);
    fn search_resumed<const DIAG: bool, const MEMO: bool>(
        mut self,
        original: &Puzzle,
        mut out: Outcome,
        saved_good: bool,
        mut memo: Option<Memo>,
        scheduler: &Scheduler<C>,
        worker: usize,
    ) -> Outcome {
        let floor = 0;
        search_loop!(
            self,
            out,
            DIAG,
            good,
            original,
            {
                good = saved_good;
            },
            {
                if self.choices.len() == floor {
                    self.choices.clear();
                }
            },
            {
                scheduler.publish(&out);
            },
            {
                if scheduler.requested(worker) {
                    let frame = self.donation_frame(floor, scheduler.policy);
                    if let Some(thief) = scheduler.recipient(worker, frame.is_some()) {
                        let frame = frame.unwrap();
                        let (job, seconds) = self.donate(frame, memo.as_mut());
                        out.steals += 1;
                        out.clone_seconds += seconds;
                        if scheduler.log {
                            eprintln!("[steal] thief {thief} took from worker {worker} (frame {frame}, floor {})", job.floor);
                        }
                        scheduler.deliver(thief, job);
                    }
                }
                if MEMO && memo.as_mut().unwrap().observe(&self, out.solutions, good) {
                    self.fail();
                    good = false;
                }
            },
            {},
            {
                if MEMO {
                    let memo = memo.as_mut().unwrap();
                    memo.finish(out.solutions);
                    out.memo_hits = memo.hits;
                    out.memo_stores = memo.stores;
                    out.memo_slots = memo.slots();
                }
            }
        )
    }
}

fn add(out: &mut Outcome, part: Outcome) {
    out.steps += part.steps;
    out.solutions += part.solutions;
    out.contradictions += part.contradictions;
    out.memo_hits += part.memo_hits;
    out.memo_stores += part.memo_stores;
    out.steals += part.steals;
    out.clone_seconds += part.clone_seconds;
    out.cpu += part.cpu;
    #[cfg(feature = "stats")]
    out.stats.merge(&part.stats);
}

pub(super) fn solve<C: CutState + Send, const DIAG: bool>(
    board: Board<C>,
    direction: u8,
    original: &Puzzle,
    options: &Options,
) -> Outcome {
    // Overrides are for the reproducible design comparison, never read in a hot loop.
    // Lowest two-board geometric mean in the retained x86 design comparison.
    let mut policy = Policy {
        global: true,
        ..Policy::default()
    };
    if let Ok(value) = std::env::var("ROWSCAN_DFS_STEAL_POLICY") {
        match value.as_str() {
            "global" => policy.global = true,
            "victim" => policy.global = false,
            "second" => {
                policy.global = false;
                policy.rank = 1;
            }
            "min3" => {
                policy.global = false;
                policy.minimum = 3;
            }
            "default" => {}
            _ => panic!("invalid ROWSCAN_DFS_STEAL_POLICY"),
        }
    }
    solve_policy::<C, DIAG>(board, direction, original, options, policy)
}
fn solve_policy<C: CutState + Send, const DIAG: bool>(
    board: Board<C>,
    direction: u8,
    original: &Puzzle,
    options: &Options,
    policy: Policy,
) -> Outcome {
    let height = board.shared.height;
    let table = (options.memo_bits != 0).then(|| Memo::new(options.memo_bits, height));
    let scheduler = Scheduler::new(board, options.threads, policy, options.log);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..options.threads)
            .map(|worker| {
                let scheduler = &scheduler;
                let table = &table;
                scope.spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut total = Outcome::default();
                        let mut finished = false;
                        while let Some(job) = scheduler.next(worker, finished) {
                            let start = Instant::now();
                            let memo = table.as_ref().map(|memo| memo.fork(height));
                            let mut part = if options.memo_bits == 0 {
                                job.board.search_job::<DIAG, false>(
                                    direction,
                                    original,
                                    (job.floor, job.root),
                                    memo,
                                    scheduler,
                                    worker,
                                )
                            } else {
                                job.board.search_job::<DIAG, true>(
                                    direction,
                                    original,
                                    (job.floor, job.root),
                                    memo,
                                    scheduler,
                                    worker,
                                )
                            };
                            part.cpu = start.elapsed().as_secs_f64();
                            add(&mut total, part);
                            finished = true;
                        }
                        if scheduler.log {
                            eprintln!(
                                "[done] worker {worker} steps {} solutions {} elapsed {:.6}",
                                total.steps, total.solutions, total.cpu
                            );
                        }
                        total
                    }));
                    match result {
                        Ok(out) => out,
                        Err(panic) => {
                            scheduler.cancel();
                            std::panic::resume_unwind(panic)
                        }
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    let mut out = Outcome {
        direction,
        threads: options.threads,
        memo_slots: table.as_ref().map_or(0, Memo::slots),
        ..Outcome::default()
    };
    for part in results {
        add(&mut out, part);
    }
    if let Some((empty, edges)) = scheduler.first.into_inner().unwrap() {
        out.first_solution_has_empty = Some(empty);
        out.first_solution = Some(edges);
    }
    out
}

pub(super) fn resume<C: CutState + Send, const DIAG: bool>(
    winner: race::Candidate<C>,
    original: &Puzzle,
    options: &Options,
) -> Outcome {
    let race::Candidate {
        mut board,
        out: initial,
        memo,
        mut good,
        backtracking,
        ..
    } = winner;
    if backtracking {
        good = race::resume_alternative::<C, DIAG>(&mut board);
    }
    let direction = initial.direction;
    let height = board.shared.height;
    let table = memo.as_ref().map(|m| m.fork(height));
    let root = Mutex::new(Some((initial, memo, good)));
    let scheduler = Scheduler::new(
        board,
        options.threads,
        Policy {
            global: true,
            ..Policy::default()
        },
        options.log,
    );
    {
        let state = root.lock().unwrap();
        if state.as_ref().unwrap().0.first_solution.is_some() {
            scheduler.publish(&state.as_ref().unwrap().0);
        }
    }
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..options.threads)
            .map(|worker| {
                let scheduler = &scheduler;
                let table = &table;
                let root = &root;
                scope.spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut total = Outcome::default();
                        let mut finished = false;
                        while let Some(job) = scheduler.next(worker, finished) {
                            let start = Instant::now();
                            let mut part = if job.root {
                                let (out, memo, good) = root.lock().unwrap().take().unwrap();
                                if options.memo_bits == 0 {
                                    job.board.search_resumed::<DIAG, false>(
                                        original, out, good, memo, scheduler, worker,
                                    )
                                } else {
                                    job.board.search_resumed::<DIAG, true>(
                                        original, out, good, memo, scheduler, worker,
                                    )
                                }
                            } else {
                                let memo = table.as_ref().map(|m| m.fork(height));
                                if options.memo_bits == 0 {
                                    job.board.search_race_job::<DIAG, false>(
                                        direction,
                                        original,
                                        (job.floor, false),
                                        memo,
                                        scheduler,
                                        worker,
                                    )
                                } else {
                                    job.board.search_race_job::<DIAG, true>(
                                        direction,
                                        original,
                                        (job.floor, false),
                                        memo,
                                        scheduler,
                                        worker,
                                    )
                                }
                            };
                            part.cpu = start.elapsed().as_secs_f64();
                            add(&mut total, part);
                            finished = true;
                        }
                        if scheduler.log {
                            eprintln!(
                                "[done] worker {worker} steps {} solutions {} elapsed {:.6}",
                                total.steps, total.solutions, total.cpu
                            );
                        }
                        total
                    }));
                    match result {
                        Ok(out) => out,
                        Err(panic) => {
                            scheduler.cancel();
                            std::panic::resume_unwind(panic)
                        }
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    let mut out = Outcome {
        direction,
        threads: options.threads,
        memo_slots: table.as_ref().map_or(0, Memo::slots),
        ..Outcome::default()
    };
    for part in results {
        add(&mut out, part);
    }
    if let Some((empty, edges)) = scheduler.first.into_inner().unwrap() {
        out.first_solution_has_empty = Some(empty);
        out.first_solution = Some(edges);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pending_board() -> (Puzzle, Board) {
        let p = Puzzle::parse(include_str!("../reference/ex3.txt")).unwrap();
        let mut b = Board::<NoCut>::new(&p);
        for y in 1..=b.shared.height {
            for x in 1..=b.shared.width {
                let c = y * b.shared.stride + x;
                if active(b.state(c)) {
                    b.endpoint(c);
                }
            }
        }
        let mut good = b.propagate();
        for _ in 0..10000 {
            if good && b.choices.iter().filter(|f| f.taken == 0).count() >= 3 {
                b.steps = 0;
                b.contradictions = 0;
                #[cfg(feature = "stats")]
                {
                    b.stats = Stats::default();
                }
                return (p, b);
            }
            if good && b.remaining != 0 {
                if let Some((c, zero)) = b.next_cell() {
                    let trail_len = b.trail.len();
                    b.log(MARK, 0, 0);
                    b.choices.push(Choice {
                        trail_len,
                        cell: c as u16,
                        zero,
                        taken: 0,
                    });
                    b.choose(c, zero, false);
                    good = b.propagate();
                    continue;
                }
            }
            loop {
                let frame = b.choices.last().expect("fixture has a pending branch");
                let (checkpoint, c, zero, taken) = (
                    frame.trail_len,
                    frame.cell as usize,
                    frame.zero,
                    frame.taken,
                );
                b.rollback_to_impl::<false>(checkpoint);
                if taken == 0 {
                    b.dirty.fill(0);
                    b.dirty_pairs = 0;
                    b.choices.last_mut().unwrap().taken = 1;
                    b.log(MARK, 0, 0);
                    b.choose(c, zero, true);
                    good = b.propagate();
                    break;
                }
                b.choices.pop();
            }
        }
        panic!("no donation point");
    }
    #[test]
    fn donation_partitions_remaining_tree_and_taints_ancestors() {
        for policy in [
            Policy::default(),
            Policy {
                rank: 1,
                ..Policy::default()
            },
            Policy {
                minimum: 3,
                ..Policy::default()
            },
        ] {
            let (p, mut b) = pending_board();
            let original = b.snapshot();
            let scheduler = Scheduler::new(b.snapshot(), 1, policy, false);
            let frame = b.donation_frame(0, policy).unwrap();
            assert!(b.donation_frame(b.choices.len(), policy).is_none());
            let mut memo = Memo::new(8, b.shared.height);
            assert!(!memo.observe(&b, 0, true));
            let (job, _) = b.donate(frame, Some(&mut memo));
            assert!(Arc::ptr_eq(&b.shared, &job.board.shared));
            assert_eq!(job.floor, frame + 1);
            assert_eq!(b.choices[frame].taken, 1);
            assert_eq!(job.board.choices[frame].taken, 1);
            assert!(original.choices[frame].taken == 0);
            assert_eq!(job.board.choices.len(), job.floor);
            memo.finish(0);
            assert_eq!(
                memo.stores, 0,
                "the donor must never cache its partial ancestors"
            );
            // Deliberately retain untried ancestors in the thief. Searching any
            // ancestor again violates both the solution and exact step partition.
            let expected =
                original.search_race_job::<false, false>(0, &p, (0, false), None, &scheduler, 0);
            let owner = b.search_race_job::<false, false>(0, &p, (0, false), None, &scheduler, 0);
            let thief = job.board.search_race_job::<false, false>(
                0,
                &p,
                (job.floor, false),
                None,
                &scheduler,
                0,
            );
            assert_eq!(owner.steps + thief.steps, expected.steps);
            assert_eq!(owner.solutions + thief.solutions, expected.solutions);
        }
    }
    #[test]
    fn reserved_recipient_cannot_request_twice() {
        let (_, b) = pending_board();
        let s = Scheduler::new(b, 2, Policy::default(), false);
        let mut state = s.state.lock().unwrap();
        s.post_request(&mut state, 1, &mut 1);
        assert_eq!(state.requester[0], Some(1));
        drop(state);
        assert_eq!(s.recipient(0, true), Some(1));
        let mut state = s.state.lock().unwrap();
        s.post_request(&mut state, 1, &mut 1);
        assert!(state.requester.iter().all(Option::is_none));
        assert_eq!(state.active, 2);
        assert!(state.inbox[1].is_none(), "donation is still in flight");
    }
    #[test]
    fn every_policy_and_inflight_termination() {
        let (p, b) = pending_board();
        let s = Scheduler::new(
            b,
            2,
            Policy {
                global: true,
                ..Policy::default()
            },
            false,
        );
        let _root = s.next(0, false).unwrap();
        assert_eq!(s.recipient(0, true), Some(1));
        // Worker 1's reserved donation remains active even after worker 0 ends.
        {
            let state = s.state.lock().unwrap();
            assert_eq!(state.active, 2);
            assert!(state.busy[1]);
        }
        s.cancel();
        assert!(s.next(0, true).is_none());
        assert_eq!(s.state.lock().unwrap().active, 1);
        let serial = super::super::solve_none(&p, 0);
        for policy in [
            Policy::default(),
            Policy {
                global: true,
                ..Policy::default()
            },
            Policy {
                rank: 1,
                ..Policy::default()
            },
            Policy {
                minimum: 3,
                ..Policy::default()
            },
        ] {
            let result = solve_policy::<NoCut, false>(
                Board::new(&p),
                0,
                &p,
                &Options {
                    threads: 4,
                    ..Options::none()
                },
                policy,
            );
            assert_eq!(result.solutions, serial.solutions);
            assert_eq!(result.steps, serial.steps);
            assert!(result.steals > 0);
        }
    }
}
