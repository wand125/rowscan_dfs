# rowscan_dfs

A dependency-free Rust row-scan DFS solver for Numberlink. It connects equal
clues with non-intersecting simple paths; unused cells are allowed. The solver
counts the shortcut normal form defined in [RULES.md](RULES.md), rather than
every raw route. Canonical counts can depend on the scan direction, and the
optional diagonal shortcut can reduce them further.

See [DESIGN.md](DESIGN.md) for the architecture and
[BENCHMARKS.md](BENCHMARKS.md) for timings and validation.

## Build and test

Install a current stable Rust toolchain.

```sh
cargo build --release
cargo test --release
cargo clippy --release --all-targets --all-features -- -D warnings
```

Tests include independent answer validation and deterministic randomized checks
across all eight directions, pruning options, memo settings, parallel searches,
and direction races.

## Input and CLI

```sh
./target/release/rowscan_dfs reference/ex1.txt
./target/release/rowscan_dfs --direction 6 --answer --answer-json reference/ex3.txt
./target/release/rowscan_dfs --threads 8 --log reference/p7933.txt
./target/release/rowscan_dfs --all-directions reference/ex1.txt
./target/release/rowscan_dfs --race 1 --threads 8 reference/p2592.txt
./target/release/rowscan_dfs --answer 'p?numlin/3/1/1g1'
```

Text input has a dashed top and bottom frame of the same width, with one
character per cell between them. `.` is blank; `1`–`9`, `a`–`z`, and `A`–`Z`
denote clue values 1–61. Each used clue character must occur exactly twice,
with at least one pair. For example:

```text
---
1.1
---
```

The CLI also accepts `https://puzz.link/p?numlin/W/H/HINTS` URLs and their bare
`p?numlin/...` form. URL hints use hexadecimal labels, `-xx` and `+xxx` extended
labels, and `g`–`z` runs of 1–20 blank cells. Up to 254 pairs are supported.
Width and height must each be 1–64. Example puzzles in `reference/` are also
used by the tests.

Defaults are **`--cut 25 --memo 26 --threads 1`**, diagonal shortcut off, and
a scan direction chosen automatically by the clue-position heuristic.
`--cut 0 --memo 0` disables cut checks and memoization.

| Option | Behavior |
|---|---|
| `--direction K` | Select transform 0–7: bit 0 transposes, bit 1 reflects original x, bit 2 reflects original y. Reflections precede transposition. Answers return to the original orientation. |
| `--all-directions` | Run all eight directions sequentially; mutually exclusive with `--direction`. |
| `--cut N` | Check row/column cut capacity at zero-based scan columns 0, N, 2N, …; 0 disables. Preserves counts. |
| `--memo BITS` | Cache exhausted failed subtrees in 2^BITS 8-byte slots; 0 disables; range 0–28. Default 26 = 512 MiB; 28 = 2 GiB. Workers in one direction share a table. |
| `--diag-shortcut` | Enable stricter diagonal canonicalization. Preserves existence and first-solution empty-cell semantics; counts may decrease. |
| `--threads N` | Use 1–256 threads with cooperative work stealing; default 1. |
| `--race B` | Probe eight directions and continue the selected state. B=1 adapts from 10,000 to 10,000,000 steps per direction; B>1 is a fixed cap. Explicit direction options bypass racing. |
| `--answer` | Print original numeric clues and box-drawing links for the first solution. |
| `--answer-json` | Print `{"width":W,"height":H,"links":[...]}` in the original orientation. |
| `--log` | Write per-steal and per-worker records to stderr. |
| `--stats` | Print event counters; requires a build with `--features stats`. |
| `--cubes M` | Split into at least M live subproblems if the frontier permits; requires `--cube-json`. M must be positive. |
| `--cube-json FILE` | Write the SAT cube JSON described below. |
| `--cube-probes P` | Estimate cube weights with P deterministic random probes per cube; default 0. Requires cube mode. |
| `-h`, `--help` | Show usage and defaults. |

Direction racing retains up to eight separate memo tables, so its nominal table
memory can reach 4 GiB at the default memo setting. Losing tables are released
before continuation. Use a smaller `--memo` value when memory is limited.

## Output

Normal stdout reports `recommended direction`, selected `direction`, `steps`,
`solutions`, `first-solution-has-empty` (`yes`, `no`, or `n/a`), and `elapsed`
in seconds. A step is an attempted branch alternative, including immediate
rejection, or a failed cut check. Initial propagation solutions take zero steps.
Elapsed includes initialization and memo allocation/release, excluding process
startup. Memo-enabled runs also print `memo hits`, `memo stores`, and `memo slots`.

