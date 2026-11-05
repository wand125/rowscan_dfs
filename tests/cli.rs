use rowscan_dfs::{solve_with, Options, Puzzle};
use std::process::Command;

#[test]
fn endpoint_scheduling_regressions() {
    let cases = [
        (
            include_str!("fixtures/end_check_before_extension.txt"),
            [1; 8],
        ),
        (
            include_str!("fixtures/end_check_across_interior.txt"),
            [3; 8],
        ),
        (
            include_str!("fixtures/end_rescan_order.txt"),
            [3, 2, 3, 2, 2, 2, 2, 2],
        ),
        (include_str!("fixtures/walk_rechecks_both_ends.txt"), [4; 8]),
    ];
    for (text, expected) in cases {
        let puzzle = Puzzle::parse(text).unwrap();
        for k in 0..8 {
            assert_eq!(
                solve_with(&puzzle, k, &Options::none()).solutions,
                expected[k as usize],
                "direction {k}\n{text}"
            );
        }
    }
}

#[test]
fn cli_all_directions_and_validation() {
    let bin = env!("CARGO_BIN_EXE_rowscan_dfs");
    let file = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/end_check_before_extension.txt"
    );
    let result = Command::new(bin)
        .args(["--cut", "0", "--memo", "0"])
        .args(["--all-directions", file])
        .output()
        .unwrap();
    assert!(result.status.success());
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(text.starts_with("recommended direction: "));
    for field in [
        "direction: ",
        "steps: ",
        "solutions: 1",
        "first-solution-has-empty: yes",
        "elapsed: ",
    ] {
        assert_eq!(
            text.lines().filter(|line| line.starts_with(field)).count(),
            8,
            "{text}"
        );
    }
    for args in [
        vec!["--direction", "8", file],
        vec!["--direction", "0", "--all-directions", file],
        vec!["--unknown", file],
        vec![],
    ] {
        let result = Command::new(bin)
            .args(["--cut", "0", "--memo", "0"])
            .args(args)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2));
        assert!(!result.stderr.is_empty());
    }
}

#[test]
fn cli_stats_feature() {
    let bin = env!("CARGO_BIN_EXE_rowscan_dfs");
    let file = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/end_check_before_extension.txt"
    );
    let result = Command::new(bin)
        .args(["--cut", "0", "--memo", "0"])
        .args(["--stats", file])
        .output()
        .unwrap();
    if cfg!(feature = "stats") {
        assert!(result.status.success());
        let text = String::from_utf8(result.stdout).unwrap();
        assert!(text.contains("stats,rule,mask,C,L,W,evaluations"));
        assert!(text.contains("counter,worklist_pops,"));
    } else {
        assert_eq!(result.status.code(), Some(2));
        assert!(String::from_utf8(result.stderr)
            .unwrap()
            .contains("--features stats"));
    }
}
