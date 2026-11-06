//! Breadth-first splitting, isolated from the normal search loops.
use super::*;
use std::{collections::VecDeque, fmt::Write, time::Instant};

#[derive(Clone, Debug, PartialEq)]
pub struct Cube {
    pub edges: Vec<(usize, usize, bool)>,
    pub next: usize,
    pub weight: f64,
}

#[derive(Debug)]
pub struct CubeSplit {
    pub width: usize,
    pub height: usize,
    pub cubes: Vec<Cube>,
    pub solutions_while_splitting: u64,
    pub steps: u64,
    pub elapsed_ms: f64,
}
impl Cube {
    /// SAT fixed edges: (horizontal, row, column, line).
    pub fn to_fixed_edges(&self) -> Vec<(bool, usize, usize, bool)> {
        self.edges
            .iter()
            .map(|&(y, x, v)| {
                if y % 2 == 0 {
                    (true, y / 2, x / 2, v)
                } else {
                    (false, y / 2, x / 2, v)
                }
            })
            .collect()
    }
}

impl CubeSplit {
    /// Edge-grid coordinates and scan positions are always in direction 0.
    pub fn to_json(&self) -> String {
        let mut s = format!(
            "{{\"width\":{},\"height\":{},\"direction\":0,\"cubes\":[",
            self.width, self.height
        );
        for (i, c) in self.cubes.iter().enumerate() {
            if i != 0 {
                s.push(',');
            }
            s.push_str("{\"edges\":[");
            for (j, &(y, x, line)) in c.edges.iter().enumerate() {
                if j != 0 {
                    s.push(',');
                }
                write!(s, "[{y},{x},{line}]").unwrap();
            }
            write!(s, "],\"next\":{},\"weight\":{}}}", c.next, c.weight).unwrap();
        }
        writeln!(
            s,
            "],\"solutions_while_splitting\":{},\"steps\":{},\"elapsed_ms\":{}}}",
            self.solutions_while_splitting, self.steps, self.elapsed_ms
        )
        .unwrap();
        s
    }
}

struct Open<C: CutState> {
    board: Board<C>,
    cell: usize,
    zero: bool,
}
enum State<C: CutState> {
    Open(Box<Open<C>>),
    Solution,
    Dead,
}

