# Benchmarks

Recorded on **2026-11-05** with release builds and event counters disabled.
Each table uses the final selected build for its measurement series; tables
from different series are not a controlled comparison of builds. Native
elapsed time includes solver initialization and memo table allocation/release,
and excludes process startup. Except where stated otherwise, values are means
of two interleaved repetitions, using cut 25, memo 26 (512 MiB), diagonal
shortcut off, and the automatically recommended direction.

The measured CPUs were **Apple M4 Pro** (arm64, macOS 26.2,
Rust 1.99.0) and **AMD Ryzen 9 9950X** (x86_64, Linux,
Rust 1.98.0 for the scaling series). Measurements were taken on shared machines;
small differences can reflect background load. Parallel memo work and steals
can vary with scheduling.

## Serial search with default options

Date: **2026-11-05**. One thread, cut 25, memo 26, diagonal shortcut off.
The two CPU columns are separate measurement series.

| Input | Recommended direction | Apple M4 Pro elapsed s | AMD Ryzen 9 9950X elapsed s |
|---|---:|---:|---:|
| `reference/ex1.txt` | 6 | — | — |
| `reference/ex3.txt` | 6 | 0.132319 | — |
| `reference/p2592.txt` | 3 | 29.337505 | 26.153587 |
| `reference/p7933.txt` | 1 | 39.518569 | 32.743173 |
| `reference/p10157.txt` | 3 | 19.814371 | 16.442761 |

A dash means no default-option serial timing was recorded in these series.
In particular, ex1 has validation coverage but no timing here.

```sh
cargo build --release
./target/release/rowscan_dfs reference/ex3.txt
./target/release/rowscan_dfs reference/p7933.txt
```

## Thread scaling

CPU: **AMD Ryzen 9 9950X**. Date: **2026-11-05**.
`reference/p7933.txt`, direction 1, cut 25, memo 26, diagonal shortcut off.

| Threads | Elapsed s | Speedup over 1 thread |
|---:|---:|---:|
| 1 | 34.272627 | 1.000× |
| 2 | 17.342737 | 1.976× |
| 4 | 8.585523 | 3.992× |
| 8 | 4.533217 | 7.560× |

A separate series on the same CPU and date measured eight-worker elapsed times
of 3.265386 s for p2592, 4.200553 s for p7933, and 2.233257 s for p10157,
with the same options and recommended directions. Those observations belong
to the serial table's series, rather than the scaling series above.

```sh
./target/release/rowscan_dfs --threads 2 reference/p7933.txt
./target/release/rowscan_dfs --threads 4 reference/p7933.txt
./target/release/rowscan_dfs --threads 8 reference/p7933.txt
```

## Memo on/off effect

CPU: **Apple M4 Pro**. Date: **2026-11-05**.
One thread, cut 25, recommended direction, diagonal shortcut off.
Memo-on uses 26 bits; memo-off uses 0. All table costs are included in elapsed.

| Input | Memo-off steps | Memo-on steps | Memo-off s | Memo-on s | On/off time |
|---|---:|---:|---:|---:|---:|
| `reference/p2592.txt` | 36,994,332 | 34,098,923 | 30.412629 | 29.337505 | 0.9646× |
| `reference/p7933.txt` | 45,267,342 | 43,446,031 | 39.173054 | 39.518569 | 1.0088× |
| `reference/p10157.txt` | 26,930,122 | 24,450,668 | 20.564097 | 19.814371 | 0.9635× |

Memo saves steps on all three inputs but slightly increases p7933 wall time.
Fewer search steps alone do not guarantee a faster solve.

```sh
./target/release/rowscan_dfs --memo 0 reference/p7933.txt
./target/release/rowscan_dfs --memo 26 reference/p7933.txt
```

## Direction racing

CPU: **AMD Ryzen 9 9950X**. Date: **2026-11-05**.
`--race 1 --threads 8 --cut 25 --memo 26`, diagonal shortcut off.
Both repetitions selected the same direction and ended at a cumulative probe
budget of 1,000,000 steps per direction.

