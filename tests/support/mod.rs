use rowscan_dfs::Puzzle;
pub fn random_puzzle(seed: u64) -> Puzzle {
    let mut rng = seed;
    let mut next = || {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (rng >> 32) as usize
    };
    let width = 6 + next() % 4;
    let height = 6 + next() % 4;
    let pairs = 2 + next() % 5;
    let mut slots: Vec<_> = (0..width * height).collect();
    for i in (1..slots.len()).rev() {
        slots.swap(i, next() % (i + 1));
    }
    let mut cells = vec![0; width * height];
    for (i, &slot) in slots[..pairs * 2].iter().enumerate() {
        cells[slot] = (i / 2 + 1) as u16;
    }
    Puzzle {
        width,
        height,
        pairs,
        cells,
    }
}
pub fn validate_solution(p: &Puzzle, edges: &[u8]) {
    assert_eq!(edges.len(), p.cells.len());
    let mut seen = vec![false; edges.len()];
    let neighbors = |c: usize, d: usize| -> Option<usize> {
        match d {
            0 => c.checked_sub(p.width),
            1 => (c % p.width + 1 < p.width).then_some(c + 1),
            2 => (c + p.width < edges.len()).then_some(c + p.width),
            3 => (!c.is_multiple_of(p.width)).then(|| c - 1),
            _ => None,
        }
    };
    for (c, &e) in edges.iter().enumerate() {
        assert_eq!(e & 0xf0, 0);
        if p.cells[c] != 0 {
            assert_eq!(e.count_ones(), 1);
        } else {
            assert!(e == 0 || e.count_ones() == 2);
        }
        for d in 0..4 {
            if e & (1 << d) != 0 {
                let n = neighbors(c, d).expect("line exits board");
                assert_ne!(edges[n] & (1 << ((d + 2) % 4)), 0);
            }
        }
    }
    for start in 0..edges.len() {
        if p.cells[start] == 0 || seen[start] {
            continue;
        }
        let (mut c, mut previous) = (start, usize::MAX);
        loop {
            assert!(!seen[c], "loop or intersecting path");
            seen[c] = true;
            if c != start && p.cells[c] != 0 {
                assert_eq!(p.cells[c], p.cells[start]);
                break;
            }
            let next = (0..4)
                .filter(|&d| edges[c] & (1 << d) != 0)
                .filter_map(|d| neighbors(c, d))
                .find(|&n| n != previous)
                .expect("path dead end");
            previous = c;
            c = next;
        }
    }
    for (c, &e) in edges.iter().enumerate() {
        assert!(e == 0 || seen[c], "orphan anonymous segment");
    }
}