fn classify<C: CutState>(mut board: Board<C>, diag: bool, steps: &mut u64) -> State<C> {
    if !board.consistent || !board.propagate() || (diag && !board.diagonal_shortcut_ok()) {
        return State::Dead;
    }
    if board.remaining == 0 {
        return if board.has_anonymous_lines() {
            State::Dead
        } else {
            State::Solution
        };
    }
    let before = board.steps;
    let next = board.next_cell();
    *steps += board.steps - before;
    match next {
        Some((cell, zero)) => State::Open(Box::new(Open { board, cell, zero })),
        None => State::Dead,
    }
}
fn children<C: CutState>(node: Open<C>, diag: bool, steps: &mut u64) -> Vec<State<C>> {
    [false, true]
        .into_iter()
        .map(|alternative| {
            let mut board = node.board.snapshot();
            // No ancestor is revisited by this splitter; discard its undo storage.
            board.trail.clear();
            board.choose(node.cell, node.zero, alternative);
            *steps += 1;
            classify(board, diag, steps)
        })
        .collect()
}
fn export<C: CutState>(node: &Open<C>) -> Cube {
    let b = &node.board;
    let (w, h, s) = (b.shared.width, b.shared.height, b.shared.stride);
    let mut edges = Vec::new();
    for y in 0..2 * h - 1 {
        for x in 0..2 * w - 1 {
            if y % 2 == x % 2 {
                continue;
            }
            let (cell, dir) = if y % 2 == 0 {
                ((y / 2 + 1) * s + (x - 1) / 2 + 1, R)
            } else {
                (((y - 1) / 2 + 1) * s + x / 2 + 1, D)
            };
            let e = b.edges(cell);
            if e & bit(dir) != 0 {
                edges.push((y, x, true));
            } else if e & (bit(dir) << 4) != 0 {
                edges.push((y, x, false));
            }
        }
    }
    Cube {
        edges,
        next: (node.cell / s - 1) * w + node.cell % s - 1,
        weight: 0.0,
    }
}
fn probe<C: CutState>(node: Open<C>, diag: bool, rng: &mut u64) -> f64 {
    let mut current = node;
    let mut product = 1.0;
    let mut estimate = 1.0;
    loop {
        let mut ignored = 0;
        let mut live: Vec<_> = children(current, diag, &mut ignored)
            .into_iter()
            .filter(|s| !matches!(s, State::Dead))
            .collect();
        if live.is_empty() {
            return estimate;
        }
        product = (product * live.len() as f64).min(f64::MAX);
        estimate = (estimate + product).min(f64::MAX);
        *rng ^= *rng << 13;
        *rng ^= *rng >> 7;
        *rng ^= *rng << 17;
        let i = *rng as usize % live.len();
        match live.swap_remove(i) {
            State::Open(n) => current = *n,
            _ => return estimate,
        }
    }
}
fn initialize<C: CutState>(p: &Puzzle, cube: Option<&Cube>, options: &Options) -> Board<C> {
    let mut board = Board::<C>::new(p);
    if C::ENABLED {
        board.pruning.data_mut().interval = if options.cut == 0 { 25 } else { options.cut };
    }
    // Match normal root initialization, including eager endpoint checks.
    for y in 1..=p.height {
        for x in 1..=p.width {
            let c = y * board.shared.stride + x;
            if active(board.state(c)) {
                board.endpoint(c);
            }
        }
    }
    if let Some(cube) = cube {
        if cube.next > p.width * p.height {
            board.fail();
            return board;
        }
        for &(y, x, line) in &cube.edges {
            if y >= 2 * p.height - 1 || x >= 2 * p.width - 1 || y % 2 == x % 2 {
                board.fail();
                return board;
            }
            let c = (y / 2 + 1) * board.shared.stride + x / 2 + 1;
            let d = if y % 2 == 0 { R } else { D };
            if line {
                board.set_line(c, d);
            } else {
                board.set_noline(c, d);
            }
        }
        // A resumable cube must decide both outgoing edges of every earlier cell.
        for at in 0..cube.next {
            let (y, x) = (at / p.width, at % p.width);
            let c = (y + 1) * board.shared.stride + x + 1;
            for (exists, d) in [(x + 1 < p.width, R), (y + 1 < p.height, D)] {
                if exists && board.edges(c) & (bit(d) | (bit(d) << 4)) == 0 {
                    board.fail();
                    return board;
                }
            }
        }
        board.cursor =
            ((cube.next / p.width + 1) * board.shared.stride + cube.next % p.width + 1) as u16;
    }
    board
}

fn frontier<C: CutState>(
    board: Board<C>,
    minimum: usize,
    options: &Options,
) -> (VecDeque<Box<Open<C>>>, u64, u64) {
    let mut queue = VecDeque::new();
    let mut found = 0;
    let mut steps = 0;
    let mut retain = |state, queue: &mut VecDeque<_>| match state {
        State::Open(n) => queue.push_back(n),
        State::Solution => found += 1,
        State::Dead => (),
    };
    retain(
        classify(board, options.diag_shortcut, &mut steps),
        &mut queue,
    );
    while queue.len() < minimum.max(1) {
        let Some(node) = queue.pop_front() else {
            break;
        };
        for state in children(*node, options.diag_shortcut, &mut steps) {
            retain(state, &mut queue);
        }
    }
    (queue, found, steps)
}

fn split<C: CutState>(
    p: &Puzzle,
    cube: &Cube,
    minimum: usize,
    options: &Options,
) -> (Vec<Cube>, u64) {
    let (queue, found, _) = frontier(initialize::<C>(p, Some(cube), options), minimum, options);
    (queue.iter().map(|node| export(node)).collect(), found)
}

