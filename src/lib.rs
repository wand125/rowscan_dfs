//! Row-major Numberlink enumeration; see README.md for usage, DESIGN.md for architecture, and RULES.md for the shortcut normal form.
mod io;
mod search;
#[cfg(feature = "browser")]
pub use search::BrowserSearch;
#[cfg(feature = "stats")]
pub mod stats;
pub use search::cubes;
pub use search::{generate_cubes, Cube, CubeSplit};
pub use search::{solve, solve_race, solve_with, CandidateReport, Options, Outcome, RaceOutcome};

/// Immutable, unpadded input. Clue values are dense pair numbers, zero is blank.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Puzzle {
    pub width: usize,
    pub height: usize,
    pub pairs: usize,
    pub cells: Vec<u16>,
}

impl Puzzle {
    pub fn parse(text: &str) -> Result<Self, String> {
        let rows: Vec<_> = text
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if rows.len() < 3 || !rows[0].bytes().all(|b| b == b'-') {
            return Err("expected a dashed top frame, rows, and a dashed bottom frame".into());
        }
        let width = rows[0].len();
        let height = rows.len() - 2;
        if !(1..=64).contains(&width)
            || !(1..=64).contains(&height)
            || rows.last() != Some(&rows[0])
        {
            return Err("invalid frame or dimensions (maximum 64 by 64)".into());
        }
        let mut values = [0u16; 256];
        let mut counts = [0u16; 256];
        let mut pairs = 0;
        let mut cells = Vec::with_capacity(width * height);
        for row in &rows[1..rows.len() - 1] {
            if row.len() != width {
                return Err("inconsistent row width".into());
            }
            for b in row.bytes() {
                if b == b'.' {
                    cells.push(0);
                    continue;
                }
                if !b.is_ascii_alphanumeric() || b == b'0' {
                    return Err(format!("invalid cell character {:?}", b as char));
                }
                if values[b as usize] == 0 {
                    pairs += 1;
                    values[b as usize] = pairs;
                }
                counts[b as usize] += 1;
                cells.push(values[b as usize]);
            }
        }
        if pairs == 0 || counts.iter().any(|&n| n != 0 && n != 2) {
            return Err(
                "each clue character must occur exactly twice, with at least one pair".into(),
            );
        }
        Ok(Self {
            width,
            height,
            pairs: pairs as usize,
            cells,
        })
    }

    pub fn to_text(&self) -> String {
        let symbols = b"123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let mut s = "-".repeat(self.width);
        s.push('\n');
        for row in self.cells.chunks(self.width) {
            for &v in row {
                s.push(if v == 0 {
                    '.'
                } else {
                    symbols[v as usize - 1] as char
                });
            }
            s.push('\n');
        }
        s.push_str(&"-".repeat(self.width));
        s.push('\n');
        s
    }

    /// H1: heuristic. Reflections act in original coordinates before transposition.
    pub fn transformed(&self, direction: u8) -> Self {
        assert!(direction < 8);
        let transpose = direction & 1 != 0;
        let (width, height) = if transpose {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        };
        let mut cells = vec![0; width * height];
        for y in 0..self.height {
            for x in 0..self.width {
                let xx = if direction & 2 != 0 {
                    self.width - 1 - x
                } else {
                    x
                };
                let yy = if direction & 4 != 0 {
                    self.height - 1 - y
                } else {
                    y
                };
                let (tx, ty) = if transpose { (yy, xx) } else { (xx, yy) };
                cells[ty * width + tx] = self.cells[y * self.width + x];
            }
        }
        Self {
            width,
            height,
            pairs: self.pairs,
            cells,
        }
    }

    pub fn recommended_direction(&self) -> u8 {
        // H1: heuristic; the strict improvement keeps the earliest exact tie.
        let mut sx = 0;
        let mut sy = 0;
        for (c, &v) in self.cells.iter().enumerate() {
            if v != 0 {
                sx += c % self.width + 1;
                sy += c / self.width + 1;
            }
        }
        let mut best = (0, 0);
        let mut direction = 0;
        for k in 0..8 {
            let (a, b, sum, flip) = if k & 1 == 0 {
                (self.width, self.height, sy, k & 4 != 0)
            } else {
                (self.height, self.width, sx, k & 2 != 0)
            };
            if a > b
                || (!flip && sum > (b + 1) * self.pairs)
                || (flip && sum < (b + 1) * self.pairs)
            {
                continue;
            }
            let p = self.transformed(k);
            let mut seen = vec![0; self.pairs + 1];
            let (mut total, mut single, mut score, mut tie) = (0usize, 0usize, 0usize, 0usize);
            for &v in &p.cells[..p.width * (p.height / 2)] {
                if v != 0 {
                    total += 1;
                    seen[v as usize] += 1;
                    if seen[v as usize] == 1 {
                        single += 1;
                    } else {
                        single -= 1;
                    }
                }
                score += total;
                tie += single;
            }
            if (score, tie) > best {
                best = (score, tie);
                direction = k;
            }
        }
        direction
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;
    #[test]
    fn input_validation() {
        assert!(Puzzle::parse("---\n1.1\n---\n").is_ok());
        assert!(Puzzle::parse("---\n1..\n---\n").is_err());
        assert!(Puzzle::parse("---\n101\n---\n").is_err());
    }
    #[test]
    fn h1_and_transforms() {
        let p = Puzzle::parse(include_str!("../reference/ex1.txt")).unwrap();
        assert_eq!(p.recommended_direction(), 6);
        let q = Puzzle::parse("---\n1..\n..1\n---\n").unwrap();
        for k in 0..8 {
            let t = q.transformed(k);
            assert_eq!(t.cells.iter().filter(|&&v| v != 0).count(), 2);
        }
        assert_eq!(q.transformed(1).width, 2);
        assert_eq!(q.transformed(6).cells, vec![1, 0, 0, 0, 0, 1]);
    }
}
