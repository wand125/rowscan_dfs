#![cfg(feature = "browser")]
#[allow(dead_code)]
mod support;
use rowscan_dfs::{solve_with, BrowserSearch, Options, Puzzle};

#[test]
fn saved_workers_match_uninterrupted_searches() {
    let p = Puzzle::parse(include_str!("../reference/ex3.txt")).unwrap();
    for cut in [0, 1, 25] {
        for memo_bits in [0, 12] {
            for diag_shortcut in [false, true] {
                let options = Options {
                    cut,
                    memo_bits,
                    diag_shortcut,
                    ..Options::none()
                };
                for direction in 0..8 {
                    let expected = solve_with(&p, direction, &options);
                    let mut worker = BrowserSearch::new(p.clone(), direction, options);
                    let mut previous = 0.0;
                    for budget in [1, 9, 90, 900, 9000] {
                        worker.run_budget(budget, 0);
                        let (_, progress, _) = worker.report();
                        assert!(progress.progress >= previous);
                        previous = progress.progress;
                    }
                    worker.run_budget(0, 0);
                    let (actual, progress, _) = worker.report();
                    assert!(progress.finished);
                    assert_eq!(progress.progress, 1.0);
                    assert_eq!(actual.steps, expected.steps);
                    assert_eq!(actual.solutions, expected.solutions);
                    assert_eq!(actual.contradictions, expected.contradictions);
                    assert_eq!(actual.memo_hits, expected.memo_hits);
                    assert_eq!(actual.memo_stores, expected.memo_stores);
                    assert_eq!(actual.first_solution, expected.first_solution);
                    support::validate_solution(&p, actual.first_solution.as_ref().unwrap());
                }
            }
        }
    }
}

#[test]
fn provisional_solutions_and_max_survive_pause() {
    let p = Puzzle::parse(include_str!("fixtures/end_check_across_interior.txt")).unwrap();
    for memo_bits in [0, 12] {
        let options = Options {
            memo_bits,
            ..Options::none()
        };
        let expected = solve_with(&p, 0, &options);
        assert!(expected.solutions > 1);
        let mut worker = BrowserSearch::new(p.clone(), 0, options);
        while worker.report().0.solutions == 0 {
            worker.run_budget(1, 0);
        }
        let first = worker.report().0.first_solution.unwrap();
        worker.run_budget(0, 0);
        let (out, _, _) = worker.report();
        assert_eq!(out.solutions, expected.solutions);
        assert_eq!(out.first_solution.unwrap(), first);
        for max in [1, 2] {
            let mut worker = BrowserSearch::new(p.clone(), 0, options);
            worker.run_budget(0, max);
            let (out, progress, _) = worker.report();
            assert_eq!(out.solutions, max);
            assert!(progress.finished);
            assert_eq!(out.first_solution.unwrap(), first);
        }
    }
}
