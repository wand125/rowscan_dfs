//! Optional, nested rule attribution. Counts include work that is later undone.
use std::fmt;
pub(crate) const R_RULE: usize = 0;
pub(crate) const UNDO_RULE: usize = 1;
pub(crate) const D1_RULE: usize = 2;
pub(crate) const D2_RULE: usize = 3;
pub(crate) const C_RULE: usize = 4;
pub(crate) const E_RULE: usize = 20;
pub(crate) const S_RULE: usize = 21;
pub(crate) const K_RULE: usize = 25;
pub(crate) const G2_RULE: usize = 27;
pub(crate) const W_RULE: usize = 28;
pub(crate) const D4_RULE: usize = 29;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub contradictions: u64,
    pub lines: u64,
    pub walls: u64,
    /// Entry calls, including calls whose predicates produce no change.
    pub evaluations: u64,
}
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub rows: [Counts; 30],
    pub failures: [[u64; 32]; 30],
    pub label_writes: u64,
    pub label_noops: u64,
    pub internal_relabels: u64,
    pub walk_calls: u64,
    pub boundary_walks: u64,
    /// Contour cell-side visits; a cell may have several boundary sides.
    pub contour_cells: u64,
    pub contour_unique_cells: u64,
    pub end_rules_calls: u64,
    pub segment_calls: u64,
    pub worklist_pops: u64,
    pub trail_entries: u64,
    pub trail_peak: u64,
    pub capacity_updates: u64,
    pub capacity_trail_entries: u64,
    pub cut_checks: u64,
    pub cut_fail_x: u64,
    pub cut_fail_y: u64,
    pub(crate) current: usize,
    pub(crate) choice_edge: bool,
}
impl Stats {
    pub(crate) fn merge(&mut self, other: &Self) {
        for (a, b) in self.rows.iter_mut().zip(&other.rows) {
            a.contradictions += b.contradictions;
            a.lines += b.lines;
            a.walls += b.walls;
            a.evaluations += b.evaluations;
        }
        for (a, b) in self
            .failures
            .iter_mut()
            .flatten()
            .zip(other.failures.iter().flatten())
        {
            *a += b;
        }
        self.label_writes += other.label_writes;
        self.label_noops += other.label_noops;
        self.internal_relabels += other.internal_relabels;
        self.walk_calls += other.walk_calls;
        self.boundary_walks += other.boundary_walks;
        self.contour_cells += other.contour_cells;
        self.contour_unique_cells += other.contour_unique_cells;
        self.end_rules_calls += other.end_rules_calls;
        self.segment_calls += other.segment_calls;
        self.worklist_pops += other.worklist_pops;
        self.trail_entries += other.trail_entries;
        self.capacity_updates += other.capacity_updates;
        self.capacity_trail_entries += other.capacity_trail_entries;
        self.cut_checks += other.cut_checks;
        self.cut_fail_x += other.cut_fail_x;
        self.cut_fail_y += other.cut_fail_y;
        self.trail_peak = self.trail_peak.max(other.trail_peak);
    }
    pub(crate) fn enter(&mut self, rule: usize) -> usize {
        let previous = self.current;
        self.current = rule;
        self.rows[rule].evaluations += 1;
        previous
    }
}
impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "stats,rule,mask,C,L,W,evaluations")?;
        for (i, c) in self.rows.iter().enumerate() {
            let (name, mask) = match i {
                R_RULE => ("R", "-".into()),
                UNDO_RULE => ("undo", "-".into()),
                D1_RULE => ("D1", "-".into()),
                D2_RULE => ("D2", "-".into()),
                C_RULE..=19 => ("C", format!("0x{:02x}", (i - C_RULE) << 4)),
                E_RULE => ("E", "-".into()),
                S_RULE..=24 => ("S", ["U", "R", "D", "L"][i - S_RULE].into()),
                K_RULE..=26 => ("K", ["RD", "DL"][i - K_RULE].into()),
                G2_RULE => ("G2", "-".into()),
                W_RULE => ("W1/loop", "-".into()),
                _ => ("D4", "-".into()),
            };
            writeln!(
                f,
                "stats,{name},{mask},{},{},{},{}",
                c.contradictions, c.lines, c.walls, c.evaluations
            )?;
        }
        let walk_c: u64 = self
            .failures
            .iter()
            .map(|sites| sites[4..=9].iter().sum::<u64>())
            .sum();
        writeln!(f, "detail,W1-W3/loop,-,{walk_c},0,0,{}", self.walk_calls)?;
        for (rule, sites) in self.failures.iter().enumerate() {
            for (site, count) in sites.iter().enumerate() {
                if *count != 0 {
                    writeln!(f, "failure,{rule},{site},{count}")?;
                }
            }
        }
        for (name, n) in [
            ("label_writes", self.label_writes),
            ("label_noops", self.label_noops),
            ("internal_relabels", self.internal_relabels),
            ("walk_calls", self.walk_calls),
            ("boundary_walks", self.boundary_walks),
            ("contour_cells", self.contour_cells),
            ("contour_unique_cells", self.contour_unique_cells),
            ("end_rules_calls", self.end_rules_calls),
            ("segment_calls", self.segment_calls),
            ("worklist_pops", self.worklist_pops),
            ("trail_entries", self.trail_entries),
            ("trail_peak", self.trail_peak),
            ("capacity_updates", self.capacity_updates),
            ("capacity_trail_entries", self.capacity_trail_entries),
            ("cut_checks", self.cut_checks),
            ("cut_fail_x", self.cut_fail_x),
            ("cut_fail_y", self.cut_fail_y),
        ] {
            writeln!(f, "counter,{name},{n}")?;
        }
        Ok(())
    }
}
