//! Failed frontier states only. No state-write or undo-time hash maintenance.
use super::{Board, CutState, WALL};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

struct Open {
    key: u64,
    row: usize,
    // Diagnostic choice depth. Row retreat governs closure; thieves start empty.
    #[allow(dead_code)]
    depth: usize,
    solutions: u64,
    tainted: bool,
}

pub(super) struct Memo {
    table: Arc<[AtomicU64]>,
    open: Vec<Open>,
    pub hits: u64,
    pub stores: u64,
}

fn mix(h: u64, v: u64) -> u64 {
    (h ^ v).wrapping_mul(0x9E3779B97F4A7C15).rotate_left(29)
}
fn cell(c: usize, label: u16, edge: u8) -> u64 {
    let v = ((c as u64) << 32 | (label as u64) << 8 | edge as u64).wrapping_mul(0x9E3779B97F4A7C15);
    v ^ (v >> 29)
}

// Advice only: preserve allocation, zero initialization and all cache semantics.
// No process-global/kernel settings are changed; other targets use normal pages.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn advise_large_pages(table: &[AtomicU64]) {
    const HUGE: usize = 2 * 1024 * 1024;
    const MADV_HUGEPAGE: i32 = 14;
    let bytes = std::mem::size_of_val(table);
    if bytes < 128 * 1024 * 1024 {
        return;
    }
    let base = table.as_ptr() as usize;
    let start = (base + HUGE - 1) & !(HUGE - 1);
    let end = (base + bytes) & !(HUGE - 1);
    if end <= start {
        return;
    }
    extern "C" {
        fn madvise(addr: *mut std::ffi::c_void, length: usize, advice: i32) -> i32;
    }
    // Both endpoints are inside the live allocation and huge-page aligned;
    // MADV_HUGEPAGE changes mapping policy, not the contents or ownership.
    // It is a best-effort hint: rejection leaves normal pages fully usable.
    unsafe {
        let ptr = table.as_ptr().byte_add(start - base).cast_mut().cast();
        let _ = madvise(ptr, end - start, MADV_HUGEPAGE);
    }
}

impl Memo {
    pub fn new(bits: usize, height: usize) -> Self {
        assert!((1..=28).contains(&bits));
        // Every bit pattern of the inner u64 is valid, including zero.
        // The zeroed allocation avoids eagerly touching 512 MiB on tiny searches.
        let table = unsafe { Arc::<[AtomicU64]>::new_zeroed_slice(1usize << bits).assume_init() };
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        advise_large_pages(&table);
        Self {
            table,
            open: Vec::with_capacity(height),
            hits: 0,
            stores: 0,
        }
    }
    pub fn fork(&self, height: usize) -> Self {
        Self {
            table: Arc::clone(&self.table),
            open: Vec::with_capacity(height),
            hits: 0,
            stores: 0,
        }
    }
    pub fn slots(&self) -> usize {
        self.table.len()
    }

    // Work-stealing donations leave donor ancestors incomplete, so their open
    // memo entries are not stored. Future entries remain eligible; thieves start empty.
    pub fn taint_open(&mut self) {
        for entry in &mut self.open {
            entry.tainted = true;
        }
    }

    #[inline(never)]
    fn key<C: CutState>(b: &Board<C>, row: usize) -> u64 {
        let first = row.saturating_sub(2).max(1);
        let s = b.shared.stride;
        let mut acc = 0u64;
        for y in first..=b.shared.height {
            for c in y * s + 1..=y * s + b.shared.width {
                acc = acc.wrapping_add(cell(c, b.state(c), b.edges(c)));
            }
        }
        for y in 1..first {
            for c in y * s + 1..=y * s + b.shared.width {
                if b.state(c) == 0 {
                    acc = acc.wrapping_add(cell(c, 0x8000, b.edges(c)));
                }
            }
        }
        let mut h = mix(mix(0x1234, b.cursor as u64), acc);
        for x in 1..=b.shared.width {
            let mut y = first - 1;
            while y > 0 {
                let c = y * s + x;
                let label = b.state(c);
                h = mix(h, (label as u64) << 8 | b.edges(c) as u64);
                if label != 0 && label != WALL {
                    break;
                }
                y -= 1;
            }
            if y == 0 {
                h = mix(h, WALL as u64);
            }
        }
        for pair in 1..b.done.len() {
            if b.done[pair] == 0 {
                h = mix(h, b.end_pos[pair] as u64);
                h = mix(h, b.end_pos[pair | 256] as u64);
            }
            h = mix(h, b.done[pair] as u64);
        }
        h | 1
    }
    fn slot(&self, key: u64, row: usize) -> &AtomicU64 {
        &self.table[(key as usize ^ (row << 3)) & (self.table.len() - 1)]
    }
    fn close(&mut self, entry: Open, solutions: u64) {
        if !entry.tainted && entry.solutions == solutions {
            self.slot(entry.key, entry.row)
                .store(entry.key, Ordering::Relaxed);
            self.stores += 1;
        }
    }
    pub fn finish(&mut self, solutions: u64) {
        while let Some(entry) = self.open.pop() {
            self.close(entry, solutions);
        }
    }
    #[inline(always)]
    pub fn observe<C: CutState>(&mut self, b: &Board<C>, solutions: u64, good: bool) -> bool {
        let row = b.cursor as usize / b.shared.stride;
        while self.open.last().is_some_and(|entry| entry.row > row) {
            let entry = self.open.pop().unwrap();
            self.close(entry, solutions);
        }
        if !good
            || !(1..=b.shared.height).contains(&row)
            || self.open.last().is_some_and(|entry| entry.row >= row)
        {
            return false;
        }
        let key = Self::key(b, row);
        if self.slot(key, row).load(Ordering::Relaxed) == key {
            self.hits += 1;
            return true;
        }
        self.open.push(Open {
            key,
            row,
            depth: b.choices.len(),
            solutions,
            tainted: false,
        });
        false
    }
}

