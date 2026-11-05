//! Input decoding and answer formatting; no solver state is changed here.
use crate::Puzzle;
use std::collections::BTreeMap;

impl Puzzle {
    /// Decode a puzz.link Numberlink URL, assigning dense pairs in encounter order.
    #[cold]
    pub fn from_url(url: &str) -> Result<Self, String> {
        Self::decode_url(url).map(|(p, _)| p)
    }

    /// Return the puzzle and original numeric labels indexed by dense pair minus one.
    #[cold]
    pub fn decode_url(url: &str) -> Result<(Self, Vec<u16>), String> {
        let encoded = url
            .strip_prefix("p?numlin/")
            .or_else(|| url.strip_prefix("https://puzz.link/p?numlin/"))
            .ok_or("expected a https://puzz.link/p?numlin/ URL or p?numlin/... path")?;
        let fields: Vec<_> = encoded.split('/').collect();
        if fields.len() != 3 {
            return Err("expected width/height/hints".into());
        }
        let dimension = |s: &str| -> Result<usize, String> {
            if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                return Err("dimensions must be integers in 1..64".into());
            }
            let n = s
                .parse()
                .map_err(|_| "dimensions must be integers in 1..64")?;
            if !(1..=64).contains(&n) {
                return Err("dimensions must be in 1..64".into());
            }
            Ok(n)
        };
        let width = dimension(fields[0])?;
        let height = dimension(fields[1])?;
        let hints = fields[2].as_bytes();
        let mut cells = Vec::with_capacity(width * height);
        let mut i = 0;
        while i < hints.len() {
            let b = hints[i];
            i += 1;
            match b {
                b'g'..=b'z' => cells.resize(cells.len() + usize::from(b - b'f'), 0),
                b'-' | b'+' => {
                    let n = if b == b'-' { 2 } else { 3 };
                    let digits = hints.get(i..i + n).ok_or("malformed extended clue")?;
                    if !digits.iter().all(u8::is_ascii_hexdigit) {
                        return Err("malformed extended clue".into());
                    }
                    let value =
                        u16::from_str_radix(std::str::from_utf8(digits).unwrap(), 16).unwrap();
                    if value < 16 {
                        return Err("extended clue must be at least 16".into());
                    }
                    cells.push(value);
                    i += n;
                }
                b'1'..=b'9' => cells.push(u16::from(b - b'0')),
                b'a'..=b'f' => cells.push(u16::from(b - b'a') + 10),
                _ => return Err(format!("invalid hint character at byte {}", i - 1)),
            }
            if cells.len() > width * height {
                return Err("hints exceed board".into());
            }
        }
        if cells.len() != width * height {
            return Err("hints do not fill board".into());
        }
        let mut ids = BTreeMap::new();
        let mut counts = Vec::new();
        let mut values = Vec::new();
        for cell in &mut cells {
            if *cell == 0 {
                continue;
            }
            let next = ids.len() + 1;
            let id = *ids.entry(*cell).or_insert_with(|| {
                values.push(*cell);
                counts.push(0);
                next
            });
            counts[id - 1] += 1;
            *cell = id as u16;
        }
        if values.is_empty() || counts.iter().any(|&n| n != 2) {
            return Err("each clue must occur exactly twice, with at least one pair".into());
        }
        // Pair 255's internal label is the WALL sentinel.
        if values.len() > 254 {
            return Err("at most 254 clue pairs are supported".into());
        }
        Ok((
            Self {
                width,
                height,
                pairs: values.len(),
                cells,
            },
            values,
        ))
    }

    /// The answer.js glyph set, with original clue values right aligned.
    #[cold]
    pub fn answer_text(&self, links: &[u8], values: &[u16]) -> String {
        assert_eq!(links.len(), self.cells.len());
        assert_eq!(values.len(), self.pairs);
        const GLYPHS: [char; 16] = [
            ' ', '│', '─', '└', '│', '│', '┌', '├', '─', '┘', '─', '┴', '┐', '┤', '┬', '┼',
        ];
        let size = values.iter().max().unwrap_or(&0).to_string().len();
        (0..self.height)
            .map(|y| {
                (0..self.width)
                    .map(|x| {
                        let c = y * self.width + x;
                        let text = if self.cells[c] == 0 {
                            GLYPHS[links[c] as usize].to_string()
                        } else {
                            values[self.cells[c] as usize - 1].to_string()
                        };
                        format!("{text:>size$}")
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[cold]
    pub fn answer_json(&self, links: &[u8]) -> String {
        assert_eq!(links.len(), self.cells.len());
        format!(
            "{{\"width\":{},\"height\":{},\"links\":[{}]}}",
            self.width,
            self.height,
            links
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}