| Input | Selected direction | Total elapsed s | Total branch work | Work / best fixed direction |
|---|---:|---:|---:|---:|
| `reference/p2592.txt` | 3 | 5.059798 | 41,124,599 | 1.9727× |
| `reference/p7933.txt` | 3 | 3.955407 | 30,274,354 | 1.3008× |
| `reference/p10157.txt` | 5 | 2.790974 | 19,647,108 | 1.5535× |

Total elapsed includes all probes and continuation. Total work counts the
winner's probe once and adds losing probes. The denominator is the minimum
serial step count across all eight directions with the same cut/memo settings:
p2592 d4 = 20,847,086; p7933 d3 = 23,274,436; p10157 d5 = 12,646,788.
The direction-enumeration runs supply step counts only, not timing samples.
A race estimates a useful direction; it does not guarantee the least work.

```sh
./target/release/rowscan_dfs --race 1 --threads 8 reference/p2592.txt
```

### puzsq 178-board summary

CPU: **AMD Ryzen 9 9950X**. Date: **2026-11-05**.
One observation per board, `--race 1 --threads 8 --cut 25 --memo 26`,
diagonal shortcut off, with a 300-second cap per solve.

| Measure | Result |
|---|---:|
| Completed | 177 / 178 |
| Timed out | 1 |
| Sum of elapsed seconds for completed solves | 275.887 |
| Geometric mean of total work / best completed fixed direction | 4.0409× over 175 comparable boards |
| Same ratio for completed cases with best ≥1,000,000 steps | 1.8083× over 14 boards |

Separate serial enumeration completed all eight directions on 173 boards,
found only a best-known minimum on 2, and completed no direction on 3.
Work ratios use completed directions; censored minima are not exact optima.
The completed elapsed sum excludes the timeout. This puzsq sample's decoded
boards and per-board manifest are not bundled, so the corpus summary cannot
be reproduced from this repository alone.

## WebAssembly browser time

CPU: **AMD Ryzen 9 9950X**. Date: **2026-11-05**.
Chrome, ex3, two workers, race mode, cut 25, memo 24 (128 MiB per worker),
diagonal shortcut off. Means of two fresh-navigation observations:

| Wasm module bytes | Browser wall ms | Winner compute ms |
|---:|---:|---:|
| 352,234 | 777.200 | 177.500 |

Wall time includes module loading, direction probes and continuation. Winner
compute time excludes scheduler waits and other workers' work. The browser
page and scheduler used for this timing are not bundled; the ABI can be
checked with `node --test wasm/tests/contract.mjs` after building the module.

## Cube splitting

CPU: **Apple M4 Pro**. Date: **2026-11-05**.
Single observations, direction 0, cut 25, probes disabled. Memo and threads
are ignored by the splitter. These measure JSON cube generation, not a
complete SAT solve.

| Input | Requested / produced cubes | Split ms | Split steps |
|---|---:|---:|---:|
| `reference/ex3.txt` | 64 / 64 | 1.016 | 148 |
| `reference/ex3.txt` | 256 / 256 | 4.037 | 844 |
| `reference/p2592.txt` | 64 / 64 | 1.180 | 156 |
| `reference/p2592.txt` | 256 / 256 | 4.850 | 862 |
| `reference/p7933.txt` | 64 / 64 | 1.358 | 136 |
| `reference/p7933.txt` | 256 / 256 | 4.537 | 692 |
| `reference/p10157.txt` | 64 / 64 | 1.224 | 148 |
| `reference/p10157.txt` | 256 / 256 | 4.010 | 666 |

No solutions were found while splitting these cases. The export format and
counting contract are documented in [README.md](README.md#sat-cube-export).

## Validation

Validation covers **2,000 deterministic random boards × 8 directions** across
pruning/memo settings and thread counts. Solution counts agree for settings
that preserve the same canonical form; memo-off parallel step sums and
race-probe-plus-continuation step sums agree exactly with serial search.
Memo can change steps, and diagonal canonicalization can reduce counts.
Returned paths are independently checked for reciprocal edges, degree,
matching clues, disjoint paths, orphan lines and empty-cell classification.