#[cfg(all(test, debug_assertions))]
thread_local! {
    pub(super) static CHECK_HITS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(super) static CHECKED_HITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Puzzle;
    fn board() -> Board {
        Board::new(
            &Puzzle::parse("-----\n1...1\n.....\n.....\n.....\n.....\n.....\n.....\n-----\n")
                .unwrap(),
        )
    }
    #[test]
    fn key_visibility() {
        let mut b = board();
        let s = b.shared.stride;
        b.cursor = (7 * s + 1) as u16;
        // Stop every upward ray on row 3, leaving earlier labelled interiors hidden.
        for x in 1..=b.shared.width {
            b.label[3 * s + x] = 0x201;
        }
        b.label[s + 2] = 0;
        b.label[s + 3] = 0x201;
        let key = Memo::key(&b, 7);
        b.edge[s + 2] ^= 2;
        assert_ne!(Memo::key(&b, 7), key, "far anonymous edge");
        b.edge[s + 2] ^= 2;
        b.label[3 * s + 2] ^= 1;
        assert_ne!(Memo::key(&b, 7), key, "first labelled cell on ray");
        b.label[3 * s + 2] ^= 1;
        b.end_pos[1] += 1;
        assert_ne!(Memo::key(&b, 7), key, "incomplete end");
        b.end_pos[1] -= 1;
        b.edge[s + 3] ^= 2;
        b.label[s + 3] ^= 1;
        assert_eq!(Memo::key(&b, 7), key, "hidden labelled interior");
        b.edge[6 * s + 2] ^= 2;
        assert_ne!(Memo::key(&b, 7), key, "visible band");
    }
    #[test]
    fn failed_only_collision_and_taint() {
        let mut b = board();
        let mut memo = Memo::new(1, 7);
        assert!(!memo.observe(&b, 0, true));
        memo.finish(1);
        assert_eq!(memo.stores, 0, "successful subtree");
        assert!(!memo.observe(&b, 1, true));
        memo.taint_open();
        memo.finish(1);
        assert_eq!(memo.stores, 0, "donated subtree");
        assert!(!memo.observe(&b, 1, true));
        memo.finish(1);
        assert!(memo.observe(&b, 1, true));
        assert!(memo.open.is_empty());
        let key = Memo::key(&b, 1);
        memo.slot(key, 1)
            .store(key.wrapping_add(2), Ordering::Relaxed);
        assert!(!memo.observe(&b, 1, true), "index collision is not a hit");
        // Retreat closes deeper rows even when propagation failed.
        b.choices.push(super::super::Choice {
            trail_len: 0,
            cell: b.cursor,
            zero: true,
            taken: 0,
        });
        b.cursor = (2 * b.shared.stride + 1) as u16;
        memo.observe(&b, 1, true);
        b.cursor = (b.shared.stride + 1) as u16;
        memo.observe(&b, 1, false);
        assert_eq!(memo.open.len(), 1);
        memo.finish(1);
        assert_eq!(memo.stores, 3);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn debug_hit_soundness_small_random_boards() {
        CHECK_HITS.with(|v| v.set(true));
        CHECKED_HITS.with(|v| v.set(0));
        let mut hits = 0;
        let mut searched = 0;
        let mut failed_root = None;
        for seed in 0..400u64 {
            let mut state = seed;
            let mut next = || {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (state >> 32) as usize
            };
            let width = 5 + next() % 3;
            let height = 5 + next() % 3;
            let pairs = 2 + next() % 3;
            let mut slots: Vec<_> = (0..width * height).collect();
            for i in (1..slots.len()).rev() {
                slots.swap(i, next() % (i + 1));
            }
            let mut cells = vec![0; width * height];
            for (i, &c) in slots[..pairs * 2].iter().enumerate() {
                cells[c] = (i / 2 + 1) as u16;
            }
            let p = Puzzle {
                width,
                height,
                pairs,
                cells,
            };
            for k in 0..8 {
                let off = super::super::solve_none(&p, k);
                if failed_root.is_none() && off.solutions == 0 && off.steps > 0 {
                    let mut root = Board::<super::super::NoCut>::new(&p.transformed(k));
                    for y in 1..=root.shared.height {
                        for x in 1..=root.shared.width {
                            let c = y * root.shared.stride + x;
                            if super::super::active(root.state(c)) {
                                root.endpoint(c);
                            }
                        }
                    }
                    if root.propagate() {
                        failed_root = Some(root);
                    }
                }
                let result = super::super::solve_with(
                    &p,
                    k,
                    &super::super::Options {
                        memo_bits: 12,
                        ..super::super::Options::none()
                    },
                );
                assert_eq!(result.solutions, off.solutions, "seed {seed} d{k}");
                assert!(result.steps <= off.steps);
                hits += result.memo_hits;
                searched += u64::from(off.steps > 0);
            }
        }
        // Random boards rarely revisit a frontier. This small, valid failed
        // subtree ensures the per-hit re-solver is exercised even for zero hits.
        let b = failed_root.expect("a failed random root that requires search");
        let mut memo = Memo::new(8, b.shared.height);
        assert!(!memo.observe(&b, 0, true));
        memo.finish(0);
        assert!(memo.observe(&b, 0, true));
        b.assert_failed_subtree::<false>();
        let checked = CHECKED_HITS.with(|v| v.get());
        assert_eq!(checked, hits + 1);
        assert!(searched > 100);
        CHECK_HITS.with(|v| v.set(false));
        eprintln!("debug memo: 400 boards × 8 directions, {searched} searched, {hits} natural hits; {checked} hit subtrees independently re-solved");
    }
}