Parallel runs add `threads`, `steals`, `cpu` (summed worker busy wall seconds,
including donation and excluding idle waits), and `clone` (summed board clone
seconds). With memo off, summed steps equal serial steps. With memo on,
scheduling can change hits and steps. The first parallel answer is the first
published by any worker and may differ from serial order.

Race output adds each candidate's steps, progress, estimated total work and
completion status, plus `race budget`, `race steps`, `race elapsed` and
`total elapsed`. Normal `steps` includes the winner's probe once; `race steps`
includes every probe. `elapsed` measures continuation; `total elapsed` includes
probing and continuation. Total branch work is
`steps + race steps - winner_probe_steps`.

Answers are captured during search without rerunning it. No answer is printed
when there is no solution. JSON `links` is a row-major array of one mask per
cell: up=1, right=2, down=4, left=8; 0 means unused. Both answer flags may be used
together. Invalid arguments or input produce an error on stderr and exit code 2.

## Library API and counters

```rust
use rowscan_dfs::{solve, solve_race, solve_with, Options, Puzzle};

let puzzle = Puzzle::parse("---\n1.1\n---\n").unwrap();
let direction = puzzle.recommended_direction();
let default_result = solve(&puzzle, direction);
let plain_result = solve_with(&puzzle, direction, &Options::none());
let raced = solve_race(&puzzle, 1, &Options { memo_bits: 16, ..Options::default() });
assert_eq!(default_result.solutions, plain_result.solutions);
assert!(raced.outcome.solutions > 0);
```

`solve` uses `Options::default()`. `solve_with` accepts `cut`, `memo_bits`,
`diag_shortcut`, `threads`, and `log`. `solve_race` takes a positive budget and
returns a `RaceOutcome` containing the selected `Outcome`, candidate reports,
budget, total probe steps, and probe/continuation/total seconds. Fixed-direction
calls require a direction in 0–7.

`Puzzle::from_url` returns a puzzle with dense pair IDs;
`Puzzle::decode_url` also returns the original numeric labels indexed by dense
ID minus one. `Puzzle::parse` assigns dense IDs in first-occurrence order.
`Outcome` owns its first solution's original-orientation masks and contains
steps, contradictions, solution count, empty-cell classification, memo and
parallel counters. Both first-solution fields are `None` when no solution exists.
`Puzzle::answer_text` accepts masks and a numeric value map; `answer_json` accepts
masks. `Puzzle::to_text` supports at most 61 pairs, matching the text alphabet.
The `browser` feature adds the single-worker resumable `BrowserSearch` API.
`generate_cubes` and `CubeSplit::to_json` expose cube splitting to library users.

Optional event counters compile out of the default build:

```sh
cargo build --release --features stats
./target/release/rowscan_dfs --stats --direction 6 reference/ex3.txt
```

`stats` rows contain rule family, entry mask/direction, contradictions, forced
undirected Lines/NoLines and evaluations. Branch edges are excluded; nested
calls restore attribution. `counter` rows report walks, contour visits,
worklist pops, label writes, trail entries and peak trail length. Cut capacity
updates/undo records are zero because capacities are recomputed at checks.
`detail` is a non-additive walker summary; `failure` identifies the originating
rule/site. Parallel counters sum across workers, except trail peak which takes
the maximum. Solution exhaustion is not a contradiction.

## WebAssembly

The separate `rowscan_dfs-wasm` crate builds `dfs_solver.js` and
`dfs_solver.wasm` with the JSON interface below. Install Rust's Emscripten
target, an Emscripten SDK, Python 3 and Node.js. `emcc` must be on PATH, or the
build script can source `~/emsdk/emsdk_env.sh`.

```sh
rustup target add wasm32-unknown-emscripten
cargo fetch --manifest-path wasm/Cargo.toml --target wasm32-unknown-emscripten
./wasm/build.sh
cargo test --manifest-path wasm/Cargo.toml --release
node --test wasm/tests/contract.mjs
```

Run these commands from the repository root. The script uses a local `.cache/`
and cached Cargo dependencies, so fetch dependencies before building. Outputs
are `wasm/target/wasm32-unknown-emscripten/release/deps/dfs_solver.{js,wasm}`.

| JavaScript export | Arguments and result |
|---|---|
| `_dfs_prepare` | `(url_ptr, url_len, options_ptr, options_len)` → metadata reply pointer. |
| `_dfs_solve` | Same arguments → search reply pointer. |
| `_dfs_resume` | No arguments → reply pointer after exhausting saved search, subject to `max`. |
| `_dfs_resume_budget` | `(additional: u32)` → reply pointer after at most that many more branch steps (0 exhausts); an unfinished search stays saved, so callers can poll `progress` / `est_total` between slices. |
| `_malloc`, `_free` | Allocate and free caller-owned input buffers. |

