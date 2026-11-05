mod support;
use rowscan_dfs::{solve_with, Options, Puzzle};
use std::process::Command;
use support::{random_puzzle, validate_solution};

#[test]
fn random_pruning_all_eight_directions() {
    let modes = [
        Options {
            cut: 1,
            diag_shortcut: false,
            memo_bits: 0,
            ..Options::none()
        },
        Options {
            cut: 5,
            diag_shortcut: false,
            memo_bits: 0,
            ..Options::none()
        },
        Options {
            cut: 0,
            diag_shortcut: true,
            memo_bits: 0,
            ..Options::none()
        },
        Options {
            cut: 5,
            diag_shortcut: true,
            memo_bits: 0,
            ..Options::none()
        },
    ];
    let started = std::time::Instant::now();
    for seed in 0..2000 {
        let p = random_puzzle(seed);
        let off: Vec<_> = (0..8)
            .map(|k| solve_with(&p, k, &Options::none()))
            .collect();
        for options in modes {
            for k in 0..8 {
                let result = solve_with(&p, k, &options);
                let base = &off[k as usize];
                assert!(result.steps <= base.steps, "seed {seed}, d{k}, {options:?}");
                assert_eq!(
                    result.solutions > 0,
                    base.solutions > 0,
                    "seed {seed}, d{k}, {options:?}"
                );
                assert_eq!(
                    result.first_solution_has_empty, base.first_solution_has_empty,
                    "seed {seed}, d{k}, {options:?}"
                );
                if options.diag_shortcut {
                    assert!(result.solutions <= base.solutions);
                } else {
                    assert_eq!(result.solutions, base.solutions);
                }
                if let Some(ref edges) = result.first_solution {
                    validate_solution(&p, edges);
                }
                #[cfg(feature = "stats")]
                assert_eq!(
                    result
                        .stats
                        .rows
                        .iter()
                        .map(|r| r.contradictions)
                        .sum::<u64>(),
                    result.contradictions
                );
            }
        }
        if (seed + 1) % 100 == 0 {
            eprintln!(
                "pruning: {} boards × 8 directions, {:.2}s",
                seed + 1,
                started.elapsed().as_secs_f64()
            );
        }
    }
}

#[test]
fn reference_pruning_all_directions() {
    for text in [
        include_str!("../reference/ex1.txt"),
        include_str!("../reference/ex3.txt"),
    ] {
        let p = Puzzle::parse(text).unwrap();
        for k in 0..8 {
            let off = solve_with(&p, k, &Options::none());
            for options in [
                Options {
                    cut: 25,
                    diag_shortcut: false,
                    memo_bits: 0,
                    ..Options::none()
                },
                Options {
                    cut: 0,
                    diag_shortcut: true,
                    memo_bits: 0,
                    ..Options::none()
                },
                Options {
                    cut: 25,
                    diag_shortcut: true,
                    memo_bits: 0,
                    ..Options::none()
                },
            ] {
                let result = solve_with(&p, k, &options);
                assert_eq!(result.solutions, off.solutions);
                assert_eq!(
                    result.first_solution_has_empty,
                    off.first_solution_has_empty
                );
                assert!(result.steps <= off.steps);
            }
        }
    }
}

#[test]
fn cli_pruning_options() {
    let bin = env!("CARGO_BIN_EXE_rowscan_dfs");
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/reference/ex1.txt");
    for args in [
        vec!["--cut", "0", file],
        vec!["--cut", "5", "--diag-shortcut", "--all-directions", file],
    ] {
        let output = Command::new(bin)
            .args(["--cut", "0", "--memo", "0"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    for args in [
        vec!["--cut"],
        vec!["--cut", "-1", file],
        vec!["--cut", "bad", file],
    ] {
        assert_eq!(
            Command::new(bin)
                .args(["--cut", "0", "--memo", "0"])
                .args(args)
                .status()
                .unwrap()
                .code(),
            Some(2)
        );
    }
}

#[test]
fn cut_failure_steps_match_scan_convention() {
    let p = Puzzle::parse(include_str!("../reference/ex3.txt")).unwrap();
    let expected = [
        198986, 138713, 185524, 210729, 147802, 115625, 142518, 144813,
    ];
    for k in 0..8 {
        assert_eq!(
            solve_with(
                &p,
                k,
                &Options {
                    cut: 25,
                    diag_shortcut: false,
                    memo_bits: 0,
                    ..Options::none()
                }
            )
            .steps,
            expected[k as usize]
        );
    }
}
