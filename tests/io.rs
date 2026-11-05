#[allow(dead_code)]
mod support;
use rowscan_dfs::{solve_race, solve_with, Options, Puzzle};
use std::process::Command;

#[test]
fn sample_urls_match_text_boards() {
    for line in include_str!("fixtures/urls.txt")
        .lines()
        .filter(|l| !l.trim().is_empty())
    {
        let (name, url) = line.split_once(' ').unwrap();
        let text = std::fs::read_to_string(format!("reference/{name}.txt")).unwrap();
        let expected = Puzzle::parse(&text).unwrap();
        assert_eq!(Puzzle::from_url(url).unwrap(), expected, "{name}");
        assert_eq!(
            Puzzle::from_url(url.strip_prefix("https://puzz.link/").unwrap()).unwrap(),
            expected
        );
    }
}

#[test]
fn url_validation_and_original_values() {
    for url in [
        "p?numlin/0/1/11",
        "p?numlin/65/1/11",
        "p?numlin/x/1/11",
        "p?numlin/2/1/1",
        "p?numlin/2/1/111",
        "p?numlin/2/1/12",
        "p?numlin/2/1/00",
        "p?numlin/2/1/-0f-0f",
        "p?numlin/2/1/+01",
        "p?numlin/2/1/-gg11",
        "p?numlin/2/1/zz",
        "p?numlin/2/1/11/",
        "https://other.test/p?numlin/2/1/11",
        "p?numlin/2/1/é",
    ] {
        assert!(Puzzle::from_url(url).is_err(), "{url}");
    }
    let (p, values) = Puzzle::decode_url("p?numlin/2/2/+abc-10+abc-10").unwrap();
    assert_eq!(values, [0xabc, 16]);
    assert_eq!(p.cells, [1, 2, 1, 2]);
    assert_eq!(
        p.answer_text(&[4, 4, 1, 1], &values),
        "2748   16\n2748   16"
    );
    let (p, values) = Puzzle::decode_url("p?numlin/3/1/1g1").unwrap();
    assert_eq!(p.answer_text(&[2, 10, 8], &values), "1 ─ 1");
    assert_eq!(
        p.answer_json(&[2, 10, 8]),
        "{\"width\":3,\"height\":1,\"links\":[2,10,8]}"
    );
}

#[test]
fn answers_valid_and_identical_in_original_orientation() {
    for name in ["ex1", "ex3"] {
        let p = Puzzle::parse(&std::fs::read_to_string(format!("reference/{name}.txt")).unwrap())
            .unwrap();
        let mut expected = None;
        for direction in 0..8 {
            for threads in [1, 4] {
                let out = solve_with(
                    &p,
                    direction,
                    &Options {
                        cut: 25,
                        memo_bits: 12,
                        threads,
                        ..Options::none()
                    },
                );
                assert_eq!(out.solutions, 1);
                let links = out.first_solution.unwrap();
                support::validate_solution(&p, &links);
                if let Some(ref expected) = expected {
                    assert_eq!(&links, expected);
                } else {
                    expected = Some(links);
                }
            }
        }
    }
}

#[test]
fn cli_answer_and_url() {
    let file = Command::new(env!("CARGO_BIN_EXE_rowscan_dfs"))
        .args([
            "--memo",
            "0",
            "--answer-json",
            "--answer",
            "reference/ex3.txt",
        ])
        .output()
        .unwrap();
    assert!(file.status.success());
    let url = include_str!("fixtures/urls.txt")
        .lines()
        .find(|l| l.starts_with("ex3 "))
        .unwrap()
        .split_once(' ')
        .unwrap()
        .1;
    let output = Command::new(env!("CARGO_BIN_EXE_rowscan_dfs"))
        .args(["--memo", "0", "--answer-json", "--answer", url])
        .output()
        .unwrap();
    assert!(output.status.success());
    let normalize = |bytes: Vec<u8>| {
        String::from_utf8(bytes)
            .unwrap()
            .lines()
            .filter(|l| !l.starts_with("elapsed:"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(normalize(file.stdout), normalize(output.stdout));
}

#[test]
fn extended_labels_up_to_254_pairs_solve_without_mask_aliasing() {
    for pairs in [62usize, 63, 64, 127, 254] {
        let height = pairs.div_ceil(16);
        let mut hints = String::new();
        for pair in 1..=pairs {
            let label = if pair < 16 {
                format!("{pair:x}")
            } else {
                format!("-{pair:02x}")
            };
            hints.push_str(&label);
            hints.push_str(&label);
        }
        hints.push_str(&"g".repeat(32 * height - pairs * 2));
        let p = Puzzle::from_url(&format!("p?numlin/32/{height}/{hints}")).unwrap();
        assert_eq!(p.pairs, pairs);
        let race = solve_race(
            &p,
            100,
            &Options {
                cut: 25,
                memo_bits: 12,
                threads: 4,
                ..Options::none()
            },
        );
        assert_eq!(race.outcome.solutions, 1);
        support::validate_solution(&p, race.outcome.first_solution.as_ref().unwrap());
        for direction in 0..8 {
            let out = solve_with(&p, direction, &Options::none());
            assert_eq!(out.solutions, 1);
            support::validate_solution(&p, out.first_solution.as_ref().unwrap());
        }
    }
}

#[test]
fn wide_pair_regions_keep_the_ex3_answer() {
    let original = Puzzle::parse(include_str!("../reference/ex3.txt")).unwrap();
    let expected = solve_with(&original, 6, &Options::none())
        .first_solution
        .unwrap();
    for pairs in [63, 87] {
        let mut cells = Vec::new();
        let mut next = original.pairs as u16 + 1;
        let width = if pairs == 63 { 23 } else { 25 };
        for (y, row) in original.cells.chunks(original.width).enumerate() {
            cells.extend_from_slice(row);
            let extra = if pairs == 63 {
                if y < 6 {
                    4
                } else {
                    3
                }
            } else {
                5
            };
            for _ in 0..extra {
                cells.extend([next, next]);
                next += 1;
            }
            cells.resize((y + 1) * width, 0);
        }
        let p = Puzzle {
            width,
            height: 15,
            pairs: next as usize - 1,
            cells,
        };
        assert_eq!(p.pairs, pairs);
        for direction in 0..8 {
            let out = solve_with(
                &p,
                direction,
                &Options {
                    cut: 25,
                    memo_bits: 12,
                    threads: 4,
                    ..Options::none()
                },
            );
            assert_eq!(out.solutions, 1);
            let links = out.first_solution.unwrap();
            support::validate_solution(&p, &links);
            for y in 0..15 {
                assert_eq!(
                    &links[y * width..y * width + 15],
                    &expected[y * 15..y * 15 + 15]
                );
            }
        }
    }
}
