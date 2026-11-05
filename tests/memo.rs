mod support;
use rowscan_dfs::{solve, solve_with, Options, Puzzle};
use std::process::Command;

#[test]
fn random_memo_all_eight_directions() {
    let mut hits = 0;
    for seed in 0..2000 {
        let p = support::random_puzzle(seed);
        for k in 0..8 {
            let off = solve_with(&p, k, &Options::none());
            let cut = solve_with(
                &p,
                k,
                &Options {
                    cut: 5,
                    ..Options::none()
                },
            );
            for options in [
                Options {
                    cut: 5,
                    memo_bits: 20,
                    ..Options::none()
                },
                Options {
                    memo_bits: 20,
                    ..Options::none()
                },
            ] {
                let result = solve_with(&p, k, &options);
                assert_eq!(
                    result.solutions, cut.solutions,
                    "seed {seed} d{k} {options:?}"
                );
                assert_eq!(result.solutions, off.solutions);
                assert!(
                    result.steps
                        <= if options.cut == 0 {
                            off.steps
                        } else {
                            cut.steps
                        }
                );
                assert_eq!(result.first_solution, off.first_solution);
                assert_eq!(
                    result.first_solution_has_empty,
                    off.first_solution_has_empty
                );
                assert_eq!(result.memo_slots, 1 << 20);
                if let Some(ref edges) = result.first_solution {
                    support::validate_solution(&p, edges);
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
                hits += result.memo_hits;
            }
        }
        if (seed + 1) % 100 == 0 {
            eprintln!("memo: {} boards × 8 directions", seed + 1);
        }
    }
    eprintln!("memo: random hits: {hits}");
}

#[test]
fn ex3_d6_and_tiny_collision_table() {
    let p = Puzzle::parse(include_str!("../reference/ex3.txt")).unwrap();
    let off = solve_with(&p, 6, &Options::none());
    let memo = solve_with(
        &p,
        6,
        &Options {
            memo_bits: 22,
            ..Options::none()
        },
    );
    assert_eq!(off.steps, 154_834);
    assert_eq!(memo.solutions, 1);
    assert!(memo.steps < off.steps);
    assert!(memo.memo_hits > 0 && memo.memo_stores > 0);
    assert_eq!(memo.first_solution, off.first_solution);
    #[cfg(feature = "stats")]
    assert_eq!(
        memo.stats
            .rows
            .iter()
            .map(|r| r.contradictions)
            .sum::<u64>(),
        memo.contradictions
    );
    eprintln!(
        "ex3 d6 memo 22: {} steps, {} hits, {} stores",
        memo.steps, memo.memo_hits, memo.memo_stores
    );
    let tiny = solve_with(
        &p,
        6,
        &Options {
            memo_bits: 1,
            ..Options::none()
        },
    );
    assert_eq!(tiny.solutions, off.solutions);
    assert!(tiny.steps <= off.steps);
}

#[test]
fn defaults_and_cli_memo_validation() {
    assert_eq!(
        Options::default(),
        Options {
            cut: 25,
            memo_bits: 26,
            diag_shortcut: false,
            threads: 1,
            log: false
        }
    );
    let p = Puzzle::parse("---\n1.1\n---\n").unwrap();
    let result = solve(&p, 0);
    assert_eq!(result.solutions, 1);
    assert_eq!(result.memo_slots, 1 << 26);
    let bin = env!("CARGO_BIN_EXE_rowscan_dfs");
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/reference/ex1.txt");
    let output = Command::new(bin).arg(file).output().unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("memo slots: 67108864"));
    let output = Command::new(bin)
        .args(["--cut", "0", "--memo", "0", file])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8(output.stdout).unwrap().contains("memo "));
    for args in [
        vec!["--memo"],
        vec!["--memo", "29", file],
        vec!["--memo", "-1", file],
        vec!["--memo", "bad", file],
    ] {
        assert_eq!(
            Command::new(bin).args(args).output().unwrap().status.code(),
            Some(2)
        );
    }
    let help = Command::new(bin).arg("--help").output().unwrap();
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("--cut 25 --memo 26") && help.contains("512 MiB"));
}
