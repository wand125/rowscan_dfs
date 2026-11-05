mod support;
use rowscan_dfs::{solve_race, solve_with, Options, Puzzle};
use std::process::Command;

#[test]
fn random_races_preserve_selected_search() {
    let mut paused = 0;
    for seed in 0..2000 {
        let p = support::random_puzzle(seed);
        for threads in [1, 4] {
            for cut in [0, 25] {
                let options = Options {
                    threads,
                    cut,
                    ..Options::none()
                };
                let raced = solve_race(&p, 2, &options);
                let winner = &raced.candidates[raced.outcome.direction as usize];
                let serial = solve_with(
                    &p,
                    raced.outcome.direction,
                    &Options {
                        threads: 1,
                        ..options
                    },
                );
                assert_eq!(raced.outcome.solutions, serial.solutions, "seed {seed}");
                // outcome.steps includes the winner's raced work exactly once.
                let continuation_steps = raced.outcome.steps - winner.steps;
                assert_eq!(
                    continuation_steps + winner.steps,
                    serial.steps,
                    "seed {seed}"
                );
                paused += usize::from(!winner.finished);
                assert_eq!(
                    raced.race_steps,
                    raced.candidates.iter().map(|c| c.steps).sum()
                );
                for c in &raced.candidates {
                    assert!(c.steps <= 2);
                }
                #[cfg(feature = "stats")]
                assert_eq!(
                    raced
                        .outcome
                        .stats
                        .rows
                        .iter()
                        .map(|r| r.contradictions)
                        .sum::<u64>(),
                    raced.outcome.contradictions
                );
                if let Some(ref edges) = raced.outcome.first_solution {
                    support::validate_solution(&p, edges);
                }
            }
        }
        if (seed + 1) % 100 == 0 {
            eprintln!("race: {}/2000 boards × threads 1/4 × cut 0/25", seed + 1);
        }
    }
    assert!(paused > 0);
}

#[test]
fn memo_diag_adaptive_and_completed() {
    let p = Puzzle::parse(include_str!("../reference/ex3.txt")).unwrap();
    for threads in [1, 4, 8] {
        for budget in [1, 1000, 1_000_000] {
            let options = Options {
                cut: 5,
                memo_bits: 16,
                threads,
                diag_shortcut: true,
                log: false,
            };
            let race = solve_race(&p, budget, &options);
            let serial = solve_with(
                &p,
                race.outcome.direction,
                &Options {
                    threads: 1,
                    ..options
                },
            );
            assert_eq!(race.outcome.solutions, serial.solutions);
            if threads == 1 {
                assert_eq!(race.outcome.steps, serial.steps);
                assert_eq!(race.outcome.memo_hits, serial.memo_hits);
                assert_eq!(race.outcome.memo_stores, serial.memo_stores);
            }
            support::validate_solution(&p, race.outcome.first_solution.as_ref().unwrap());
        }
    }
}

#[test]
fn cli_race_and_overrides() {
    let bin = env!("CARGO_BIN_EXE_rowscan_dfs");
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/reference/ex1.txt");
    for args in [
        vec!["--race"],
        vec!["--race", "0", file],
        vec!["--race", "-1", file],
        vec!["--race", "bad", file],
        vec!["--race", "18446744073709551616", file],
    ] {
        assert_eq!(
            Command::new(bin).args(args).output().unwrap().status.code(),
            Some(2)
        );
    }
    let out = Command::new(bin)
        .args(["--race", "1", "--threads", "4", "--memo", "0", file])
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        text.lines()
            .filter(|l| l.starts_with("[race] direction"))
            .count(),
        8
    );
    for field in [
        "race budget:",
        "race steps:",
        "total elapsed:",
        "direction:",
        "elapsed:",
    ] {
        assert!(text.lines().any(|l| l.starts_with(field)));
    }
    for flag in [vec!["--direction", "3"], vec!["--all-directions"]] {
        let run = |race: bool| {
            let mut args = vec!["--memo", "0"];
            if race {
                args.extend(["--race", "1"]);
            }
            args.extend(flag.iter().copied());
            args.push(file);
            let out = Command::new(bin).args(args).output().unwrap();
            assert!(out.status.success());
            String::from_utf8(out.stdout)
                .unwrap()
                .lines()
                .filter(|l| !l.starts_with("elapsed:"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(run(true), run(false));
    }
}
