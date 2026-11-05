//! Cold G1/G2 paths for URL inputs with more pairs than the 64-bit fast masks.
use super::*;

#[derive(Clone)]
pub(super) struct Many<C: CutState>(C);
impl<C: CutState> CutState for Many<C> {
    const ENABLED: bool = C::ENABLED;
    const WIDE: bool = true;
    fn new(p: &Puzzle) -> Self {
        Self(C::new(p))
    }
    fn data(&self) -> &CutCapacity {
        self.0.data()
    }
    fn data_mut(&mut self) -> &mut CutCapacity {
        self.0.data_mut()
    }
}
impl<C: CutState> Board<C> {
    #[cold]
    pub(super) fn regions_wide(&mut self) -> bool {
        if self.remaining < 2 {
            return true;
        }
        self.boundary_epoch = self.boundary_epoch.wrapping_add(1);
        if self.boundary_epoch == 0 {
            self.boundary_cell_seen.fill(0);
            self.boundary_epoch = 1;
        }
        // Dirty buckets alias modulo 64; the per-end flags remain exact.
        let todo = std::mem::take(&mut self.dirty_pairs);
        for pair in 1..self.done.len() {
            if todo & (1u64 << (pair & 63)) == 0 {
                continue;
            }
            for id in [pair, pair | 256] {
                let c = self.end_pos[id] as usize;
                let flags = self.dirty[id];
                self.dirty[id] = 0;
                for d in 0..4 {
                    if flags & bit(d) == 0 {
                        continue;
                    }
                    let q = self.neighbor(c, d);
                    let eligible = if self.done[pair] != 0 {
                        self.state(q) == 0
                    } else {
                        self.free(c) & bit(d) != 0
                    };
                    if eligible && self.state(q) == 0 && !self.boundary(q, opposite(d)) {
                        self.fail_site(16);
                        return false;
                    }
                }
            }
        }
        true
    }

    #[cold]
    pub(super) fn contacts_wide(&mut self) -> bool {
        // G2: D. Same XOR-exit and alternating-contour semantics as the fast path.
        let mut exits = [0u8; 256];
        let mut seen = [0u8; 256];
        for &(id, dir) in &self.contacts {
            let pair = (id & 255) as usize;
            exits[pair] ^= dir << ((id >> 6) & 4);
            seen[pair] |= if id & 256 == 0 { 1 } else { 2 };
        }
        let mut candidates = [0u16; 254];
        let mut count = 0;
        for pair in 1..self.done.len() {
            if seen[pair] == 3
                && exits[pair] & 15 == self.free(self.end_pos[pair] as usize)
                && exits[pair] >> 4 == self.free(self.end_pos[pair | 256] as usize)
            {
                candidates[count] = pair as u16;
                count += 1;
            }
        }
        for (i, &a) in candidates[..count].iter().enumerate() {
            for &b in &candidates[i + 1..count] {
                let (mut first, mut last, mut transitions) = (0u16, 0u16, 0);
                let mut abandon = false;
                for &(id, _) in &self.contacts {
                    let pair = id & 255;
                    if pair != a && pair != b {
                        continue;
                    }
                    if first == 0 {
                        first = id;
                        last = id;
                    } else if last != id {
                        transitions += 1;
                        if last == (id ^ 256) || transitions > 4 {
                            abandon = true;
                            break;
                        }
                        last = id;
                    }
                }
                if !abandon && last != (first ^ 256) && (last == first || transitions < 4) {
                    self.fail_site(17);
                    return false;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn high_pair_ids_preserve_alternating_contour_and_escape() {
        fn check<C: CutState>(escape: bool) {
            let wide = C::WIDE;
            let p = Puzzle::parse("---\n.1.\n2.2\n.1.\n---\n").unwrap();
            let mut b = Board::<C>::new(&p);
            let c = 2 * b.shared.stride + 2;
            if wide {
                b.done.resize(67, 1);
                b.done.fill(1);
                b.dirty.fill(0);
                for pair in 1..=2 {
                    b.done[pair + 64] = 0;
                    for id in [pair, pair | 256] {
                        let cell = b.end_pos[id] as usize;
                        b.label[cell] = (id + 64) as u16;
                        b.end_pos[id + 64] = cell as u16;
                        b.dirty[id + 64] = 15;
                    }
                }
                b.dirty_pairs = u64::MAX;
            }
            for d in usize::from(escape)..4 {
                let q = b.neighbor(c, d);
                for t in 0..4 {
                    if b.neighbor(q, t) != c {
                        b.set_noline(q, t);
                    }
                }
            }
            assert_eq!(b.regions(), escape, "wide={wide}");
        }
        for escape in [false, true] {
            check::<NoCut>(escape);
            check::<Many<NoCut>>(escape);
        }
    }
}