Inputs are UTF-8 URL and options-JSON buffers with byte lengths. Replies point
to a little-endian u32 byte length followed by UTF-8 JSON, owned by the module
and valid until the next ABI call. Free only input allocations. Refresh
`HEAPU8` after calls because memory can grow. Errors return `{"error":"..."}`.

Options are `direction` (0–7, -1/`"recommended"`, or `"race"`), `cut` (default
25), `memo_bits` (0–27, default **24 = 128 MiB**), `diag_shortcut` (default
false), `race_budget` and `max` (both default 0). Zero `max` means unlimited.
A numeric direction with zero budget runs to completion; a positive budget
pauses after that many steps. Budget 1 expands to `2 * (width * height)^2`.
Direct `"race"` calls probe directions sequentially, retain the best state,
and exhaust it immediately if the budget was zero. Browser workers are single
threaded; a scheduler can race independent workers.

Prepare replies include `width`, `height`, `pairs`, `recommended`, and
`value_map`; `value_map[n-1]` preserves original numeric labels in increasing
order. Solve/resume replies include dimensions, recommendation, value map,
selected `direction`, cumulative `steps`, `solutions`, `memo_hits`,
`memo_stores`, `memo_slots`, `elapsed_ms`, `progress`, `finished`, `race_budget`,
and `est_total` (`null` at zero progress). `answer` is null or contains dimensions,
original-orientation `links`, and `has_empty`. Empty-cell classification is also
returned as both `first_has_empty` and `first_solution_has_empty`.
Compute time excludes scheduler waits. A valid new solve replaces saved state;
resume consumes it without replaying the probe. A second resume returns an error.

## SAT cube export

```sh
./target/release/rowscan_dfs --cubes 64 --cube-json /tmp/ex3-cubes.json reference/ex3.txt
```

A compatible SAT cube-and-conquer driver can import it. Cube mode uses direction
**0**, rejects other directions, race and all-directions, and splits the
shallowest decision node first in stable branch order with propagation and cut
checks. Memo and threads are ignored; cut 0 selects interval 25. An exhausted
frontier can have fewer than M cubes, including zero.

The JSON object has these fields:

| Field | Meaning |
|---|---|
| `width`, `height`, `direction` | Input dimensions and fixed direction 0. |
| `cubes` | Array of disjoint unfinished subproblems. |
| `solutions_while_splitting` | Completed solutions removed from the frontier; add this count to the sum of cube results. |
| `steps`, `elapsed_ms` | Split branch work and elapsed milliseconds. |

Each cube is `{"edges":[[y,x,line],...],"next":N,"weight":W}`. Every decided
interior edge is listed once; `line` is a Boolean (`true` = Line, `false` =
NoLine). Coordinates use a `(2*height-1)` by `(2*width-1)` edge grid: cells are
at even/even coordinates, horizontal edges at even/odd, and vertical edges at
odd/even. Exterior edges are implicit NoLine. `next` is the zero-based row-major
index of the next scan decision; cells before it have decided outgoing edges.
`weight` is 0 without probes. `--cube-probes P` uses deterministic random paths
and the Knuth estimator; probe work is included in elapsed time and excluded
from split steps. Consumers must use the same orientation and canonical rules
to reproduce the solver's counts.

## License

MIT licensed, **Copyright (c) 2026 wand125**; see [LICENSE](LICENSE).
Serialization dependencies in `wasm/` retain their own licenses.

### Resuming SAT cubes

`rowscan_dfs::cubes::split_cube(&puzzle, &cube, minimum)` returns a disjoint
frontier and the number of solutions completed outside that frontier.
`estimate_cube_size(&puzzle, &cube, probes, seed)` estimates the remaining tree
with deterministic Knuth probes; inconsistent cubes estimate to zero. The
`*_with_options` variants use the same cut/diagonal settings as generation.
`Cube::to_fixed_edges()` converts edge-grid coordinates into
`(horizontal, row, column, line)` SAT tuples. Earlier cells must have both outgoing
edges decided before `next`; malformed prefixes are rejected as inconsistent.

These APIs scan the puzzle as supplied. Transform it first to choose a direction.
The SAT driver's clockwise-rotation numbering maps to rowscan directions
`[0, 5, 6, 3, 2, 7, 4, 1]`. Run the optional external reference check with
`cargo test --release --manifest-path tools/transform-check/Cargo.toml` from this
crate. That isolated harness uses the sibling original solver as a dev dependency;
the library itself remains dependency-free.
