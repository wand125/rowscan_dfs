mod support;
use rowscan_dfs::{solve_with, Options, Outcome, Puzzle};
use std::process::Command;

fn check(p: &Puzzle, serial: &Outcome, parallel: &Outcome, memo: bool) {
    assert_eq!(parallel.solutions, serial.solutions);
    if !memo {
        assert_eq!(parallel.steps, serial.steps);
    }
    assert_eq!(
        parallel.first_solution.is_some(),
        serial.first_solution.is_some()
    );
    if let Some(ref edges) = parallel.first_solution {
        support::validate_solution(p, edges);
        assert_eq!(parallel.first_solution_has_empty, Some(edges.contains(&0)));
    }
    #[cfg(feature = "stats")]
    assert_eq!(
        parallel
            .stats
            .rows
            .iter()
            .map(|r| r.contradictions)
            .sum::<u64>(),
        parallel.contradictions
    );
}

#[test]
fn random_parallel_all_directions() {
    let mut over = 0;
    let mut maximum = 0.0f64;
    let mut steals = 0;
    for seed in 0..2000 {
        let p = support::random_puzzle(seed);
        for k in 0..8 {
            for cut in [0, 25] {
                for memo_bits in [0, 20] {
                    let options = Options {
                        cut,
                        memo_bits,
                        ..Options::none()
                    };
                    let serial = solve_with(&p, k, &options);
                    let parallel = solve_with(
                        &p,
                        k,
                        &Options {
                            threads: 4,
                            ..options
                        },
                    );
                    check(&p, &serial, &parallel, memo_bits != 0);
                    if memo_bits != 0 && serial.steps > 0 {
                        let ratio = parallel.steps as f64 / serial.steps as f64;
                        maximum = maximum.max(ratio);
                        over += u64::from(ratio > 1.05);
                    }
                    steals += parallel.steals;
                }
            }
        }
        if (seed + 1) % 100 == 0 {
            eprintln!(
                "parallel: {}/2000 boards × 8 directions × cut 0/25 × memo 0/20",
                seed + 1
            );
        }
    }
    eprintln!("parallel: random: {steals} donations, memo max step ratio {maximum:.6}, {over} over 1.05 (reported only)");
    assert!(steals > 0);
}

#[test]
fn references_and_termination() {
    let mut steals = 0;
    for text in [
        include_str!("../reference/ex1.txt"),
        include_str!("../reference/ex3.txt"),
    ]
    .into_iter()
    .chain([
        "---\n1.1\n---\n",
        "----\n12..\n21..\n----\n",
        include_str!("fixtures/end_check_across_interior.txt"),
    ]) {
        let p = Puzzle::parse(text).unwrap();
        for k in 0..8 {
            for options in [
                Options::none(),
                Options {
                    cut: 25,
                    memo_bits: 20,
                    ..Options::none()
                },
                Options {
                    cut: 5,
                    diag_shortcut: true,
                    ..Options::none()
                },
            ] {
                let serial = solve_with(&p, k, &options);
                for threads in [2, 4, 8] {
                    for _ in 0..3 {
                        let result = solve_with(&p, k, &Options { threads, ..options });
                        check(&p, &serial, &result, options.memo_bits != 0);
                        steals += result.steals;
                        if options.memo_bits != 0 && serial.steps > 0 {
                            eprintln!(
                                "parallel: reference d{k} t{threads}: memo step ratio {:.6}",
                                result.steps as f64 / serial.steps as f64
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(steals > 0);
}

#[test]
fn cli_threads_and_log() {
    let bin = env!("CARGO_BIN_EXE_rowscan_dfs");
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/reference/ex3.txt");
    for args in [
        vec!["--threads"],
        vec!["--threads", "0", file],
        vec!["--threads", "257", file],
        vec!["--threads", "-1", file],
        vec!["--threads", "bad", file],
    ] {
        assert_eq!(
            Command::new(bin).args(args).output().unwrap().status.code(),
            Some(2)
        );
    }
    let normalize = |text: Vec<u8>| {
        String::from_utf8(text)
            .unwrap()
            .lines()
            .filter(|l| !l.starts_with("elapsed: "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let plain = Command::new(bin)
        .args(["--cut", "0", "--memo", "0", file])
        .output()
        .unwrap();
    let one = Command::new(bin)
        .args(["--threads", "1", "--log", "--cut", "0", "--memo", "0", file])
        .output()
        .unwrap();
    assert_eq!(normalize(plain.stdout), normalize(one.stdout));
    assert!(one.stderr.is_empty());
    let result = Command::new(bin)
        .args([
            "--threads",
            "4",
            "--log",
            "--all-directions",
            "--cut",
            "0",
            "--memo",
            "0",
            file,
        ])
        .output()
        .unwrap();
    assert!(result.status.success());
    let stdout = String::from_utf8(result.stdout).unwrap();
    for field in [
        "threads: 4",
        "steals: ",
        "cpu: ",
        "clone: ",
        "first solution: ",
    ] {
        assert_eq!(stdout.lines().filter(|l| l.starts_with(field)).count(), 8);
    }
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(stderr.contains("[steal]") && stderr.contains("[done]"));
}
