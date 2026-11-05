use rowscan_dfs::{solve, solve_race, solve_with, Options, Puzzle};
use std::{env, fs, process, time::Instant};
fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let mut direction = None;
    let mut all = false;
    let mut race = None;
    let mut stats = false;
    let mut answer = false;
    let mut answer_json = false;
    let mut cubes = None;
    let mut cube_json = None;
    let mut cube_probes = 0usize;
    let mut file = None;
    let mut options = Options::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--cubes" => {
                let n = args
                    .next()
                    .ok_or("missing cube count")?
                    .parse::<usize>()
                    .map_err(|_| "cube count must be positive")?;
                if n == 0 {
                    return Err("cube count must be positive".into());
                }
                cubes = Some(n);
            }
            "--cube-json" => cube_json = Some(args.next().ok_or("missing cube JSON path")?),
            "--cube-probes" => {
                cube_probes = args
                    .next()
                    .ok_or("missing probe count")?
                    .parse()
                    .map_err(|_| "probe count must be nonnegative")?
            }
            "--race" => {
                let budget = args
                    .next()
                    .ok_or("missing race budget")?
                    .parse::<u64>()
                    .map_err(|_| "race budget must be a positive integer")?;
                if budget == 0 {
                    return Err("race budget must be a positive integer".into());
                }
                race = Some(budget);
            }
            "--threads" => {
                options.threads = args
                    .next()
                    .ok_or("missing thread count")?
                    .parse()
                    .map_err(|_| "threads must be 1..=256")?;
                if !(1..=256).contains(&options.threads) {
                    return Err("threads must be 1..=256".into());
                }
            }
            "--answer" => answer = true,
            "--answer-json" => answer_json = true,
            "--log" => options.log = true,
            "--cut" => {
                options.cut = args
                    .next()
                    .ok_or("missing cut interval")?
                    .parse()
                    .map_err(|_| "cut interval must be a nonnegative integer")?;
            }
            "--memo" => {
                options.memo_bits = args
                    .next()
                    .ok_or("missing memo bits")?
                    .parse()
                    .map_err(|_| "memo bits must be 0..=28")?;
                if options.memo_bits > 28 {
                    return Err("memo bits must be 0..=28".into());
                }
            }
            "--diag-shortcut" => options.diag_shortcut = true,
            "--stats" => {
                if !cfg!(feature = "stats") {
                    return Err("--stats requires cargo build --features stats".into());
                }
                stats = true;
            }
            "--direction" => {
                if direction.is_some() || all {
                    return Err("choose one direction option".into());
                }
                let k = args
                    .next()
                    .ok_or("missing direction")?
                    .parse::<u8>()
                    .map_err(|_| "direction must be 0..7")?;
                if k > 7 {
                    return Err("direction must be 0..7".into());
                }
                direction = Some(k);
            }
            "--all-directions" => {
                if all || direction.is_some() {
                    return Err("choose one direction option".into());
                }
                all = true;
            }
            "-h" | "--help" => {
                println!("rowscan_dfs [--stats] [--threads N] [--log] [--race B] [--direction K|--all-directions] [--cut N] [--memo BITS] [--diag-shortcut] [--answer] [--answer-json] FILE|URL");
                println!("Defaults: --cut 25 --memo 26; 0 disables either. Memo: 2^BITS 8-byte slots (26 = 512 MiB, 28 = 2 GiB), BITS 0..28. Diagonal shortcut defaults off.");
                println!("Threads default to 1. Parallel first solution is the first published by any worker and may differ from serial order. Directions run sequentially. --log writes steal events to stderr.");
                println!("--race B races eight directions; 1 adapts from 10000 to 10000000 steps. Explicit direction options skip the race. Race steps sum all candidates; steps include the winner's race work.");
                println!("--cubes M --cube-json FILE [--cube-probes P]: split breadth-first with cut enabled and memo ignored. Only direction 0 (default in cube mode) is supported.");
                return Ok(());
            }
            _ => {
                if arg.starts_with('-') || file.replace(arg).is_some() {
                    return Err(
                        "usage: rowscan_dfs [--stats] [--threads N] [--log] [--race B] [--direction K|--all-directions] [--cut N] [--memo BITS] [--diag-shortcut] [--answer] [--answer-json] FILE|URL".into(),
                    );
                }
            }
        }
    }
    if cubes.is_some() != cube_json.is_some() || (cubes.is_none() && cube_probes != 0) {
        return Err("use --cubes M --cube-json FILE together".into());
    }
    if cubes.is_some() && (all || race.is_some() || direction.unwrap_or(0) != 0) {
        return Err(
            "cube emission supports only --direction 0, without race or all-directions".into(),
        );
    }
    let file = file.ok_or("usage: rowscan_dfs [--stats] [--threads N] [--log] [--race B] [--direction K|--all-directions] [--cut N] [--memo BITS] [--diag-shortcut] [--answer] [--answer-json] FILE|URL")?;
    let (puzzle, values) = if file.starts_with("https://") || file.starts_with("p?") {
        Puzzle::decode_url(&file)?
    } else {
        let text = fs::read_to_string(file).map_err(|e| e.to_string())?;
        let puzzle = Puzzle::parse(&text)?;
        let mut values = Vec::new();
        if answer {
            for b in text.bytes().filter(u8::is_ascii_alphanumeric) {
                let value = match b {
                    b'1'..=b'9' => u16::from(b - b'0'),
                    b'a'..=b'z' => u16::from(b - b'a') + 10,
                    b'A'..=b'Z' => u16::from(b - b'A') + 36,
                    _ => continue,
                };
                if !values.contains(&value) {
                    values.push(value);
                }
            }
        }
        (puzzle, values)
    };
    if let Some(minimum) = cubes {
        let split = rowscan_dfs::generate_cubes(&puzzle, minimum, &options, cube_probes);
        fs::write(cube_json.unwrap(), split.to_json()).map_err(|e| e.to_string())?;
        println!(
            "cubes: {}  split steps: {}  elapsed: {:.6} s",
            split.cubes.len(),
            split.steps,
            split.elapsed_ms / 1000.0
        );
        println!("cube solutions: {}", split.solutions_while_splitting);
        return Ok(());
    }
    let recommended = puzzle.recommended_direction();
    println!("recommended direction: {recommended}");
    let directions: Vec<_> = if all {
        (0..8).collect()
    } else {
        vec![direction.unwrap_or(recommended)]
    };
    for k in directions {
        let start = Instant::now();
        let raced = race
            .filter(|_| !all && direction.is_none())
            .map(|budget| solve_race(&puzzle, budget, &options));
        if let Some(ref race) = raced {
            for candidate in &race.candidates {
                println!(
                    "[race] direction {}: steps {} progress {:.9} est {:.3}{}{}",
                    candidate.direction,
                    candidate.steps,
                    candidate.progress,
                    candidate.estimated_total,
                    if candidate.direction == recommended {
                        " (recommended)"
                    } else {
                        ""
                    },
                    if candidate.finished {
                        " (finished)"
                    } else {
                        ""
                    }
                );
            }
            println!(
                "race budget: {}\nrace steps: {}\nrace elapsed: {:.6} s\ntotal elapsed: {:.6} s",
                race.budget, race.race_steps, race.race_seconds, race.total_seconds
            );
            println!("steps convention: includes selected candidate race work");
        }
        let continuation = raced.as_ref().map(|r| r.continuation_seconds);
        let result = if let Some(race) = raced {
            race.outcome
        } else if options == Options::default() {
            solve(&puzzle, k)
        } else {
            solve_with(&puzzle, k, &options)
        };
        let elapsed = continuation.unwrap_or_else(|| start.elapsed().as_secs_f64());
        let k = result.direction;
        println!("direction: {k}\nsteps: {}\nsolutions: {}\nfirst-solution-has-empty: {}\nelapsed: {:.6} s",result.steps,result.solutions,
            match result.first_solution_has_empty {Some(true)=>"yes",Some(false)=>"no",None=>"n/a"},elapsed);
        if options.memo_bits != 0 {
            println!(
                "memo hits: {}\nmemo stores: {}\nmemo slots: {}",
                result.memo_hits, result.memo_stores, result.memo_slots
            );
        }
        if options.threads > 1 {
            println!(
                "threads: {}\nsteals: {}\ncpu: {:.6} s\nclone: {:.6} s",
                result.threads, result.steals, result.cpu, result.clone_seconds
            );
            println!("first solution: first published by any worker; may differ from serial order");
        }
        #[cfg(feature = "stats")]
        if stats {
            print!("{}", result.stats);
        }
        #[cfg(not(feature = "stats"))]
        let _ = stats;
        if let Some(ref links) = result.first_solution {
            if answer {
                println!("{}", puzzle.answer_text(links, &values));
            }
            if answer_json {
                println!("{}", puzzle.answer_json(links));
            }
        }
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("rowscan_dfs: {e}");
        process::exit(2);
    }
}