/// Resume a direction-0 (or already transformed) puzzle's cube using the default cube rules.
pub fn split_cube(puzzle: &Puzzle, cube: &Cube, min_children: usize) -> (Vec<Cube>, u64) {
    split_cube_with_options(puzzle, cube, min_children, &Options::default())
}

pub fn split_cube_with_options(
    puzzle: &Puzzle,
    cube: &Cube,
    min_children: usize,
    options: &Options,
) -> (Vec<Cube>, u64) {
    if puzzle.pairs > 63 {
        split::<wide::Many<CutCapacity>>(puzzle, cube, min_children, options)
    } else {
        split::<CutCapacity>(puzzle, cube, min_children, options)
    }
}

fn estimate<C: CutState>(
    p: &Puzzle,
    cube: &Cube,
    probes: usize,
    seed: u64,
    options: &Options,
) -> f64 {
    let mut ignored = 0;
    match classify(
        initialize::<C>(p, Some(cube), options),
        options.diag_shortcut,
        &mut ignored,
    ) {
        State::Dead => 0.0,
        State::Solution => 1.0,
        State::Open(node) => {
            let mut rng = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407)
                .max(1);
            (0..probes.max(1)).fold(0.0, |mean, i| {
                let value = probe(
                    Open {
                        board: node.board.snapshot(),
                        cell: node.cell,
                        zero: node.zero,
                    },
                    options.diag_shortcut,
                    &mut rng,
                );
                mean + (value - mean) / (i + 1) as f64
            })
        }
    }
}

/// Knuth random-path tree estimate; inconsistent cubes return zero. Zero probes uses one dive.
pub fn estimate_cube_size(puzzle: &Puzzle, cube: &Cube, probes: usize, seed: u64) -> f64 {
    estimate_cube_size_with_options(puzzle, cube, probes, seed, &Options::default())
}

pub fn estimate_cube_size_with_options(
    puzzle: &Puzzle,
    cube: &Cube,
    probes: usize,
    seed: u64,
    options: &Options,
) -> f64 {
    if puzzle.pairs > 63 {
        estimate::<wide::Many<CutCapacity>>(puzzle, cube, probes, seed, options)
    } else {
        estimate::<CutCapacity>(puzzle, cube, probes, seed, options)
    }
}

/// Result of a bounded serial search within a direction-0 cube.
#[derive(Debug, PartialEq, Eq)]
pub struct CubeOutcome {
    /// True when the DFS exhausted the cube or reached `max_solutions`.
    pub finished: bool,
    /// Solutions found so far, capped at `max_solutions` for open cubes.
    pub solutions: u64,
    pub steps: u64,
}

/// Progress of a bounded serial search within a direction-0 cube.
#[derive(Debug)]
pub struct CubeProgress {
    pub steps: u64,
    pub progress: f64,
    pub estimated_total: f64,
    /// True only when the search exhausted the cube within its budget.
    pub finished: bool,
    pub solutions: u64,
}

fn solve_budget<C: CutState>(
    puzzle: &Puzzle,
    cube: &Cube,
    budget: u64,
    max_solutions: u64,
    options: &Options,
) -> CubeProgress {
    let mut steps = 0;
    match classify(
        initialize::<C>(puzzle, Some(cube), options),
        options.diag_shortcut,
        &mut steps,
    ) {
        State::Dead => CubeProgress {
            progress: 1.0,
            estimated_total: steps as f64,
            finished: true,
            solutions: 0,
            steps,
        },
        State::Solution => CubeProgress {
            progress: 1.0,
            estimated_total: steps as f64,
            finished: true,
            solutions: 1,
            steps,
        },
        State::Open(node) => {
            let board = node.board;
            let memo =
                (options.memo_bits != 0).then(|| Memo::new(options.memo_bits, board.shared.height));
            let candidate = race::Candidate {
                board,
                memo,
                out: Outcome {
                    threads: 1,
                    ..Outcome::default()
                },
                good: true,
                // initialize/classify already performed endpoint checks and propagation.
                started: true,
                finished: false,
                backtracking: false,
            }
            .run_cube_budget(budget, max_solutions, puzzle, options);
            let report = candidate.report();
            CubeProgress {
                progress: report.progress,
                estimated_total: report.estimated_total,
                finished: candidate.finished,
                solutions: candidate.out.solutions,
                steps: candidate.board.steps,
            }
        }
    }
}

/// Search a cube in identity coordinates, stopping at the step or solution limit.
/// Threads and logging are ignored; cut, diagonal shortcuts and memo are respected.
pub fn solve_cube_budget(
    puzzle: &Puzzle,
    cube: &Cube,
    budget: u64,
    max_solutions: u64,
    options: &Options,
) -> CubeOutcome {
    let result = cube_budget(puzzle, cube, budget, max_solutions, options);
    CubeOutcome {
        finished: result.finished,
        solutions: result.solutions,
        steps: result.steps,
    }
}

/// Estimate a cube from the binary fraction of alternatives consumed by a bounded DFS.
/// An untouched first branch can report zero progress and an infinite estimate.
/// Threads and logging are ignored; cut, diagonal shortcuts and memo are respected.
pub fn estimate_cube_progress(
    puzzle: &Puzzle,
    cube: &Cube,
    budget: u64,
    options: &Options,
) -> CubeProgress {
    cube_budget(puzzle, cube, budget, u64::MAX, options)
}

fn cube_budget(
    puzzle: &Puzzle,
    cube: &Cube,
    budget: u64,
    max_solutions: u64,
    options: &Options,
) -> CubeProgress {
    assert!(options.memo_bits <= 28, "memo bits must be 0..=28");
    if puzzle.pairs > 63 {
        if options.cut == 0 {
            solve_budget::<wide::Many<NoCut>>(puzzle, cube, budget, max_solutions, options)
        } else {
            solve_budget::<wide::Many<CutCapacity>>(puzzle, cube, budget, max_solutions, options)
        }
    } else if options.cut == 0 {
        solve_budget::<NoCut>(puzzle, cube, budget, max_solutions, options)
    } else {
        solve_budget::<CutCapacity>(puzzle, cube, budget, max_solutions, options)
    }
}

fn generate<C: CutState>(
    p: &Puzzle,
    minimum: usize,
    options: &Options,
    probes: usize,
) -> CubeSplit {
    let started = Instant::now();
    let board = initialize::<C>(p, None, options);
    let (queue, found, steps) = frontier(board, minimum, options);
    let mut rng = 0x6c69_6e6b_7363_616e;
    let cubes = queue
        .into_iter()
        .map(|node| {
            let mut c = export(&node);
            if probes != 0 {
                c.weight = (0..probes).fold(0.0, |mean, i| {
                    let estimate = probe(
                        Open {
                            board: node.board.snapshot(),
                            cell: node.cell,
                            zero: node.zero,
                        },
                        options.diag_shortcut,
                        &mut rng,
                    );
                    mean + (estimate - mean) / (i + 1) as f64
                });
            }
            c
        })
        .collect();
    CubeSplit {
        width: p.width,
        height: p.height,
        cubes,
        solutions_while_splitting: found,
        steps,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
    }
}

/// Deterministic identity-orientation cubes. Memo and threads are ignored; cut is enabled.
pub fn generate_cubes(
    puzzle: &Puzzle,
    minimum: usize,
    options: &Options,
    probes: usize,
) -> CubeSplit {
    if puzzle.pairs > 63 {
        generate::<wide::Many<CutCapacity>>(puzzle, minimum, options, probes)
    } else {
        generate::<CutCapacity>(puzzle, minimum, options, probes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Solve with fixed edges by pruning inconsistent states in the independent
    // normal DFS. This preserves K-rule event ordering.
    fn fixed_count(p: &Puzzle, cube: &Cube) -> u64 {
        fn agrees<C: CutState>(b: &Board<C>, fixed: &Cube) -> bool {
            fixed.edges.iter().all(|&(y, x, line)| {
                let (c, d) = if y % 2 == 0 {
                    ((y / 2 + 1) * b.shared.stride + (x - 1) / 2 + 1, R)
                } else {
                    (((y - 1) / 2 + 1) * b.shared.stride + x / 2 + 1, D)
                };
                let e = b.edges(c);
                e & if line { bit(d) << 4 } else { bit(d) } == 0
            })
        }
        fn run<C: CutState>(mut b: Board<C>, p: &Puzzle, fixed: &Cube) -> Outcome {
            const DIAG: bool = false;
            let mut out = Outcome::default();
            search_loop!(
                b,
                out,
                DIAG,
                good,
                p,
                {
                    if good
                        && (!agrees(&b, fixed)
                            || (b.remaining == 0
                                && fixed.edges.iter().any(|&(y, x, line)| {
                                    let (c, d) = if y % 2 == 0 {
                                        ((y / 2 + 1) * b.shared.stride + (x - 1) / 2 + 1, R)
                                    } else {
                                        (((y - 1) / 2 + 1) * b.shared.stride + x / 2 + 1, D)
                                    };
                                    line && b.edges(c) & bit(d) == 0
                                })))
                    {
                        good = false;
                    }
                },
                {},
                {}
            )
        }
        let mut b = Board::<CutCapacity>::new(p);
        b.pruning.interval = 25;
        run(b, p, cube).solutions
    }
    fn check(p: &Puzzle, minimum: usize) {
        let split = generate_cubes(p, minimum, &Options::default(), 0);
        assert_eq!(
            split.cubes,
            generate_cubes(p, minimum, &Options::none(), 0).cubes
        );
        let expected = solve_with(
            p,
            0,
            &Options {
                memo_bits: 0,
                ..Options::default()
            },
        )
        .solutions;
        assert_eq!(
            split.solutions_while_splitting
                + split.cubes.iter().map(|c| fixed_count(p, c)).sum::<u64>(),
            expected
        );
        for (i, c) in split.cubes.iter().enumerate() {
            assert!(c.next < p.width * p.height);
            for at in 0..c.next {
                let (y, x) = (at / p.width, at % p.width);
                if x + 1 < p.width {
                    assert!(
                        c.edges.iter().any(|e| e.0 == 2 * y && e.1 == 2 * x + 1),
                        "undecided right before next: {at}"
                    );
                }
                if y + 1 < p.height {
                    assert!(
                        c.edges.iter().any(|e| e.0 == 2 * y + 1 && e.1 == 2 * x),
                        "undecided down before next: {at}"
                    );
                }
            }
            for d in &split.cubes[..i] {
                assert!(
                    c.edges
                        .iter()
                        .any(|&(y, x, v)| d.edges.contains(&(y, x, !v))),
                    "overlapping fixed sets"
                );
            }
        }
    }
    fn check_resplit(p: &Puzzle) {
        let parents = generate_cubes(p, 3, &Options::default(), 0);
        for parent in &parents.cubes {
            let expected = fixed_count(p, parent);
            for minimum in [1, 2, 8, 64] {
                let (children, extra) = split_cube(p, parent, minimum);
                assert_eq!(
                    extra + children.iter().map(|c| fixed_count(p, c)).sum::<u64>(),
                    expected,
                    "resplit next={} minimum={minimum}",
                    parent.next
                );
                for child in &children {
                    assert!(child.next >= parent.next);
                    assert!(parent.edges.iter().all(|e| child.edges.contains(e)));
                }
            }
            for seed in [0, 1, u64::MAX] {
                let a = estimate_cube_size(p, parent, 3, seed);
                assert!(a.is_finite() && a > 0.0);
                assert_eq!(a, estimate_cube_size(p, parent, 3, seed));
            }
        }
    }
    #[test]
    fn resumed_partitions_and_estimates() {
        for text in [
            include_str!("../reference/ex1.txt"),
            include_str!("../reference/ex3.txt"),
        ] {
            check_resplit(&Puzzle::parse(text).unwrap());
        }
        let mut rng = 987654321u64;
        for _ in 0..20 {
            let mut positions: Vec<_> = (0..36).collect();
            for i in (1..36).rev() {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                positions.swap(i, rng as usize % (i + 1));
            }
            let mut p = Puzzle {
                width: 6,
                height: 6,
                pairs: 3,
                cells: vec![0; 36],
            };
            for (i, &c) in positions.iter().take(6).enumerate() {
                p.cells[c] = (i / 2 + 1) as u16;
            }
            // Start at the root as well, so exhausted/unsatisfiable random boards are tested.
            let root = Cube {
                edges: vec![],
                next: 0,
                weight: 0.0,
            };
            let (parts, found) = split_cube(&p, &root, 8);
            assert_eq!(
                found + parts.iter().map(|c| fixed_count(&p, c)).sum::<u64>(),
                fixed_count(&p, &root)
            );
            check_resplit(&p);
        }
    }
    #[test]
    fn contradictory_and_exhausted_cubes() {
        let p = Puzzle::parse("---\n1..\n...\n..1\n---").unwrap();
        let cube = Cube {
            edges: vec![(0, 1, true), (0, 1, false)],
            next: 0,
            weight: 0.0,
        };
        assert_eq!(split_cube(&p, &cube, 2), (vec![], 0));
        assert_eq!(estimate_cube_size(&p, &cube, 0, 0), 0.0);
        let c = Cube {
            edges: vec![(0, 1, true), (1, 0, false)],
            next: 0,
            weight: 0.0,
        };
        assert_eq!(
            c.to_fixed_edges(),
            vec![(true, 0, 0, true), (false, 0, 0, false)]
        );
    }
    #[test]
    fn cube_progress_tracks_exhaustion_and_partial_search() {
        let p = Puzzle::parse(include_str!("../reference/ex3.txt")).unwrap();
        for cut in [5, 25] {
            for diag_shortcut in [false, true] {
                for memo_bits in [0, 12] {
                    let options = Options {
                        cut,
                        diag_shortcut,
                        memo_bits,
                        ..Options::none()
                    };
                    let split = generate_cubes(&p, 3, &options, 0);
                    let mut partial = false;
                    let mut solutions = split.solutions_while_splitting;
                    for cube in &split.cubes {
                        let full = estimate_cube_progress(&p, cube, u64::MAX, &options);
                        let solved = solve_cube_budget(&p, cube, u64::MAX, u64::MAX, &options);
                        assert!(full.finished);
                        assert_eq!(full.progress, 1.0);
                        assert_eq!(full.estimated_total, full.steps as f64);
                        assert_eq!(
                            (full.steps, full.solutions),
                            (solved.steps, solved.solutions)
                        );
                        solutions += full.solutions;
                        let paused = estimate_cube_progress(&p, cube, 100, &options);
                        if !paused.finished && paused.progress > 0.0 {
                            partial = true;
                            assert!(paused.progress < 1.0);
                            assert!(paused.estimated_total >= paused.steps as f64);
                            assert!(paused.steps <= 100);
                        }
                    }
                    assert!(
                        partial,
                        "no partial progress: cut={cut} diag={diag_shortcut} memo={memo_bits}"
                    );
                    assert_eq!(solutions, 1);
                }
            }
        }
    }

    #[test]
    fn budgeted_cube_partition_and_limits() {
        let p = Puzzle::parse(include_str!("../reference/ex3.txt")).unwrap();
        for cut in [5, 25] {
            for diag_shortcut in [false, true] {
                for memo_bits in [0, 12] {
                    let options = Options {
                        cut,
                        diag_shortcut,
                        memo_bits,
                        ..Options::none()
                    };
                    let expected = solve_with(&p, 0, &options).solutions;
                    for minimum in [1, 3, 64] {
                        let split = generate_cubes(&p, minimum, &options, 0);
                        let mut found = split.solutions_while_splitting;
                        for cube in &split.cubes {
                            let result = solve_cube_budget(&p, cube, u64::MAX, u64::MAX, &options);
                            assert!(result.finished);
                            found += result.solutions;
                        }
                        assert_eq!(found, expected, "{options:?}, minimum {minimum}");
                    }
                    let open = &generate_cubes(&p, 1, &options, 0).cubes[0];
                    let paused = solve_cube_budget(&p, open, 1, u64::MAX, &options);
                    assert!(!paused.finished);
                    assert_eq!(paused.steps, 1);
                    assert_eq!(solve_cube_budget(&p, open, 0, u64::MAX, &options).steps, 0);
                }
            }
        }
        let p = Puzzle::parse(include_str!(
            "../tests/fixtures/end_check_across_interior.txt"
        ))
        .unwrap();
        let root = Cube {
            edges: vec![],
            next: 0,
            weight: 0.0,
        };
        for cut in [0, 1, 25] {
            for memo_bits in [0, 12] {
                let options = Options {
                    cut,
                    memo_bits,
                    ..Options::none()
                };
                let all = solve_cube_budget(&p, &root, u64::MAX, u64::MAX, &options);
                assert!(all.finished && all.solutions > 1);
                for max in [0, 1, 2] {
                    let limited = solve_cube_budget(&p, &root, u64::MAX, max, &options);
                    assert!(limited.finished);
                    assert_eq!(limited.solutions, max);
                    assert!(limited.steps <= all.steps);
                }
            }
        }
    }

    #[test]
    fn partitions() {
        for text in [
            include_str!("../reference/ex1.txt"),
            include_str!("../reference/ex3.txt"),
        ] {
            let p = Puzzle::parse(text).unwrap();
            for minimum in [1, 3, 64, 256] {
                check(&p, minimum);
            }
        }
    }
    #[test]
    fn small_multiple_solutions() {
        let mut rng = 123456789u64;
        let mut covered = [false; 4];
        for _ in 0..30000 {
            let mut positions: Vec<_> = (0..36).collect();
            for i in (1..36).rev() {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                positions.swap(i, rng as usize % (i + 1));
            }
            let mut p = Puzzle {
                width: 6,
                height: 6,
                pairs: 3,
                cells: vec![0; 36],
            };
            for (i, &position) in positions.iter().take(6).enumerate() {
                p.cells[position] = (i / 2 + 1) as u16;
            }
            let n = solve_with(
                &p,
                0,
                &Options {
                    cut: 25,
                    ..Options::none()
                },
            )
            .solutions;
            if (2..=5).contains(&n) && !covered[n as usize - 2] {
                for m in [1, 3, 8, 64] {
                    check(&p, m);
                }
                covered[n as usize - 2] = true;
            }
            if covered.iter().all(|v| *v) {
                break;
            }
        }
        assert!(covered.iter().all(|v| *v), "{covered:?}");
    }
    #[test]
    fn coordinates_3x3_and_probes() {
        let p = Puzzle::parse("---\n1..\n...\n..1\n---").unwrap();
        let mut b = Board::<CutCapacity>::new(&p);
        b.pruning.interval = 25;
        b.set_noline(b.shared.stride + 1, R);
        b.set_line(2 * b.shared.stride + 2, D);
        let c = export(&Open {
            board: b,
            cell: 2 * (p.width + 2) + 2,
            zero: false,
        });
        assert_eq!(c.next, 4);
        assert!(c.edges.contains(&(0, 1, false)));
        assert!(c.edges.contains(&(3, 2, true)));
        assert!(c
            .edges
            .iter()
            .all(|&(y, x, _)| y < 5 && x < 5 && y % 2 != x % 2));
        let a = generate_cubes(&p, 2, &Options::default(), 4);
        let b = generate_cubes(&p, 2, &Options::default(), 4);
        assert_eq!(a.cubes, b.cubes);
        assert!(a.cubes.iter().all(|c| c.weight > 0.0));
    }
}
