#[cfg(feature = "stats")]
use crate::stats::{
    Stats, C_RULE, D1_RULE, D2_RULE, D4_RULE, E_RULE, G2_RULE, K_RULE, R_RULE, S_RULE, UNDO_RULE,
    W_RULE,
};
use crate::Puzzle;
#[path = "memo.rs"]
mod memo;
use memo::Memo;
use std::sync::Arc;

/// Search settings. Defaults enable cut 25 and a 512 MiB failed-frontier table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Check at zero-based scan columns 0, N, 2N, ...; zero disables cut checks.
    pub cut: usize,
    /// Count the stricter diagonal-shortcut normal form.
    pub diag_shortcut: bool,
    /// Log2 of the number of 64-bit memo slots; zero disables memo (maximum 28).
    pub memo_bits: usize,
    /// Worker count; one retains the serial search and output (maximum 256).
    pub threads: usize,
    /// Write work-stealing events to stderr.
    pub log: bool,
}

impl Options {
    pub const fn none() -> Self {
        Self {
            cut: 0,
            diag_shortcut: false,
            memo_bits: 0,
            threads: 1,
            log: false,
        }
    }
}
impl Default for Options {
    fn default() -> Self {
        Self {
            cut: 25,
            diag_shortcut: false,
            memo_bits: 26,
            threads: 1,
            log: false,
        }
    }
}

const WALL: u16 = 0x3ff;
const INSIDE: u16 = 0x200;
const U: usize = 0;
const R: usize = 1;
const D: usize = 2;
const L: usize = 3;
const EDGE: u16 = 0;
const LABEL: u16 = 1;
const END: u16 = 2;
const DONE: u16 = 3;
const CURSOR: u16 = 4;
const MARK: u16 = 5;
const LINE_EDGE: u16 = 6;
const WALL_EDGE: u16 = 7;
fn bit(d: usize) -> u8 {
    1 << d
}
fn opposite(d: usize) -> usize {
    (d + 2) & 3
}
fn count4(mask: u8) -> u32 {
    debug_assert!(mask < 16);
    // Nibble popcounts, packed to avoid scalar-to-SIMD round trips on arm64.
    ((0x4332_3221_3221_2110u64 >> (mask * 4)) & 15) as u32
}
fn active(v: u16) -> bool {
    v != 0 && v & INSIDE == 0
}

// With stats disabled the expansion is exactly the rule body. The closure
// confines early returns in instrumented builds so caller attribution restores.
macro_rules! stats_rule {
    ($rule:expr; $(#[$attr:meta])* fn $name:ident(&mut $this:ident $(, $arg:ident: $ty:ty)*) $body:block) => {
        $(#[$attr])*
        fn $name(&mut $this $(, $arg: $ty)*) {
            #[cfg(feature = "stats")]
            {
                let previous = $this.stats.enter($rule);
                #[allow(clippy::redundant_closure_call)]
                (|| $body)();
                $this.stats.current = previous;
            }
            #[cfg(not(feature = "stats"))]
            $body
        }
    };
    ($rule:expr; $(#[$attr:meta])* fn $name:ident(&mut $this:ident $(, $arg:ident: $ty:ty)*) -> bool $body:block) => {
        $(#[$attr])*
        fn $name(&mut $this $(, $arg: $ty)*) -> bool {
            #[cfg(feature = "stats")]
            {
                let previous = $this.stats.enter($rule);
                #[allow(clippy::redundant_closure_call)]
                let result = (|| $body)();
                $this.stats.current = previous;
                result
            }
            #[cfg(not(feature = "stats"))]
            $body
        }
    };
}

// Expand the disabled case directly to the propagation call. Keeping an
// extra generic wrapper here changes LLVM's hot-path inlining decisions.
macro_rules! propagate_optional {
    ($board:ident, $diag:ident) => {{
        let good = $board.propagate();
        if $diag && good && !$board.diagonal_shortcut_ok() {
            $board.fail();
            false
        } else {
            good
        }
    }};
}

// One search body keeps branch ordering identical in the plain and memo loops.
// Keep the plain signature and local result layout stable for hot-loop code generation.
macro_rules! search_loop {
    ($self:ident, $out:ident, $DIAG:ident, $good:ident, $original:ident,
     $observe:block, $retreat:block, $finish:block) => {{
        search_loop!(
            $self, $out, $DIAG, $good, $original,
            {
                for y in 1..=$self.shared.height {
                    for x in 1..=$self.shared.width {
                        let c = y * $self.shared.stride + x;
                        if active($self.state(c)) {
                            $self.endpoint(c);
                        }
                    }
                }
                $good = propagate_optional!($self, $DIAG);
            },
            {}, {}, $observe, $retreat, $finish
        )
    }};
    ($self:ident, $out:ident, $DIAG:ident, $good:ident, $original:ident,
     $start:block, $at_floor:block, $publish:block,
     $observe:block, $retreat:block, $finish:block) => {{
        search_loop!(
            $self, $out, $DIAG, $good, $original,
            $start, $at_floor, $publish, $observe, $retreat, $finish, {},
            {
                $out.steps = $self.steps;
                $out.contradictions = $self.contradictions;
                #[cfg(feature = "stats")]
                { $out.stats = $self.stats; }
                return $out;
            }, {}
        )
    }};
    ($self:ident, $out:ident, $DIAG:ident, $good:ident, $original:ident,
     $start:block, $at_floor:block, $publish:block,
     $observe:block, $retreat:block, $finish:block, $pause:block, $complete:block, $pause_alt:block) => {{
        let mut $good: bool;
        $start
        loop {
            $observe
            $pause
            if $good {
                if $self.remaining == 0 {
                    // D4: D. Anonymous lines invalidate otherwise complete pairs.
                    #[cfg(feature = "stats")]
                    {
                        $self.stats.rows[D4_RULE].evaluations += 1;
                    }
                    let orphan = $self.has_anonymous_lines();
                    if !orphan {
                        $out.solutions += 1;
                        if $out.first_solution.is_none() {
                            $out.first_solution_has_empty =
                                Some((1..=$self.shared.height).any(|y| {
                                    (1..=$self.shared.width)
                                        .any(|x| $self.edges(y * $self.shared.stride + x) & 15 == 0)
                                }));
                            $out.first_solution = Some($self.original_solution($out.direction, $original));
                            $publish
                        }
                    } else {
                        $self.reject_solution();
                        $self.contradictions += 1;
                    }
                } else if let Some((c, zero)) = $self.next_cell() {
                    let checkpoint = $self.trail.len();
                    $self.log(MARK, 0, 0);
                    $self.choices.push(Choice {
                        trail_len: checkpoint,
                        cell: c as u16,
                        zero,
                        taken: 0,
                    });
                    $self.choose(c, zero, false);
                    $good = propagate_optional!($self, $DIAG);
                    continue;
                } else if C::ENABLED && !$self.consistent {
                    $self.contradictions += 1;
                } else {
                    #[cfg(feature = "stats")]
                    {
                        $self.stats.rows[D4_RULE].evaluations += 1;
                    }
                    $self.reject_solution();
                    $self.contradictions += 1;
                }
            } else {
                $self.contradictions += 1;
            }
            loop {
                $at_floor
                let Some(frame) = $self.choices.last() else {
                    $finish
                    $complete
                };
                let (checkpoint, c, zero, taken) = (
                    frame.trail_len,
                    frame.cell as usize,
                    frame.zero,
                    frame.taken,
                );
                $retreat
                $pause_alt
                $self.rollback_to_impl::<false>(checkpoint);
                if taken == 0 {
                    // This checkpoint was taken after a successful G1/G2
                    // fixpoint. Restoring it reinstates that checked geometry;
                    // only the alternative's new events require fresh seeds.
                    $self.dirty.fill(0);
                    $self.dirty_pairs = 0;
                    $self.choices.last_mut().unwrap().taken = 1;
                    $self.log(MARK, 0, 0);
                    $self.choose(c, zero, true);
                    $good = propagate_optional!($self, $DIAG);
                    break;
                }
                $self.choices.pop();
            }
        }
    }};
}

#[path = "cubes.rs"]
pub mod cubes;
#[path = "parallel.rs"]
mod parallel;
#[path = "race.rs"]
mod race;
#[path = "wide.rs"]
mod wide;
pub use cubes::{
    estimate_cube_progress, generate_cubes, solve_cube_budget, Cube, CubeOutcome, CubeProgress, CubeSplit,
};
#[cfg(feature = "browser")]
pub use race::browser::BrowserSearch;
pub use race::{solve_race, CandidateReport, RaceOutcome};

#[derive(Clone)]
struct Shared {
    width: usize,
    height: usize,
    stride: usize,
    offsets: [isize; 4],
    clue: Vec<u16>,
    wedge: Vec<[u16; 4]>,
    southeast: Vec<bool>,
    southwest: Vec<bool>,
}

trait CutState: Clone {
    const ENABLED: bool;
    const WIDE: bool = false;
    fn new(p: &Puzzle) -> Self;
    fn data(&self) -> &CutCapacity;
    fn data_mut(&mut self) -> &mut CutCapacity;
}

// Zero-sized storage adds no cut-check fields to boards with cut checks disabled.
#[derive(Clone)]
struct NoCut;
impl CutState for NoCut {
    const ENABLED: bool = false;
    fn new(_: &Puzzle) -> Self {
        Self
    }
    fn data(&self) -> &CutCapacity {
        unreachable!("cut is disabled")
    }
    fn data_mut(&mut self) -> &mut CutCapacity {
        unreachable!("cut is disabled")
    }
}

#[derive(Clone)]
struct CutCapacity {
    interval: usize,
}
impl CutState for CutCapacity {
    const ENABLED: bool = true;
    fn new(_: &Puzzle) -> Self {
        Self { interval: 0 }
    }
    fn data(&self) -> &Self {
        self
    }
    fn data_mut(&mut self) -> &mut Self {
        self
    }
}

/// Mutable state is worker-owned; immutable tables are shared across donations.
#[derive(Clone)]
struct Board<C: CutState = NoCut> {
    shared: Arc<Shared>,
    offsets: [isize; 4],
    edge: Vec<u8>,
    label: Vec<u16>,
    end_pos: [u16; 512],
    done: Vec<u8>,
    dirty: [u8; 512],
    dirty_pairs: u64,
    remaining: usize,
    queue: Vec<u16>,
    in_queue: Vec<bool>,
    queue_head: usize,
    trail: Vec<u32>,
    choices: Vec<Choice>,
    cursor: u16,
    consistent: bool,
    boundary_cell_seen: Vec<u32>,
    boundary_epoch: u32,
    contacts: Vec<(u16, u8)>,
    steps: u64,
    contradictions: u64,
    pruning: C,
    #[cfg(feature = "stats")]
    stats: Stats,
}

#[derive(Clone)]
struct Choice {
    trail_len: usize,
    cell: u16,
    zero: bool,
    taken: u8,
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub direction: u8,
    pub steps: u64,
    pub contradictions: u64,
    pub memo_hits: u64,
    pub memo_stores: u64,
    pub memo_slots: usize,
    /// Configured workers (one in serial results).
    pub threads: usize,
    pub steals: u64,
    /// Sum of worker busy wall seconds, excluding idle waits; zero for the uninstrumented serial path.
    pub cpu: f64,
    /// Sum of complete board clone calls, in seconds.
    pub clone_seconds: f64,
    pub solutions: u64,
    pub first_solution_has_empty: Option<bool>,
    /// Line masks in the ORIGINAL input orientation, one byte per interior cell.
    pub first_solution: Option<Vec<u8>>,
    #[cfg(feature = "stats")]
    pub stats: Stats,
}

#[derive(Default)]
struct SearchOutcome {
    pub direction: u8,
    pub steps: u64,
    pub contradictions: u64,
    pub solutions: u64,
    pub first_solution_has_empty: Option<bool>,
    /// Line masks in the ORIGINAL input orientation, one byte per interior cell.
    pub first_solution: Option<Vec<u8>>,
    #[cfg(feature = "stats")]
    pub stats: Stats,
}

impl From<SearchOutcome> for Outcome {
    fn from(out: SearchOutcome) -> Self {
        Self {
            direction: out.direction,
            threads: 1,
            steps: out.steps,
            contradictions: out.contradictions,
            solutions: out.solutions,
            first_solution_has_empty: out.first_solution_has_empty,
            first_solution: out.first_solution,
            #[cfg(feature = "stats")]
            stats: out.stats,
            ..Self::default()
        }
    }
}

impl Shared {
    fn new(p: &Puzzle) -> Self {
        let stride = p.width + 2;
        let n = stride * (p.height + 2);
        let mut s = Self {
            width: p.width,
            height: p.height,
            stride,
            offsets: [-(stride as isize), 1, stride as isize, -1],
            clue: vec![WALL; n],
            wedge: vec![[0; 4]; n],
            southeast: vec![false; n],
            southwest: vec![false; n],
        };
        for y in 1..=p.height {
            for x in 1..=p.width {
                s.clue[y * stride + x] = p.cells[(y - 1) * p.width + x - 1];
            }
        }
        // Precompute corner-rule coordinates, preserving the slot-8 exterior asymmetry.
        for y in 1..=p.height {
            for x in 1..=p.width {
                let c = y * stride + x;
                s.wedge[c] = [
                    (y * stride + p.width + 1) as u16,
                    ((p.height + 1) * stride + x) as u16,
                    (y * stride + p.width + 1) as u16,
                    ((p.height + 1) * stride + x) as u16,
                ];
                if s.clue[c] != 0 {
                    continue;
                }
                for t in 0..4 {
                    let max_k = if t == U || t == D {
                        p.width - x
                    } else {
                        p.height - y
                    };
                    'candidates: for k in 2..=max_k {
                        for j in 1..=k / 2 {
                            let sample = match t {
                                U => {
                                    if y + j > p.height {
                                        break;
                                    }
                                    (y + j) * stride + x + k - j
                                }
                                D => {
                                    if j >= y {
                                        break;
                                    }
                                    (y - j) * stride + x + k - j
                                }
                                R => {
                                    if j >= x {
                                        break;
                                    }
                                    (y + k - j) * stride + x - j
                                }
                                L => (y + k - j) * stride + (x + j).min(p.width + 1),
                                _ => unreachable!(),
                            };
                            if s.clue[sample] != 0 {
                                s.wedge[c][t] = if t == U || t == D {
                                    (y * stride + x + k) as u16
                                } else {
                                    ((y + k) * stride + x) as u16
                                };
                                break 'candidates;
                            }
                        }
                    }
                }
            }
        }
        // Unequal intervening clues stop diagonal corner-rule permission.
        for y in 1..=p.height {
            for x in 1..=p.width {
                if s.clue[y * stride + x] == 0 {
                    continue;
                }
                let (mut yy, mut xx) = (y as isize - 1, x as isize - 1);
                while yy > 0 && xx > 0 {
                    let q = yy as usize * stride + xx as usize;
                    if s.clue[q] != 0 {
                        break;
                    }
                    let (a, b) = (s.clue[q + 1], s.clue[q + stride]);
                    if a != 0 && b != 0 && a != b {
                        break;
                    }
                    s.southeast[q] = true;
                    yy -= 1;
                    xx -= 1;
                }
                let (mut yy, mut xx) = (y as isize - 1, x + 1);
                while yy > 0 && xx <= p.width {
                    let q = yy as usize * stride + xx;
                    if s.clue[q] != 0 {
                        break;
                    }
                    let (a, b) = (s.clue[q - 1], s.clue[q + stride]);
                    if a != 0 && b != 0 && a != b {
                        break;
                    }
                    s.southwest[q] = true;
                    yy -= 1;
                    xx += 1;
                }
            }
        }
        s
    }
}

impl<C: CutState> Board<C> {
    fn new(p: &Puzzle) -> Self {
        let shared = Arc::new(Shared::new(p));
        let n = shared.clue.len();
        let mut b = Self {
            offsets: shared.offsets,
            shared,
            edge: vec![0xf0; n],
            label: vec![WALL; n],
            end_pos: [0; 512],
            done: vec![0; p.pairs + 1],
            dirty: [15; 512],
            dirty_pairs: if p.pairs < 63 {
                (1u64 << (p.pairs + 1)) - 2
            } else if p.pairs == 63 {
                u64::MAX - 1
            } else {
                u64::MAX
            },
            remaining: p.pairs,
            queue: Vec::with_capacity(n * 32),
            in_queue: vec![false; n],
            queue_head: 0,
            trail: Vec::with_capacity(n * 128),
            choices: Vec::with_capacity(n),
            cursor: 0,
            consistent: true,
            boundary_cell_seen: vec![0; n],
            boundary_epoch: 0,
            contacts: Vec::with_capacity(n * 4),
            steps: 0,
            contradictions: 0,
            pruning: C::new(p),
            #[cfg(feature = "stats")]
            stats: Stats::default(),
        };
        let mut seen = [false; 256];
        for y in 1..=p.height {
            for x in 1..=p.width {
                let c = y * b.shared.stride + x;
                b.edge[c] = 0;
                b.label[c] = 0;
                if y == 1 {
                    b.edge[c] |= 0x10;
                }
                if x == p.width {
                    b.edge[c] |= 0x20;
                }
                if y == p.height {
                    b.edge[c] |= 0x40;
                }
                if x == 1 {
                    b.edge[c] |= 0x80;
                }
                let v = b.shared.clue[c];
                if v != 0 {
                    let id = v | if seen[v as usize] { 256 } else { 0 };
                    seen[v as usize] = true;
                    b.label[c] = id;
                    b.end_pos[id as usize] = c as u16;
                }
                b.enqueue(c);
            }
        }
        b.cursor = (b.shared.stride + 1) as u16;
        b
    }
    #[inline(always)]
    fn neighbor(&self, c: usize, d: usize) -> usize {
        debug_assert!(d < 4);
        let offset = unsafe { *self.offsets.get_unchecked(d) };
        let q = c.wrapping_add_signed(offset);
        debug_assert!(q < self.edge.len());
        q
    }
    #[inline(always)]
    fn edges(&self, c: usize) -> u8 {
        debug_assert!(c < self.edge.len());
        // The one-cell sentinel ring guarantees all neighbor reads are in bounds.
        unsafe { *self.edge.get_unchecked(c) }
    }
    #[inline(always)]
    fn state(&self, c: usize) -> u16 {
        debug_assert!(c < self.label.len());
        // Same sentinel-ring invariant as edges().
        unsafe { *self.label.get_unchecked(c) }
    }
    fn free(&self, c: usize) -> u8 {
        let e = self.edges(c);
        !(e | (e >> 4)) & 15
    }
    fn log(&mut self, kind: u16, index: usize, old: u16) {
        // A 64x64 board needs 13-bit positions: spill bit 12 into the unused
        // top index bit. Cell indices themselves fit in 13 bits. Still one u32.
        #[cfg(feature = "stats")]
        {
            self.stats.trail_entries += 1;
            self.stats.trail_peak = self.stats.trail_peak.max(self.trail.len() as u64 + 1);
        }
        let hi = index as u32 | (((old as u32) >> 12) << 15);
        let entry = (hi << 16) | (((old & 0xfff) as u32) << 4) | kind as u32;
        let len = self.trail.len();
        // Along a branch, edges and labels are monotone: at most 4n edge,
        // 2n label, n end, n cursor, n mark and pairs done entries coexist.
        // Board::new reserves 128n entries, well above that bound.
        debug_assert!(len < self.trail.capacity());
        unsafe {
            self.trail.as_mut_ptr().add(len).write(entry);
            self.trail.set_len(len + 1);
        }
    }
    #[inline(always)]
    fn put_label(&mut self, c: usize, v: u16) {
        let old = self.state(c);
        if old == v {
            #[cfg(feature = "stats")]
            {
                self.stats.label_noops += 1;
            }
            return;
        }
        #[cfg(feature = "stats")]
        {
            self.stats.label_writes += 1;
            if old != WALL && old & INSIDE != 0 {
                self.stats.internal_relabels += 1;
            }
        }
        self.log(LABEL, c, old);
        debug_assert!(c < self.label.len());
        unsafe { *self.label.get_unchecked_mut(c) = v };
        // Internal vertices were saturated at Line insertion: no usable edge
        // can expose this label change to a zero-cell pop.
        if v & INSIDE != 0 && v != WALL {
            return;
        }
        self.enqueue(c);
        // Only usable incident edges can expose a changed label to C.
        let mut free = self.free(c);
        while free != 0 {
            let d = free.trailing_zeros() as usize;
            free &= free - 1;
            let q = self.neighbor(c, d);
            if self.state(q) == 0 {
                self.enqueue(q);
            }
        }
    }
    fn dirty_end(&mut self, id: u16) {
        debug_assert!(active(id) && (id & 255) <= 254);
        self.dirty[id as usize] = 15;
        self.dirty_pairs |= 1u64 << (id & 63);
    }
    fn put_end(&mut self, id: u16, c: usize) {
        self.log(END, id as usize, self.end_pos[id as usize]);
        self.end_pos[id as usize] = c as u16;
        self.dirty_end(id);
    }
    fn enqueue(&mut self, c: usize) {
        debug_assert!(c < self.in_queue.len());
        if self.state(c) & INSIDE == 0 && !unsafe { *self.in_queue.get_unchecked(c) } {
            unsafe { *self.in_queue.get_unchecked_mut(c) = true };
            self.queue.push(c as u16);
        }
    }
    fn fail_site(&mut self, site: usize) {
        #[cfg(feature = "stats")]
        if self.consistent {
            self.stats.failures[self.stats.current][site] += 1;
        }
        #[cfg(not(feature = "stats"))]
        let _ = site;
        self.fail();
    }
    fn fail(&mut self) {
        #[cfg(feature = "stats")]
        if self.consistent {
            self.stats.rows[self.stats.current].contradictions += 1;
        }
        self.consistent = false;
    }
    #[cfg(test)]
    fn rollback_to(&mut self, checkpoint: usize) {
        self.rollback_to_impl::<true>(checkpoint);
    }
    // Keep rollback inlined across optional search specializations.
    #[inline(always)]
    fn rollback_to_impl<const RECHECK: bool>(&mut self, checkpoint: usize) {
        while self.trail.len() > checkpoint {
            let e = self.trail.pop().unwrap();
            let kind = (e & 15) as u16;
            let index = ((e >> 16) & 0x7fff) as usize;
            let old = (((e >> 4) & 0xfff) | ((e >> 31) << 12)) as u16;
            match kind {
                EDGE => {
                    debug_assert!(index < self.edge.len());
                    unsafe {
                        *self.edge.get_unchecked_mut(index) = old as u8;
                    }
                    let id = self.state(index);
                    if RECHECK && active(id) {
                        self.dirty_end(id);
                    }
                }
                LINE_EDGE | WALL_EDGE => {
                    let c = index >> 2;
                    let d = index & 3;
                    let q = self.neighbor(c, d);
                    let shift = if kind == WALL_EDGE { 4 } else { 0 };
                    debug_assert!(c < self.edge.len() && q < self.edge.len());
                    unsafe {
                        *self.edge.get_unchecked_mut(c) &= !(bit(d) << shift);
                        *self.edge.get_unchecked_mut(q) &= !(bit(opposite(d)) << shift);
                    }
                    for v in [self.state(c), self.state(q)] {
                        if RECHECK && active(v) {
                            self.dirty_end(v);
                        }
                    }
                }
                LABEL => {
                    debug_assert!(index < self.label.len());
                    unsafe {
                        *self.label.get_unchecked_mut(index) = old;
                    }
                    if RECHECK && active(old) {
                        self.dirty_end(old);
                    }
                }
                END => {
                    debug_assert!(index < self.end_pos.len());
                    unsafe {
                        *self.end_pos.get_unchecked_mut(index) = old;
                    }
                    if RECHECK {
                        self.dirty_end(index as u16);
                    }
                }
                DONE => {
                    debug_assert!(index < self.done.len());
                    unsafe {
                        self.remaining += (*self.done.get_unchecked(index) - old as u8) as usize;
                        *self.done.get_unchecked_mut(index) = old as u8;
                    }
                    if RECHECK {
                        self.dirty_end(index as u16);
                        self.dirty_end((index | 256) as u16);
                    }
                }
                CURSOR => self.cursor = old,
                MARK => {}
                _ => unreachable!(),
            }
        }
        // Consumed entries cleared their flags when popped. Only pending
        // entries can still be marked when propagation fails.
        for &c in &self.queue[self.queue_head..] {
            debug_assert!((c as usize) < self.in_queue.len());
            unsafe {
                *self.in_queue.get_unchecked_mut(c as usize) = false;
            }
        }
        self.queue.clear();
        self.queue_head = 0;
        self.consistent = true;
    }
    #[inline(always)]
    fn set_noline(&mut self, c: usize, d: usize) {
        if !self.consistent {
            return;
        }
        let m = bit(d);
        let e = self.edges(c);
        if e & (m << 4) != 0 {
            return;
        }
        if e & m != 0 {
            self.fail_site(0);
            return;
        }
        let q = self.neighbor(c, d);
        let r = bit(opposite(d));
        #[cfg(feature = "stats")]
        {
            self.stats.rows[self.stats.current].walls += 1;
        }
        self.log(WALL_EDGE, c * 4 + d, 0);
        debug_assert!(c < self.edge.len() && q < self.edge.len());
        unsafe { *self.edge.get_unchecked_mut(c) = e | (m << 4) };
        unsafe { *self.edge.get_unchecked_mut(q) |= r << 4 };
        self.enqueue(c);
        self.enqueue(q);
        for v in [self.state(c), self.state(q)] {
            if active(v) {
                self.dirty_end(v);
            }
        }
    }
    #[inline(always)]
    fn set_line(&mut self, c: usize, d: usize) {
        #[cfg(feature = "stats")]
        let choice = std::mem::take(&mut self.stats.choice_edge);
        if !self.consistent {
            return;
        }
        let e = self.edges(c);
        let m = bit(d);
        if e & m != 0 {
            return;
        }
        if e & (m << 4) != 0 {
            self.fail_site(1);
            return;
        }
        let q = self.neighbor(c, d);
        let r = bit(opposite(d));
        #[cfg(feature = "stats")]
        if !choice {
            self.stats.rows[self.stats.current].lines += 1;
        }
        self.log(LINE_EDGE, c * 4 + d, 0);
        debug_assert!(c < self.edge.len() && q < self.edge.len());
        unsafe { *self.edge.get_unchecked_mut(c) = e | m };
        unsafe { *self.edge.get_unchecked_mut(q) |= r };
        self.enqueue(c);
        self.enqueue(q);
        // Labelled insertions move an end or finish its pair below; those
        // events dirty the final exits. The old positions need no G1 seed.
        // D3 / S1: saturation precedes the geometric segment rules. NoLine
        // writes do not change either degree, so reuse the incident-cell gates.
        let c_one = self.shared.clue[c] == 0 && (self.edges(c) & 15).is_power_of_two();
        let q_one = self.shared.clue[q] == 0 && (self.edges(q) & 15).is_power_of_two();
        if !c_one {
            #[cfg(feature = "stats")]
            let previous = self.stats.enter(S_RULE + d);
            self.saturate_inserted(c);
            #[cfg(feature = "stats")]
            {
                self.stats.current = previous;
            }
        }
        if !q_one {
            #[cfg(feature = "stats")]
            let previous = self.stats.enter(S_RULE + opposite(d));
            self.saturate_inserted(q);
            #[cfg(feature = "stats")]
            {
                self.stats.current = previous;
            }
        }
        if c_one {
            self.segment_incident(c, d);
        }
        if q_one {
            self.segment_incident(q, opposite(d));
        }
        if !self.consistent {
            return;
        }
        let a = self.state(c);
        let b = self.state(q);
        if active(a) {
            self.advance_end(a, c, d);
        } else if active(b) {
            self.advance_end(b, q, opposite(d));
        } else if a == 0 && b == 0 {
            if count4(self.edges(c) & 15) == 2 && count4(self.edges(q) & 15) == 2 {
                self.check_loop(c);
            }
        } else if a != 0 || b != 0 {
            self.fail_site(2);
        }
        if !self.consistent {
            return;
        }
        self.corner_at(c);
        self.corner_at(q);
    }
    fn saturate_inserted(&mut self, c: usize) {
        // Before insertion, every saturated vertex has no free edges. Thus a
        // newly inserted Line that is not a blank's first Line saturates it.
        debug_assert_eq!(
            count4(self.edges(c) & 15),
            if self.shared.clue[c] != 0 { 1 } else { 2 }
        );
        let mut free = self.free(c);
        while free != 0 {
            let d = free.trailing_zeros() as usize;
            free &= free - 1;
            self.set_noline(c, d);
        }
    }
    #[cfg(test)]
    fn saturate(&mut self, c: usize) {
        let degree = count4(self.edges(c) & 15);
        let max = if self.shared.clue[c] != 0 { 1 } else { 2 };
        // D3 / S1: clues have degree one, used blanks have degree two.
        if degree > max || (self.state(c) == WALL && degree != 0) {
            self.fail_site(3);
            return;
        }
        if degree == max {
            let mut free = self.free(c);
            while free != 0 {
                let d = free.trailing_zeros() as usize;
                free &= free - 1;
                self.set_noline(c, d);
            }
        }
    }
    stats_rule! { W_RULE;
    fn check_loop(&mut self, start: usize) {
        #[cfg(feature = "stats")]
        {
            self.stats.walk_calls += 1;
        }
        // R3 / K4: D. A degree-two anonymous component must not close.
        let mut previous = usize::MAX;
        let mut c = start;
        for _ in 0..self.edge.len() {
            let mut next = None;
            for d in 0..4 {
                if self.edges(c) & bit(d) != 0 {
                    let q = self.neighbor(c, d);
                    if q != previous {
                        next = Some(q);
                        break;
                    }
                }
            }
            let Some(q) = next else {
                return;
            };
            if q == start {
                self.fail_site(4);
                return;
            }
            previous = c;
            c = q;
            if self.shared.clue[c] != 0 || count4(self.edges(c) & 15) < 2 {
                return;
            }
        }
        self.fail_site(5);
    }
    }
    fn advance_end(&mut self, id: u16, mut c: usize, mut d: usize) {
        #[cfg(feature = "stats")]
        {
            self.stats.walk_calls += 1;
        }
        // W1: D. Claim an anonymous segment by walking it, without union-find.
        let mate_start = self.end_pos[(id ^ 256) as usize] as usize;
        let stride = self.shared.stride;
        let mut mate_changed = false;
        self.put_label(c, id | INSIDE);
        loop {
            if !self.consistent {
                return;
            }
            let distance = c.abs_diff(mate_start);
            // E2 sees adjacent claims; E3 sees new interiors at distance two.
            // Padded row offsets may admit harmless extra checks, never omit one.
            mate_changed |= distance <= 2
                || (distance >= stride - 1 && distance <= stride + 1)
                || distance == 2 * stride;
            let q = self.neighbor(c, d);
            let v = self.state(q);
            if v != 0 {
                if v != (id ^ 256) {
                    self.fail_site(6);
                    return;
                }
                self.put_label(q, v | INSIDE);
                if self.end_pos[id as usize] as usize != c {
                    self.put_end(id, c);
                }
                self.log(
                    DONE,
                    (id & 255) as usize,
                    self.done[(id & 255) as usize] as u16,
                );
                self.done[(id & 255) as usize] = 1;
                self.remaining -= 1;
                // G1: retain both last positions for the completed-pair variant.
                self.dirty_end(id);
                self.dirty_end(id ^ 256);
                return;
            }
            // W2 and W3 inspect the same neighbors; retain the wall directions
            // while checking W2 so claiming q does not require a second scan.
            let mut walls = 0u8;
            for side in 0..4 {
                let n = self.neighbor(q, side);
                let v = self.state(n);
                if n != c && v & 0x2ff == (id & 255) | INSIDE {
                    self.fail_site(7);
                    return;
                }
                if v == WALL {
                    walls |= bit(side);
                }
            }
            let forward = (self.edges(q) & 15) & !bit(opposite(d));
            // Claim an anonymous straight run once. Intermediate cells never
            // become observable active ends; only the terminal end can extend.
            self.put_label(q, if forward == 0 { id } else { id | INSIDE });
            if forward == 0 {
                self.put_end(id, q);
            }
            // W3: K. Unknown vertices terminate an unused-cell ray.
            while walls != 0 {
                let side = walls.trailing_zeros() as usize;
                walls &= walls - 1;
                let mut n = self.neighbor(q, side);
                while self.shared.clue[n] != WALL && self.state(n) == WALL {
                    n = self.neighbor(n, side);
                }
                if self.state(n) != WALL && self.state(n) != 0 && self.state(n) & 255 == id & 255 {
                    self.fail_site(8);
                    return;
                }
            }
            if forward == 0 {
                if active(self.state(q)) {
                    self.endpoint(q);
                }
                let mate = self.end_pos[(id ^ 256) as usize] as usize;
                if mate_changed && active(self.state(mate)) {
                    self.endpoint(mate);
                }
                return;
            }
            if !forward.is_power_of_two() {
                self.fail_site(9);
                return;
            }
            c = q;
            d = forward.trailing_zeros() as usize;
        }
    }
    fn make_unused(&mut self, c: usize) {
        if self.state(c) == WALL {
            return;
        }
        if self.state(c) != 0 || self.edges(c) & 15 != 0 {
            self.fail_site(10);
            return;
        }
        self.put_label(c, WALL);
        // W4 / G1: an unused square changes contours at nearby active ends.
        let stride = self.shared.stride;
        for row in [c - stride, c, c + stride] {
            for q in [row - 1, row, row + 1] {
                let id = self.state(q);
                if active(id) {
                    self.dirty_end(id);
                }
            }
        }
        // W4: K. Check both axes after making c unused; the first unknown stops.
        for d in [U, R] {
            let mut a = self.neighbor(c, d);
            let mut b = self.neighbor(c, opposite(d));
            while self.shared.clue[a] != WALL && self.state(a) == WALL {
                a = self.neighbor(a, d);
            }
            while self.shared.clue[b] != WALL && self.state(b) == WALL {
                b = self.neighbor(b, opposite(d));
            }
            if self.state(a) != 0
                && self.state(b) != 0
                && self.state(a) != WALL
                && self.state(b) != WALL
                && self.state(a) & 255 == self.state(b) & 255
            {
                self.fail_site(11);
                return;
            }
        }
        // D: isolation of an unused vertex is mandatory.
        for d in 0..4 {
            self.set_noline(c, d);
        }
    }
    fn ray(&self, c: usize, d: usize) -> Option<(usize, usize)> {
        let mut q = c;
        loop {
            q = self.neighbor(q, d);
            let mask = (self.edges(q) & 15) & !bit(opposite(d));
            if mask == 0 {
                return None;
            }
            if mask != bit(d) {
                return if mask.is_power_of_two() {
                    Some((q, mask.trailing_zeros() as usize))
                } else {
                    None
                };
            }
        }
    }
    fn clear(&self, a: usize, b: usize, t: usize) -> bool {
        a.max(b) < self.shared.wedge[a.min(b)][opposite(t)] as usize
    }
    #[cfg(test)]
    fn segment(&mut self, c: usize) {
        if !self.consistent || self.state(c) == WALL {
            return;
        }
        let lines = self.edges(c) & 15;
        if self.shared.clue[c] != 0 || !lines.is_power_of_two() {
            self.saturate(c);
        } else {
            self.segment_incident(c, lines.trailing_zeros() as usize);
        }
    }
    fn segment_incident(&mut self, c: usize, i: usize) {
        #[cfg(feature = "stats")]
        let previous = self.stats.enter(S_RULE + i);
        match i {
            U => self.segment_inner::<U>(c),
            R => self.segment_inner::<R>(c),
            D => self.segment_inner::<D>(c),
            _ => self.segment_inner::<L>(c),
        }
        #[cfg(feature = "stats")]
        {
            self.stats.current = previous;
        }
    }
    fn segment_inner<const I: usize>(&mut self, c: usize) {
        #[cfg(feature = "stats")]
        {
            self.stats.segment_calls += 1;
        }
        debug_assert!(self.consistent && self.shared.clue[c] == 0 && self.edges(c) & 15 == bit(I));
        let i = I;
        // S2: K, clue-dependent downward-turn permissions.
        match i {
            R => {
                if !self.shared.southeast[c] {
                    self.set_noline(c, D);
                }
            }
            L => {
                if !self.shared.southwest[c] {
                    self.set_noline(c, D);
                }
            }
            D => {
                if !self.shared.southeast[c] {
                    self.set_noline(c, R);
                }
                if !self.shared.southwest[c] {
                    self.set_noline(c, L);
                }
            }
            _ => {}
        }
        if self.free(c) == 0 {
            return;
        }
        let sides = if i == R || i == L {
            [U, D]
        } else if i == D {
            [L, R]
        } else {
            [R, L]
        };
        // S3: K. Rays inspect line geometry, even across a proposed NoLine edge.
        for s in sides {
            if self.free(c) & bit(s) != 0 {
                if let Some((q, t)) = self.ray(c, s) {
                    if t == i && self.clear(c, q, i) {
                        self.set_noline(c, s);
                    }
                }
            }
        }
        let free = self.free(c);
        let f = opposite(i);
        // At a one-Line blank, two walls leave exactly one free side.
        if free == 0 || (free == bit(f) && self.edges(self.neighbor(c, f)) & 15 == 0) {
            return;
        }
        if let Some((q, t)) = self.ray(c, i) {
            // S4: K. Forbid a turn toward the distant bend of this arm.
            if self.free(c) & bit(t) != 0 && self.clear(c, q, t) {
                self.set_noline(c, t);
            }
            // S5: K. The forward arm must have the identical terminal turn.
            if self.free(c) & bit(f) != 0 {
                if let Some((r, u)) = self.ray(c, f) {
                    if t == u && self.clear(q, r, t) {
                        self.set_noline(c, f);
                    }
                }
            }
        }
    }
    stats_rule! { C_RULE + (self.edges(c) >> 4) as usize;
    // Keep this dispatch table in the hot loop; specialization otherwise outlines it.
    #[inline(always)]
    fn zero_cell(&mut self, c: usize) {
        let u = self.state(self.neighbor(c, U));
        let r = self.state(self.neighbor(c, R));
        let d = self.state(self.neighbor(c, D));
        let l = self.state(self.neighbor(c, L));
        let mate = |a: u16, b: u16| a == (b ^ 256);
        let a = self.shared.southeast[c];
        let z = self.shared.southwest[c];
        // C1-C6: the full 16-row dispatch, with the catalogue's evaluation order.
        // C1/C2 and incompatible known labels: D. C3 permissions/C4/C5/C6: K.
        match self.edges(c) & 0xf0 {
            0x00 => {
                if [u, r, d, l].contains(&0)
                    || mate(u, r)
                    || mate(u, d)
                    || mate(u, l)
                    || mate(r, d)
                    || mate(r, l)
                    || mate(d, l)
                {
                    return;
                }
                self.make_unused(c);
            }
            0x10 => {
                if r == 0 || l == 0 {
                    return;
                }
                if mate(r, l) {
                    self.straight(c, L, R);
                    return;
                }
                if d != 0 {
                    if mate(r, d) {
                        self.set_noline(c, L);
                    } else if mate(l, d) {
                        self.set_noline(c, R);
                    } else {
                        self.make_unused(c);
                    }
                } else if !z {
                    self.set_noline(c, L);
                } else if !a {
                    self.set_noline(c, R);
                }
            }
            0x20 => {
                if l == 0 || u == 0 {
                    return;
                }
                if mate(l, u) {
                    self.set_noline(c, D);
                    return;
                }
                if d == 0 {
                    if !z {
                        self.set_noline(c, L);
                    }
                } else if mate(l, d) {
                    self.set_noline(c, U);
                } else if mate(u, d) {
                    self.straight(c, U, D);
                } else {
                    self.make_unused(c);
                }
            }
            0x30 => {
                if !z || (d != 0 && l != 0 && !mate(d, l)) {
                    self.make_unused(c);
                }
            }
            0x40 => {
                if r == 0 || l == 0 {
                    return;
                }
                if mate(r, l) {
                    self.straight(c, L, R);
                    return;
                }
                if u == 0 {
                    return;
                }
                if mate(r, u) {
                    self.set_noline(c, L);
                } else if mate(l, u) {
                    self.set_noline(c, R);
                } else {
                    self.make_unused(c);
                }
            }
            0x50 => {
                if r == 0 || l == 0 {
                    return;
                }
                if mate(r, l) {
                    self.straight(c, L, R);
                } else {
                    self.make_unused(c);
                }
            }
            0x60 => {
                if l != 0 && u != 0 && !mate(l, u) {
                    self.make_unused(c);
                }
            }
            0x70 => self.make_unused(c),
            0x80 => {
                if r == 0 || u == 0 {
                    return;
                }
                if mate(r, u) {
                    self.set_noline(c, D);
                    return;
                }
                if d != 0 {
                    if mate(r, d) {
                        self.set_noline(c, U);
                    } else if mate(u, d) {
                        self.straight(c, U, D);
                    } else {
                        self.make_unused(c);
                    }
                } else if !a {
                    self.set_noline(c, R);
                }
            }
            0x90 => {
                if !a || (d != 0 && r != 0 && !mate(d, r)) {
                    self.make_unused(c);
                }
            }
            0xa0 => {
                if d == 0 || u == 0 {
                    return;
                }
                if mate(u, d) {
                    self.straight(c, U, D);
                } else {
                    self.make_unused(c);
                }
            }
            0xb0 => self.make_unused(c),
            0xc0 => {
                if r != 0 && u != 0 && !mate(r, u) {
                    self.make_unused(c);
                }
            }
            0xd0 => self.make_unused(c),
            0xe0 => self.make_unused(c),
            0xf0 => self.make_unused(c),
            _ => unreachable!(),
        }
    }
    }
    fn straight(&mut self, c: usize, a: usize, b: usize) {
        // C4: K. Unlike degree propagation this chooses use over isolation.
        self.set_line(c, a);
        self.set_line(c, b);
    }
    stats_rule! { E_RULE;
    fn endpoint(&mut self, c: usize) {
        self.end_rules(c, false);
    }
    }
    stats_rule! { D2_RULE;
    fn force_end(&mut self, c: usize) {
        // D2 only extends a sole exit. E1-E3 run on reached ends and mates.
        let f = self.free(c);
        if f == 0 {
            self.fail_site(12);
        } else if f.is_power_of_two() {
            // The shared forced E entry visits U,R,D,L and stops at the
            // forced direction. Earlier non-Line sides are all NoLine, so
            // only a blocked same-value E1 join can act before that direction.
            let d = f.trailing_zeros() as usize;
            let id = self.state(c) & 255;
            let mut earlier = (bit(d) - 1) & !(self.edges(c) & 15);
            while earlier != 0 {
                let side = earlier.trailing_zeros() as usize;
                earlier &= earlier - 1;
                if self.state(self.neighbor(c, side)) & 255 == id {
                    self.set_line(c, side);
                    return;
                }
            }
            self.set_line(c, d);
        }
    }
    }
    fn end_rules(&mut self, c: usize, force_single: bool) {
        #[cfg(feature = "stats")]
        {
            self.stats.end_rules_calls += 1;
        }
        if !self.consistent {
            return;
        }
        let id = self.state(c);
        // D2: D. An active end needs exactly one additional line.
        let f = self.free(c);
        if force_single && f == 0 {
            self.fail_site(13);
            return;
        }
        let forced = if force_single && f.is_power_of_two() {
            Some(f.trailing_zeros() as usize)
        } else {
            None
        };
        for side in 0..4 {
            if self.edges(c) & bit(side) != 0 {
                continue;
            }
            let q = self.neighbor(c, side);
            let v = self.state(q);
            // E1: K. A blocked direct join is a contradiction as well.
            if v & 255 == id & 255 || forced == Some(side) {
                self.set_line(c, side);
                return;
            }
            if self.free(c) & bit(side) == 0 {
                continue;
            }
            // E2: D. Occupied vertices other than the mate cannot accept a line.
            if v != 0 {
                self.set_noline(c, side);
                continue;
            }
            // E3: K. Anticipate W2 before extending into the adjacent blank.
            let inside = (id & 255) | INSIDE;
            let stride = self.shared.stride;
            // c is still an active end, so including it cannot match INSIDE.
            if self.state(q - stride) & 0x2ff == inside
                || self.state(q + 1) & 0x2ff == inside
                || self.state(q + stride) & 0x2ff == inside
                || self.state(q - 1) & 0x2ff == inside
            {
                self.set_noline(c, side);
            }
        }
    }
    fn corner_at(&mut self, c: usize) {
        if !self.consistent {
            return;
        }
        let m = self.edges(c) & 15;
        if m == bit(R) | bit(D) {
            let q = self.neighbor(self.neighbor(c, D), R);
            self.diagonal(q, R);
        } else if m == bit(D) | bit(L) {
            let q = self.neighbor(self.neighbor(c, D), L);
            self.diagonal(q, L);
        }
    }
    stats_rule! { K_RULE + usize::from(side == L);
    fn diagonal(&mut self, q: usize, side: usize) {
        let mut q = q;
        // K1: K. The next diagonal vertex cannot reach backward into this corner.
        self.set_noline(q, U);
        self.set_noline(q, opposite(side));
        loop {
            if !self.consistent {
                return;
            }
            // K2: K-derived failure; clues terminate the chain.
            if self.shared.clue[q] != 0 {
                self.enqueue(q);
                return;
            }
            if self.edges(q) & ((bit(side) | bit(D)) << 4) != 0 {
                self.fail_site(14);
                return;
            }
            if self.edges(q) & (bit(side) | bit(D)) == bit(side) | bit(D) {
                return;
            }
            // K3: K. Install the next backward bans before forcing this corner.
            let next = self.neighbor(self.neighbor(q, D), side);
            self.set_noline(next, U);
            self.set_noline(next, opposite(side));
            self.set_line(q, side);
            self.set_line(q, D);
            // K4 / R3: D, checked by set_line on every anonymous insertion.
            q = next;
        }
    }
    }
    fn propagate(&mut self) -> bool {
        while self.queue_head < self.queue.len() && self.consistent {
            #[cfg(feature = "stats")]
            {
                self.stats.worklist_pops += 1;
            }
            let c = self.queue[self.queue_head] as usize;
            self.queue_head += 1;
            self.in_queue[c] = false;
            let v = self.state(c);
            if v == WALL || v & INSIDE != 0 {
                continue;
            }
            // Every Line insertion performs D3 before the worklist is drained.
            if active(v) {
                self.force_end(c);
            } else {
                let lines = self.edges(c) & 15;
                if lines != 0 && lines & lines.wrapping_sub(1) == 0 {
                    // D1: D. An anonymous used blank must get a second line.
                    #[cfg(feature = "stats")]
                    let previous = self.stats.enter(D1_RULE);
                    let f = self.free(c);
                    if f == 0 {
                        self.fail_site(15);
                    } else if f.is_power_of_two() {
                        self.set_line(c, f.trailing_zeros() as usize);
                    }
                    #[cfg(feature = "stats")]
                    {
                        self.stats.current = previous;
                    }
                } else if lines == 0 {
                    self.zero_cell(c);
                }
            }
        }
        if !self.consistent {
            return false;
        }
        self.queue.clear();
        self.queue_head = 0;
        // G1 then G2: D. Only contradictions, no speculative edge choices.
        self.regions()
    }
    fn regions(&mut self) -> bool {
        if C::WIDE {
            return self.regions_wide();
        }
        if self.remaining < 2 {
            return true;
        }
        self.boundary_epoch = self.boundary_epoch.wrapping_add(1);
        if self.boundary_epoch == 0 {
            self.boundary_cell_seen.fill(0);
            self.boundary_epoch = 1;
        }
        // G1: revisit dirty exits of both active ends, or adjacent zero cells
        // of the last endpoint positions of completed pairs.
        let mut todo = std::mem::take(&mut self.dirty_pairs);
        while todo != 0 {
            let pair = todo.trailing_zeros() as usize;
            todo &= todo - 1;
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
    stats_rule! { G2_RULE;
    fn boundary(&mut self, start: usize, side: usize) -> bool {
        debug_assert!(start < self.boundary_cell_seen.len());
        if unsafe { *self.boundary_cell_seen.get_unchecked(start) } == self.boundary_epoch {
            return true;
        }
        #[cfg(feature = "stats")]
        {
            self.stats.boundary_walks += 1;
        }
        self.contacts.clear();
        let (mut c, mut d) = (start, side);
        // Clockwise contour of the union of unit squares with state zero.
        // Concave corners step diagonally; holes form independent contours.
        loop {
            // A seed side faces an occupied cell. The deterministic contour
            // returns to that exact side, so per-side epoch storage is redundant.
            debug_assert!(c < self.boundary_cell_seen.len());
            unsafe {
                #[cfg(feature = "stats")]
                if *self.boundary_cell_seen.get_unchecked(c) != self.boundary_epoch {
                    self.stats.contour_unique_cells += 1;
                }
                *self.boundary_cell_seen.get_unchecked_mut(c) = self.boundary_epoch;
            }
            #[cfg(feature = "stats")]
            {
                self.stats.contour_cells += 1;
            }
            let q = self.neighbor(c, d);
            let id = self.state(q);
            let into = opposite(d);
            debug_assert_ne!(id, 0); // The side being traced faces an occupied cell.
            if id & INSIDE == 0 && self.edges(q) & (bit(into) << 4) == 0 {
                let len = self.contacts.len();
                // A contour visits at most four sides per cell; reserve is 4n.
                debug_assert!(len < self.contacts.capacity() && (id as usize) < self.dirty.len());
                unsafe {
                    self.contacts.as_mut_ptr().add(len).write((id, bit(into)));
                    self.contacts.set_len(len + 1);
                    *self.dirty.get_unchecked_mut(id as usize) &= !bit(into);
                }
            }
            let t = (d + 1) & 3;
            let n = self.neighbor(c, t);
            if self.state(n) != 0 {
                d = t;
            } else {
                c = n;
                let n = self.neighbor(c, d);
                if self.state(n) == 0 {
                    c = n;
                    d = opposite(t);
                }
            }
            if c == start && d == side {
                break;
            }
        }
        // G2: D. Keep repeated contacts and XOR exits, as the specification requires.
        if self.contacts.len() < 4 {
            return true;
        }
        if self.done.len() > 64 { return self.contacts_wide(); }
        let mut exits = [0u8; 64];
        let (mut lower, mut upper) = (0u64, 0u64);
        for &(id, dir) in &self.contacts {
            // Pair numbers fit six bits; pack the two four-bit exit masks.
            exits[(id & 63) as usize] ^= dir << ((id >> 6) & 4);
            if id & 256 == 0 {
                lower |= 1u64 << (id & 255);
            } else {
                upper |= 1u64 << (id & 255);
            }
        }
        let mut candidates = lower & upper;
        let mut todo = candidates;
        while todo != 0 {
            let pair = todo.trailing_zeros() as usize;
            todo &= todo - 1;
            // The all-exits condition is independent of the second pair.
            if exits[pair] & 15 != self.free(self.end_pos[pair] as usize)
                || exits[pair] >> 4 != self.free(self.end_pos[pair | 256] as usize)
            {
                candidates &= !(1u64 << pair);
            }
        }
        while candidates & candidates.wrapping_sub(1) != 0 {
            let a = candidates.trailing_zeros() as u16;
            candidates &= candidates - 1;
            let mut others = candidates;
            while others != 0 {
                let b = others.trailing_zeros() as u16;
                others &= others - 1;
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
    // Keep the full-board cut scan out of x86's per-cell cursor dispatch.
    #[cfg_attr(target_arch = "x86_64", inline(never))]
    fn cut_capacity_ok(&mut self) -> bool {
        let good = self.cut_capacity_scanned_ok();
        #[cfg(test)]
        assert_eq!(good, self.cut_capacity_recomputed_ok());
        good
    }

    fn cut_capacity_scanned_ok(&mut self) -> bool {
        #[cfg(feature = "stats")]
        {
            self.stats.cut_checks += 1;
        }
        // D: current ends of incomplete pairs, including anonymous bridges
        // above the frontier. Recompute only at checks; no mutation or undo.
        let mut dx = [0i16; 66];
        let mut dy = [0i16; 66];
        let s = self.shared.stride;
        for pair in 1..self.done.len() {
            if self.done[pair] != 0 {
                continue;
            }
            let a = self.end_pos[pair] as usize;
            let b = self.end_pos[pair | 256] as usize;
            let (ax, bx, ay, by) = (a % s, b % s, a / s, b / s);
            dx[ax.min(bx)] += 1;
            dx[ax.max(bx)] -= 1;
            dy[ay.min(by)] += 1;
            dy[ay.max(by)] -= 1;
        }
        let mut cx = [0i16; 66];
        let mut cy = [0i16; 66];
        for (y, capacity_y) in cy
            .iter_mut()
            .enumerate()
            .take(self.shared.height + 1)
            .skip(1)
        {
            for (x, capacity_x) in cx
                .iter_mut()
                .enumerate()
                .take(self.shared.width + 1)
                .skip(1)
            {
                let c = y * s + x;
                let e = self.edges(c);
                let anonymous = self.state(c) == 0;
                *capacity_x += (e & 0x22 == 0 || (e & 2 != 0 && anonymous)) as i16;
                *capacity_y += (e & 0x44 == 0 || (e & 4 != 0 && anonymous)) as i16;
            }
        }
        let mut demand = 0;
        for x in 1..self.shared.width {
            demand += dx[x];
            if demand > cx[x] {
                #[cfg(feature = "stats")]
                {
                    self.stats.cut_fail_x += 1;
                }
                return false;
            }
        }
        demand = 0;
        for y in 1..self.shared.height {
            demand += dy[y];
            if demand > cy[y] {
                #[cfg(feature = "stats")]
                {
                    self.stats.cut_fail_y += 1;
                }
                return false;
            }
        }
        true
    }

    #[cfg(test)]
    fn cut_capacity_recomputed_ok(&self) -> bool {
        // D: use current ends of incomplete pairs, never original clues.
        let mut dx = [0i16; 66];
        let mut dy = [0i16; 66];
        let s = self.shared.stride;
        for pair in 1..self.done.len() {
            if self.done[pair] != 0 {
                continue;
            }
            let a = self.end_pos[pair] as usize;
            let b = self.end_pos[pair | 256] as usize;
            let (ax, bx) = (a % s, b % s);
            let (ay, by) = (a / s, b / s);
            dx[ax.min(bx)] += 1;
            dx[ax.max(bx)] -= 1;
            dy[ay.min(by)] += 1;
            dy[ay.max(by)] -= 1;
        }
        let mut demand = 0;
        for (x, delta) in dx.iter().enumerate().take(self.shared.width).skip(1) {
            demand += delta;
            if demand == 0 {
                continue;
            }
            let mut capacity = 0;
            for y in 1..=self.shared.height {
                let c = y * s + x;
                let e = self.edges(c);
                if e & 0x22 == 0 || (e & 2 != 0 && self.state(c) == 0) {
                    capacity += 1;
                }
            }
            if demand > capacity {
                return false;
            }
        }
        demand = 0;
        for (y, delta) in dy.iter().enumerate().take(self.shared.height).skip(1) {
            demand += delta;
            if demand == 0 {
                continue;
            }
            let mut capacity = 0;
            for x in 1..=self.shared.width {
                let c = y * s + x;
                let e = self.edges(c);
                // Anonymous bridges above the frontier count on both axes.
                if e & 0x44 == 0 || (e & 4 != 0 && self.state(c) == 0) {
                    capacity += 1;
                }
            }
            if demand > capacity {
                return false;
            }
        }
        true
    }

    fn anonymous_connected(&self, a: usize, b: usize) -> bool {
        // Anonymous components are paths. Walk each arm without scratch allocation.
        let mut arms = self.edges(a) & 15;
        while arms != 0 {
            let d = arms.trailing_zeros() as usize;
            arms &= arms - 1;
            let mut previous = a;
            let mut c = self.neighbor(a, d);
            for _ in 0..self.edge.len() {
                if c == b {
                    return true;
                }
                let mut lines = self.edges(c) & 15;
                let mut next = None;
                while lines != 0 {
                    let side = lines.trailing_zeros() as usize;
                    lines &= lines - 1;
                    let q = self.neighbor(c, side);
                    if q != previous {
                        next = Some(q);
                        break;
                    }
                }
                let Some(q) = next else {
                    break;
                };
                previous = c;
                c = q;
                if c == a {
                    break;
                }
            }
        }
        false
    }

    fn diagonal_shortcut_ok(&self) -> bool {
        // K: only committed unused cells justify a two-edge shortcut. At early
        // completion all unclaimed degree-zero cells are also unused.
        for y in 1..=self.shared.height {
            for x in 1..=self.shared.width {
                let u = y * self.shared.stride + x;
                if self.state(u) != WALL
                    && !(self.remaining == 0 && self.state(u) == 0 && self.edges(u) & 15 == 0)
                {
                    continue;
                }
                for dv in [U, D] {
                    let a = self.neighbor(u, dv);
                    if self.edges(a) & 15 == 0 {
                        continue;
                    }
                    for dh in [L, R] {
                        let b = self.neighbor(u, dh);
                        if self.edges(b) & 15 == 0 {
                            continue;
                        }
                        let va = self.state(a);
                        let vb = self.state(b);
                        let same_component;
                        if va == 0 && vb == 0 {
                            same_component = self.anonymous_connected(a, b);
                            if !same_component {
                                continue;
                            }
                        } else if va != 0 && vb != 0 && va & 255 == vb & 255 {
                            same_component =
                                va & 0x1ff == vb & 0x1ff || self.done[(va & 255) as usize] != 0;
                        } else {
                            continue;
                        }
                        let c = self.neighbor(a, dh);
                        let ac = self.edges(a) & bit(dh) != 0;
                        let bc = self.edges(b) & bit(dv) != 0;
                        if ac && bc {
                            continue;
                        }
                        // Separate halves may still join through c with equal length.
                        if !same_component
                            && self.shared.clue[c] == 0
                            && self.edges(a) & (bit(dh) << 4) == 0
                            && self.edges(b) & (bit(dv) << 4) == 0
                            && count4(self.edges(c) & 15) == ac as u32 + bc as u32
                        {
                            continue;
                        }
                        return false;
                    }
                }
            }
        }
        true
    }

    #[cfg_attr(target_arch = "x86_64", inline(always))]
    fn next_cell(&mut self) -> Option<(usize, bool)> {
        let s = self.shared.stride;
        let mut c = self.cursor as usize;
        while c < (self.shared.height + 1) * s {
            let v = self.state(c);
            let lines = self.edges(c) & 15;
            if C::ENABLED {
                let x = c % s;
                if x > 0
                    && x <= self.shared.width
                    && (x - 1).is_multiple_of(self.pruning.data().interval)
                    && !self.cut_capacity_ok()
                {
                    // A failed scan check is an attempted search step, just like
                    // a branch rejected before its first edge can be placed.
                    self.steps += 1;
                    self.fail();
                    return None;
                }
            }
            if v & INSIDE == 0 && (active(v) || lines & lines.wrapping_sub(1) == 0) {
                return Some((c, v == 0 && lines == 0));
            }
            c += 1;
        }
        None
    }
    stats_rule! { if alternative { UNDO_RULE } else { R_RULE };
    fn choose(&mut self, c: usize, zero: bool, alternative: bool) {
        self.steps += 1;
        self.log(CURSOR, 0, self.cursor);
        self.cursor = (c + 1) as u16;
        if zero {
            // R1: choices. R2: K square bans, then D degree-required Down.
            if alternative {
                self.make_unused(c);
            } else {
                #[cfg(feature = "stats")]
                {
                    self.stats.choice_edge = true;
                }
                self.set_line(c, R);
                let right = self.neighbor(c, R);
                let down = self.neighbor(c, D);
                self.set_noline(right, D);
                self.set_noline(down, R);
                self.set_line(c, D);
            }
        } else {
            #[cfg(feature = "stats")]
            {
                self.stats.choice_edge = true;
            }
            self.set_line(c, if alternative { D } else { R });
        }
    }
    }
    // Keep this loop outlined on non-x86 targets; x86 inlines it at entry.
    #[cfg_attr(target_arch = "x86_64", inline(always))]
    #[cfg_attr(not(target_arch = "x86_64"), inline(never))]
    fn search_cut<const DIAG: bool>(mut self, direction: u8, original: &Puzzle) -> SearchOutcome {
        let mut out = SearchOutcome {
            direction,
            ..SearchOutcome::default()
        };
        search_loop!(self, out, DIAG, good, original, {}, {}, {})
    }
    fn search<const DIAG: bool>(mut self, direction: u8, original: &Puzzle) -> Outcome {
        let mut out = Outcome {
            direction,
            threads: 1,
            ..Outcome::default()
        };
        search_loop!(self, out, DIAG, good, original, {}, {}, {})
    }
    #[cfg_attr(target_arch = "x86_64", inline(never))]
    fn search_memo<const DIAG: bool>(
        mut self,
        direction: u8,
        original: &Puzzle,
        memo_bits: usize,
    ) -> Outcome {
        let mut memo = Memo::new(memo_bits, self.shared.height);
        let mut out = Outcome {
            direction,
            threads: 1,
            ..Outcome::default()
        };
        search_loop!(
            self,
            out,
            DIAG,
            good,
            original,
            {
                if memo.observe(&self, out.solutions, good) {
                    #[cfg(all(test, debug_assertions))]
                    self.assert_failed_subtree::<DIAG>();
                    self.fail();
                    good = false;
                }
            },
            {},
            {
                memo.finish(out.solutions);
                out.memo_hits = memo.hits;
                out.memo_stores = memo.stores;
                out.memo_slots = memo.slots();
            }
        )
    }
    #[cfg(all(test, debug_assertions))]
    fn assert_failed_subtree<const DIAG: bool>(&self) {
        if !memo::CHECK_HITS.with(|enabled| enabled.get()) {
            return;
        }
        assert!(self.shared.width * self.shared.height <= 100);
        let mut budget = 100_000;
        assert!(
            !self.snapshot().subtree_has_solution::<DIAG>(&mut budget),
            "memo discarded a solution"
        );
        memo::CHECKED_HITS.with(|n| n.set(n.get() + 1));
    }
    fn snapshot(&self) -> Self {
        let mut copy = self.clone();
        // Vec::clone does not preserve spare capacity required by packed writers.
        copy.queue.reserve(self.queue.capacity() - copy.queue.len());
        copy.choices
            .reserve(self.choices.capacity() - copy.choices.len());
        copy.trail.reserve(self.trail.capacity() - copy.trail.len());
        copy.contacts
            .reserve(self.contacts.capacity() - copy.contacts.len());
        copy
    }
    #[cfg(all(test, debug_assertions))]
    fn subtree_has_solution<const DIAG: bool>(mut self, budget: &mut usize) -> bool {
        assert!(*budget > 0, "memo soundness check exceeded its bound");
        *budget -= 1;
        if self.remaining == 0 {
            return !self.has_anonymous_lines();
        }
        let Some((c, zero)) = self.next_cell() else {
            return false;
        };
        for alternative in [false, true] {
            let mut child = self.snapshot();
            child.choose(c, zero, alternative);
            if propagate_optional!(child, DIAG) && child.subtree_has_solution::<DIAG>(budget) {
                return true;
            }
        }
        false
    }
    fn reject_solution(&mut self) {
        #[cfg(feature = "stats")]
        let previous = self.stats.current;
        #[cfg(feature = "stats")]
        {
            self.stats.current = D4_RULE;
        }
        self.fail_site(18);
        #[cfg(feature = "stats")]
        {
            self.stats.current = previous;
        }
    }
    fn has_anonymous_lines(&self) -> bool {
        // D4: D. A completed set of pairs cannot leave a separate anonymous path.
        self.label
            .iter()
            .zip(&self.edge)
            .any(|(&v, &e)| v == 0 && e & 15 != 0)
    }
    fn original_solution(&self, k: u8, p: &Puzzle) -> Vec<u8> {
        let mut result = vec![0; p.width * p.height];
        for y in 0..p.height {
            for x in 0..p.width {
                let xx = if k & 2 != 0 { p.width - 1 - x } else { x };
                let yy = if k & 4 != 0 { p.height - 1 - y } else { y };
                let (tx, ty) = if k & 1 != 0 { (yy, xx) } else { (xx, yy) };
                let e = self.edges((ty + 1) * self.shared.stride + tx + 1) & 15;
                for d in 0..4 {
                    let mut td = d;
                    if k & 2 != 0 && (td == R || td == L) {
                        td = opposite(td);
                    }
                    if k & 4 != 0 && (td == U || td == D) {
                        td = opposite(td);
                    }
                    if k & 1 != 0 {
                        td = match td {
                            U => L,
                            R => D,
                            D => R,
                            L => U,
                            _ => unreachable!(),
                        };
                    }
                    if e & bit(td) != 0 {
                        result[y * p.width + x] |= bit(d);
                    }
                }
            }
        }
        result
    }
}

fn solve_none(puzzle: &Puzzle, direction: u8) -> Outcome {
    assert!(direction < 8);
    assert!((1..=64).contains(&puzzle.width) && (1..=64).contains(&puzzle.height));
    assert!((1..=254).contains(&puzzle.pairs));
    assert_eq!(puzzle.cells.len(), puzzle.width * puzzle.height);
    let p = puzzle.transformed(direction);
    if puzzle.pairs > 63 {
        Board::<wide::Many<NoCut>>::new(&p).search::<false>(direction, puzzle)
    } else {
        Board::<NoCut>::new(&p).search::<false>(direction, puzzle)
    }
}

/// Solve with the default cut and memo settings.
pub fn solve(puzzle: &Puzzle, direction: u8) -> Outcome {
    solve_with(puzzle, direction, &Options::default())
}

pub fn solve_with(puzzle: &Puzzle, direction: u8, options: &Options) -> Outcome {
    assert!(options.memo_bits <= 28, "memo bits must be 0..=28");
    assert!(
        (1..=256).contains(&options.threads),
        "threads must be 1..=256"
    );
    if *options == Options::none() {
        return solve_none(puzzle, direction);
    }
    assert!(direction < 8);
    assert!((1..=64).contains(&puzzle.width) && (1..=64).contains(&puzzle.height));
    assert!((1..=254).contains(&puzzle.pairs));
    assert_eq!(puzzle.cells.len(), puzzle.width * puzzle.height);
    let p = puzzle.transformed(direction);
    // Specialize once at entry. Disabled checks and counter updates disappear.
    #[cfg_attr(target_arch = "x86_64", inline(never))]
    fn run<C: CutState + Send, const DIAG: bool>(
        p: &Puzzle,
        direction: u8,
        original: &Puzzle,
        cut: usize,
        options: &Options,
    ) -> Outcome {
        let mut board = Board::<C>::new(p);
        if C::ENABLED {
            board.pruning.data_mut().interval = cut;
        }
        if options.threads > 1 {
            return parallel::solve::<C, DIAG>(board, direction, original, options);
        }
        if options.memo_bits == 0 {
            if C::ENABLED {
                board.search_cut::<DIAG>(direction, original).into()
            } else {
                board.search::<DIAG>(direction, original)
            }
        } else {
            board.search_memo::<DIAG>(direction, original, options.memo_bits)
        }
    }
    if puzzle.pairs > 63 {
        return match (options.cut != 0, options.diag_shortcut) {
            (false, false) => run::<wide::Many<NoCut>, false>(&p, direction, puzzle, 0, options),
            (true, false) => {
                run::<wide::Many<CutCapacity>, false>(&p, direction, puzzle, options.cut, options)
            }
            (false, true) => run::<wide::Many<NoCut>, true>(&p, direction, puzzle, 0, options),
            (true, true) => {
                run::<wide::Many<CutCapacity>, true>(&p, direction, puzzle, options.cut, options)
            }
        };
    }
    match (options.cut != 0, options.diag_shortcut) {
        (false, false) => run::<NoCut, false>(&p, direction, puzzle, 0, options),
        (true, false) => run::<CutCapacity, false>(&p, direction, puzzle, options.cut, options),
        (false, true) => run::<NoCut, true>(&p, direction, puzzle, 0, options),
        (true, true) => run::<CutCapacity, true>(&p, direction, puzzle, options.cut, options),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn board(rows: &[&str]) -> Board {
        let w = rows[0].len();
        let text = format!(
            "{}\n{}\n{}\n",
            "-".repeat(w),
            rows.join("\n"),
            "-".repeat(w)
        );
        Board::new(&Puzzle::parse(&text).unwrap())
    }
    fn cell(b: &Board, y: usize, x: usize) -> usize {
        y * b.shared.stride + x
    }
    fn raw_line(b: &mut Board, c: usize, d: usize) {
        let q = b.neighbor(c, d);
        b.edge[c] &= !(bit(d) << 4);
        b.edge[c] |= bit(d);
        b.edge[q] &= !(bit(opposite(d)) << 4);
        b.edge[q] |= bit(opposite(d));
    }
    fn quiet(b: &mut Board) {
        b.queue.clear();
        b.queue_head = 0;
        b.in_queue.fill(false);
    }

    #[test]
    fn cut_recomputation_matches_axis_reference_through_search_and_undo() {
        let p = Puzzle::parse(include_str!("../reference/ex1.txt")).unwrap();
        for k in 0..8 {
            let transformed = p.transformed(k);
            let mut b = Board::<CutCapacity>::new(&transformed);
            b.pruning.interval = 1;
            assert_eq!(b.cut_capacity_ok(), b.cut_capacity_recomputed_ok());
            // Every scan check is compared with an independent axis-by-axis
            // implementation, including states restored through backtracking.
            let result = b.search_cut::<true>(k, &p);
            assert_eq!(result.solutions, 1);
        }
    }

    #[test]
    fn cut_recomputation_handles_maximum_and_single_axis_boards() {
        for (width, height) in [(64, 64), (1, 64), (64, 1)] {
            let mut cells = vec![0; width * height];
            cells[0] = 1;
            cells[width * height - 1] = 1;
            let puzzle = Puzzle {
                width,
                height,
                pairs: 1,
                cells,
            };
            let mut b = Board::<CutCapacity>::new(&puzzle);
            assert!(b.cut_capacity_ok());
            if width > 1 {
                for y in 1..=height {
                    b.set_noline(y * b.shared.stride + width / 2, R);
                }
            } else {
                b.set_noline((height / 2) * b.shared.stride + 1, D);
            }
            assert!(!b.cut_capacity_ok());
            b.rollback_to(0);
            assert!(b.cut_capacity_ok());
        }
    }

    #[test]
    fn cut_anonymous_bridges_above_frontier_both_axes() {
        for horizontal in [true, false] {
            let mut b = board(&["1....", ".....", ".....", ".....", "....1"]);
            let s = b.shared.stride;
            b.cursor = cell(&b, 5, 5) as u16;
            for i in 1..=5 {
                let c = if horizontal {
                    cell(&b, i, 2)
                } else {
                    cell(&b, 2, i)
                };
                let d = if horizontal { R } else { D };
                let q = b.neighbor(c, d);
                b.edge[c] |= bit(d) << 4;
                b.edge[q] |= bit(opposite(d)) << 4;
            }
            assert!(!b.cut_capacity_ok());
            let c = if horizontal { s + 2 } else { 2 * s + 1 };
            let d = if horizontal { R } else { D };
            raw_line(&mut b, c, d);
            assert!(b.cut_capacity_ok(), "anonymous bridge on axis {horizontal}");
            b.label[c] = 1 | INSIDE;
            let q = b.neighbor(c, d);
            b.label[q] = 1 | INSIDE;
            assert!(!b.cut_capacity_ok(), "claimed line is not spare capacity");
        }
    }

    #[test]
    fn cut_uses_current_ends_and_skips_completed_pairs() {
        let mut b = board(&["1...1", ".....", ".....", ".....", "....."]);
        for y in 1..=5 {
            let c = cell(&b, y, 2);
            b.edge[c] |= 0x20;
            b.edge[c + 1] |= 0x80;
        }
        assert!(!b.cut_capacity_ok());
        let original = b.end_pos[257];
        b.end_pos[257] = cell(&b, 4, 1) as u16;
        assert!(b.cut_capacity_ok(), "current ends are on the same side");
        b.end_pos[257] = original;
        b.done[1] = 1;
        assert!(b.cut_capacity_ok(), "completed pair adds no demand");
    }

    fn diagonal_case(dv: usize, dh: usize, labelled: bool) -> (Board, usize, usize, usize) {
        let mut b = board(&[
            "1......", ".......", ".......", ".......", ".......", ".......", "......1",
        ]);
        let u = cell(&b, 4, 4);
        b.label[u] = WALL;
        let a = b.neighbor(u, dv);
        let end = b.neighbor(u, dh);
        let mut c = a;
        for d in [dv, dh, dh, opposite(dv), opposite(dv), opposite(dh)] {
            raw_line(&mut b, c, d);
            c = b.neighbor(c, d);
        }
        assert_eq!(c, end);
        if labelled {
            for c in 0..b.label.len() {
                if b.edge[c] & 15 != 0 {
                    b.label[c] = 1 | INSIDE;
                }
            }
        }
        (b, u, a, end)
    }

    #[test]
    fn diagonal_four_corners_anonymous_and_labelled() {
        for dv in [U, D] {
            for dh in [L, R] {
                for labelled in [false, true] {
                    let (mut b, u, a, end) = diagonal_case(dv, dh, labelled);
                    assert!(
                        !b.diagonal_shortcut_ok(),
                        "corner {dv}/{dh}, labelled {labelled}"
                    );
                    b.label[u] = 0;
                    assert!(b.diagonal_shortcut_ok(), "undecided cell is not unused");
                    b.remaining = 0;
                    assert!(
                        !b.diagonal_shortcut_ok(),
                        "early completion commits empty cells"
                    );
                    b.remaining = 1;
                    b.label[u] = WALL;
                    // Keep just the two-edge tie through the diagonal corner.
                    for e in &mut b.edge {
                        *e &= 0xf0;
                    }
                    raw_line(&mut b, a, dh);
                    raw_line(&mut b, end, dv);
                    assert!(b.diagonal_shortcut_ok(), "existing two-edge tie");
                }
            }
        }
    }

    #[test]
    fn diagonal_separate_anonymous_segments_are_not_the_same_line() {
        let (mut b, _, a, _) = diagonal_case(U, L, false);
        let c = b.neighbor(a, U);
        let q = b.neighbor(c, L);
        b.edge[c] &= !bit(L);
        b.edge[q] &= !bit(R);
        assert!(b.diagonal_shortcut_ok());
    }

    #[test]
    fn diagonal_possible_corner_join_between_separate_halves() {
        for dv in [U, D] {
            for dh in [L, R] {
                let (mut b, _, a, end) = diagonal_case(dv, dh, true);
                b.label[end] = 257 | INSIDE;
                assert!(
                    b.diagonal_shortcut_ok(),
                    "separate halves may tie through corner"
                );
                b.edge[a] |= bit(dh) << 4;
                assert!(
                    !b.diagonal_shortcut_ok(),
                    "blocked corner forces longer join"
                );
                b.edge[a] &= !(bit(dh) << 4);
                let corner = b.neighbor(a, dh);
                Arc::make_mut(&mut b.shared).clue[corner] = 2;
                assert!(
                    !b.diagonal_shortcut_ok(),
                    "a clue cannot be a joining corner"
                );
                Arc::make_mut(&mut b.shared).clue[corner] = 0;
                raw_line(&mut b, corner, dv);
                assert!(
                    !b.diagonal_shortcut_ok(),
                    "unrelated corner line prevents tie"
                );
            }
        }
    }

    #[cfg(feature = "stats")]
    #[test]
    fn stats_undirected_edges_nested_failure_and_branch_exclusion() {
        let mut b = board(&[".....", ".....", ".....", ".....", "1...1"]);
        let c = cell(&b, 3, 3);
        b.set_noline(c, U);
        b.set_noline(c, U);
        assert_eq!(b.stats.rows[R_RULE].walls, 1);
        b.set_line(c, U);
        b.set_line(c, U);
        assert_eq!(b.stats.rows[R_RULE].contradictions, 1);
        b.rollback_to(0);
        b.choose(c, false, false);
        assert_eq!(b.stats.rows.iter().map(|x| x.lines).sum::<u64>(), 0);
        assert!(b.stats.trail_entries > 0);

        let mut b = board(&["11", ".."]);
        let c = cell(&b, 1, 1);
        b.set_noline(c, R);
        b.endpoint(c);
        assert_eq!(b.stats.rows[E_RULE].contradictions, 1);
        assert_eq!(b.stats.rows[R_RULE].contradictions, 0);
        assert_eq!(b.stats.current, R_RULE);
    }

    #[cfg(feature = "stats")]
    #[test]
    fn stats_solution_exhaustion_is_not_a_failure() {
        let p = Puzzle::parse("--\n11\n--\n").unwrap();
        let out = solve_none(&p, 0);
        assert_eq!(out.solutions, 1);
        assert_eq!(out.steps, 0);
        assert_eq!(out.stats.rows.iter().map(|x| x.lines).sum::<u64>(), 1);
        assert_eq!(
            out.stats.rows.iter().map(|x| x.contradictions).sum::<u64>(),
            0
        );
    }

    #[test]
    fn p1c_forced_end_checks_blocked_join_before_forced_direction() {
        let mut b = board(&[".....", ".....", ".....", ".....", "1...1"]);
        let c = cell(&b, 3, 3);
        b.label[c] = 1;
        b.end_pos[1] = c as u16;
        b.label[c - b.shared.stride] = 1 | INSIDE;
        raw_line(&mut b, c, L);
        b.set_noline(c, U);
        b.set_noline(c, R);
        assert_eq!(b.free(c), bit(D));
        b.force_end(c);
        assert!(!b.consistent);
        assert_eq!(b.free(c), bit(D));
    }

    #[test]
    fn p1c_claims_intermediate_cells_once() {
        let mut b = board(&["1....", ".....", "....1"]);
        let c = cell(&b, 1, 1);
        raw_line(&mut b, c + 1, R);
        raw_line(&mut b, c + 2, R);
        quiet(&mut b);
        b.set_line(c, R);
        assert!(b.consistent);
        assert_eq!(b.end_pos[1] as usize, c + 3);
        for q in [c + 1, c + 2] {
            assert_eq!(b.state(q), 1 | INSIDE);
            assert_eq!(
                b.trail
                    .iter()
                    .filter(|&&entry| {
                        entry & 15 == LABEL as u32 && ((entry >> 16) & 0x7fff) as usize == q
                    })
                    .count(),
                1
            );
        }
        assert_eq!(
            b.trail
                .iter()
                .filter(|&&entry| entry & 15 == END as u32)
                .count(),
            1
        );
    }

    #[cfg(feature = "stats")]
    #[test]
    fn p1c_contour_marks_and_forbidden_dirty_exit() {
        let mut b = board(&["1...2", ".....", "1...2"]);
        b.boundary_epoch = 1;
        assert!(b.boundary(cell(&b, 1, 2), L));
        let walks = b.stats.boundary_walks;
        assert!(b.boundary(cell(&b, 1, 3), U));
        assert_eq!(b.stats.boundary_walks, walks);
        let c = cell(&b, 1, 1);
        b.set_noline(c, R);
        b.dirty.fill(0);
        b.dirty[1] = bit(R);
        b.dirty_pairs = 1 << 1;
        assert!(b.regions());
        assert_eq!(b.stats.boundary_walks, walks);
    }

    #[test]
    fn p1b_segment_rules_only_fire_on_line_incidents() {
        let mut b = board(&[".....", ".....", ".....", ".....", "1...1"]);
        let c = cell(&b, 3, 3);
        let q = cell(&b, 1, 3);
        let middle = cell(&b, 2, 3);
        raw_line(&mut b, c, L);
        raw_line(&mut b, middle, U);
        raw_line(&mut b, q, L);
        Arc::make_mut(&mut b.shared).southwest[c] = true;
        quiet(&mut b);
        b.set_noline(c, D);
        assert!(b.propagate());
        assert_ne!(b.free(c) & bit(U), 0);
        b.segment(c);
        assert_eq!(b.free(c) & bit(U), 0); // A remote re-fire would have banned it.
    }

    #[test]
    fn p1b_dirty_events_stay_local() {
        let mut b = board(&["1...2", ".....", "..3..", ".....", "1.3.2"]);
        quiet(&mut b);
        b.dirty.fill(0);
        b.dirty_pairs = 0;
        let c = cell(&b, 1, 1);
        b.set_noline(c, R);
        assert_eq!(b.dirty[1], 15);
        assert_eq!(b.dirty_pairs, 1 << 1);
        b.dirty.fill(0);
        b.dirty_pairs = 0;
        let c = cell(&b, 4, 4);
        b.put_label(c, 2 | INSIDE);
        assert_eq!(b.dirty_pairs, 0); // Generic label writes do not invalidate G1.
        let c = cell(&b, 2, 4);
        b.make_unused(c);
        assert!(b.consistent);
        assert_eq!(b.dirty[2], 15); // Diagonally adjacent end.
        assert_eq!(b.dirty[3], 15);
        assert_eq!(b.dirty[1], 0);
        let c = cell(&b, 2, 1);
        b.put_end(1, c);
        assert_eq!(b.dirty[1], 15);
    }

    #[test]
    fn p1b_completed_ends_and_rollback_restore_region_work() {
        let mut b = board(&["11..", "..22"]);
        quiet(&mut b);
        let c = cell(&b, 1, 1);
        b.set_line(c, R);
        assert!(b.consistent);
        assert_eq!(b.remaining, 1);
        assert_eq!(b.dirty[1], 15);
        assert_eq!(b.dirty[257], 15);
        assert_eq!(b.end_pos[1] as usize, c);
        assert_eq!(b.end_pos[257] as usize, c + 1);
        b.dirty.fill(0);
        b.dirty_pairs = 0;
        b.rollback_to(0);
        assert_eq!(b.remaining, 2);
        assert_eq!(b.done[1], 0);
        assert_eq!(b.dirty[1], 15);
        assert_eq!(b.dirty[257], 15);
        assert_eq!(b.dirty_pairs, 1 << 1);
        assert!(b.in_queue.iter().all(|&v| !v));
    }

    #[test]
    fn p1b_boundary_epoch_wrap_keeps_alternating_rejection() {
        let mut b = board(&[".1.", "2.2", ".1."]);
        let c = cell(&b, 2, 2);
        for d in 0..4 {
            let q = b.neighbor(c, d);
            for t in 0..4 {
                if b.neighbor(q, t) != c {
                    b.set_noline(q, t);
                }
            }
        }
        b.boundary_epoch = u32::MAX;
        b.boundary_cell_seen.fill(u32::MAX);
        assert!(!b.regions());
        assert_eq!(b.boundary_epoch, 1);
    }

    #[test]
    fn p1b_nibble_popcounts() {
        for mask in 0u8..16 {
            assert_eq!(count4(mask), mask.count_ones());
        }
    }

    #[test]
    fn r1_r2_choices_and_undo() {
        let mut b = board(&["...", "...", "1.1"]);
        let c = cell(&b, 1, 1);
        let checkpoint = b.trail.len();
        b.choose(c, true, false);
        assert!(b.consistent);
        assert_eq!(b.edges(c) & 15, 6);
        assert_ne!(b.edges(b.neighbor(c, R)) & 0x40, 0);
        assert_ne!(b.edges(b.neighbor(c, D)) & 0x20, 0);
        assert_eq!(b.edges(cell(&b, 2, 2)) & 15, 6);
        b.rollback_to(checkpoint);
        assert_eq!(b.edges(c) & 15, 0);
        b.choose(c, true, true);
        assert_eq!(b.label[c], WALL);
    }
    #[test]
    fn r3_k4_anonymous_loop() {
        let mut b = board(&["....", "....", "....", "1..1"]);
        let c = cell(&b, 1, 1);
        raw_line(&mut b, c, R);
        raw_line(&mut b, c, D);
        let r = b.neighbor(c, R);
        let d = b.neighbor(c, D);
        raw_line(&mut b, r, D);
        raw_line(&mut b, d, R);
        b.check_loop(c);
        assert!(!b.consistent);
    }
    #[test]
    fn d1_d2_d3_forced_degrees_and_d4_completion() {
        let p = Puzzle::parse("-----\n1...1\n-----\n").unwrap();
        let out = solve_none(&p, 0);
        assert_eq!(out.solutions, 1);
        assert_eq!(out.steps, 0);
        assert_eq!(out.first_solution_has_empty, Some(false));
        assert_eq!(out.first_solution.unwrap(), vec![2, 10, 10, 10, 8]);
        let mut b = board(&["1..", "...", "..1"]);
        let c = cell(&b, 2, 2);
        for d in [U, R, D] {
            raw_line(&mut b, c, d);
        }
        b.saturate(c);
        assert!(!b.consistent);
        // D4 must not accept an anonymous segment when the labelled pairs finish.
        let mut b = board(&["11..", "....", "...."]);
        quiet(&mut b);
        let c = cell(&b, 3, 2);
        raw_line(&mut b, c, R);
        b.done[1] = 1;
        assert!(b.has_anonymous_lines());
        b.edge[c] = 0;
        b.edge[c + 1] = 0;
        assert!(!b.has_anonymous_lines());
    }
    #[test]
    fn w1_rejects_a_wrong_clue() {
        let mut b = board(&["1.2.1.2"]);
        let c = cell(&b, 1, 1);
        b.set_line(c, R);
        b.set_line(c + 1, R);
        assert!(!b.consistent);
    }
    #[test]
    fn w2_adjacent_interior_and_w3_wall_ray() {
        for ray in [false, true] {
            let mut b = board(&[".....", "1....", "....1"]);
            quiet(&mut b);
            let c = cell(&b, 2, 1);
            let q = c + 1;
            raw_line(&mut b, c, R);
            if ray {
                b.label[q + 1] = WALL;
                b.label[q + 2] = 1 | INSIDE;
            } else {
                let n = b.neighbor(q, D);
                b.label[n] = 1 | INSIDE;
            }
            b.advance_end(1, c, R);
            assert!(!b.consistent, "ray={ray}");
        }
    }
    #[test]
    fn w4_unused_ray_stops_at_unknown() {
        let mut b = board(&["1...1"]);
        let c = cell(&b, 1, 3);
        b.make_unused(c);
        assert!(b.consistent);
        let mut b = board(&["1...1"]);
        b.label[c - 1] = WALL;
        b.label[c + 1] = WALL;
        b.make_unused(c);
        assert!(!b.consistent);
    }
    #[test]
    fn c1_c2_all_sixteen_masks_incompatible_known_neighbors() {
        for mask in 0..16 {
            let mut b = board(&["1.1", "...", "..."]);
            let c = cell(&b, 2, 2);
            for d in 0..4 {
                let q = b.neighbor(c, d);
                b.label[q] = (d + 1) as u16;
                if mask & bit(d) != 0 {
                    b.set_noline(c, d);
                }
            }
            Arc::make_mut(&mut b.shared).southeast[c] = true;
            Arc::make_mut(&mut b.shared).southwest[c] = true;
            b.zero_cell(c);
            assert_eq!(b.label[c], WALL, "mask {mask:x}");
            assert!(b.consistent, "mask {mask:x}");
        }
    }
    #[test]
    fn c3_permissions_c4_straight_c5_third_exit_c6_asymmetry() {
        let mut b = board(&["...", "...", "1.1"]);
        let c = cell(&b, 1, 2);
        b.set_noline(c, L);
        b.set_noline(c, U); // mask 0x90, no SE clue on this diagonal
        Arc::make_mut(&mut b.shared).southeast[c] = false;
        b.zero_cell(c);
        assert_eq!(b.label[c], WALL);
        let mut b = board(&["1.1"]);
        let c = cell(&b, 1, 2);
        b.zero_cell(c);
        assert!(b.consistent);
        assert_eq!(b.done[1], 1);
        assert_eq!(b.edges(c) & 15, 10);
        let mut b = board(&["...", "...", "1.1"]);
        let c = cell(&b, 1, 2);
        let r = b.neighbor(c, R);
        let l = b.neighbor(c, L);
        let d = b.neighbor(c, D);
        b.label[r] = 1;
        b.label[d] = 257;
        b.label[l] = 2;
        b.zero_cell(c);
        assert_ne!(b.edges(c) & 0x80, 0); // C5 R~D bans L
        let mut b = board(&["...", "...", "1.1"]);
        let c = cell(&b, 1, 2);
        let r = b.neighbor(c, R);
        let l = b.neighbor(c, L);
        b.label[r] = 1;
        b.label[l] = 2;
        Arc::make_mut(&mut b.shared).southwest[c] = false;
        Arc::make_mut(&mut b.shared).southeast[c] = false;
        b.zero_cell(c);
        assert_ne!(b.edges(c) & 0x80, 0);
        assert_eq!(b.edges(c) & 0x20, 0); // C6 Z first
    }
    #[test]
    fn e1_direct_join_e2_occupied_and_e3_self_touch() {
        let mut b = board(&["11"]);
        let c = cell(&b, 1, 1);
        b.endpoint(c);
        assert_eq!(b.done[1], 1);
        let mut b = board(&["....", ".12.", "....", ".12."]);
        let c = cell(&b, 2, 2);
        b.endpoint(c);
        assert_ne!(b.edges(c) & 0x20, 0); // E2
        let mut b = board(&["....", ".1..", "....", "...1"]);
        let c = cell(&b, 2, 2);
        let n = cell(&b, 3, 3);
        b.label[n] = 1 | INSIDE;
        b.endpoint(c);
        assert_ne!(b.edges(c) & 0x20, 0);
        assert_ne!(b.edges(c) & 0x40, 0); // E3
    }
    #[test]
    fn e1_precedes_d2_forced_extension() {
        let p = Puzzle::parse(include_str!(
            "../tests/fixtures/end_check_before_extension.txt"
        ))
        .unwrap();
        assert_eq!(solve_none(&p, 0).solutions, 1);
    }
    #[test]
    fn p1_exclusive_wedge_and_slot8_exterior() {
        let b = board(&[".....", "..1..", ".....", "....1", "....."]);
        let c = cell(&b, 1, 1);
        let limit = b.shared.wedge[c][U] as usize;
        assert_eq!(limit, cell(&b, 1, 4)); // at k=3, sample (2,3) is a clue
        assert!(b.clear(c, cell(&b, 1, 3), D));
        assert!(!b.clear(c, limit, D));
        let c = cell(&b, 1, 5);
        assert_eq!(b.shared.wedge[c][L] as usize, cell(&b, 3, 5)); // right sentinel sampled
    }
    #[test]
    fn p2_diagonal_permission_and_unequal_clue_stop() {
        let b = board(&["1..", "...", "..1"]);
        assert!(b.shared.southeast[cell(&b, 2, 2)]);
        let b = board(&["1...", "..1.", ".23.", "2..3"]);
        assert!(!b.shared.southeast[cell(&b, 2, 2)]); // blank q, E=1 and S=2
        let b = board(&["2...", "..1.", ".12.", "...."]);
        assert!(b.shared.southeast[cell(&b, 2, 2)]); // equal intervening clues permit it
    }
    #[test]
    fn s1_s2_degree_and_downward_turn() {
        let mut b = board(&[".....", ".....", "1...1"]);
        let c = cell(&b, 1, 2);
        raw_line(&mut b, c, R);
        Arc::make_mut(&mut b.shared).southeast[c] = false;
        b.segment(c);
        assert_ne!(b.edges(c) & 0x40, 0);
        raw_line(&mut b, c, L);
        b.segment(c);
        assert_eq!(b.free(c), 0);
    }
    #[test]
    fn s3_side_ray_ignores_noline_and_s4_remote_bend() {
        let mut b = board(&[".....", ".....", ".....", ".....", "1...1"]);
        let c = cell(&b, 3, 3);
        raw_line(&mut b, c, L);
        let q = cell(&b, 1, 3);
        let middle = cell(&b, 2, 3);
        raw_line(&mut b, middle, U);
        raw_line(&mut b, q, L);
        assert_eq!(b.ray(c, U), Some((q, L)));
        b.set_noline(c, U);
        assert_eq!(b.ray(c, U), Some((q, L)));
        // Reset only the proposed edge: a ray is about the remote line shape.
        b.edge[c] &= !0x10;
        b.edge[middle] &= !0x40;
        Arc::make_mut(&mut b.shared).southwest[c] = true;
        b.segment(c);
        assert_ne!(b.edges(c) & 0x10, 0); // S3
        let mut b = board(&[".....", ".....", ".....", ".....", "1...1"]);
        let c = cell(&b, 3, 3);
        raw_line(&mut b, c, U);
        let q = cell(&b, 1, 3);
        let middle = cell(&b, 2, 3);
        raw_line(&mut b, middle, U);
        raw_line(&mut b, q, R);
        b.segment(c);
        assert_ne!(b.edges(c) & 0x20, 0); // S4
    }
    #[test]
    fn s5_two_arms_same_terminal_turn() {
        let mut b = board(&[".....", ".....", ".....", ".....", "1...1"]);
        let c = cell(&b, 3, 3);
        raw_line(&mut b, c, U);
        let top = cell(&b, 2, 3);
        raw_line(&mut b, top, R);
        let bottom = cell(&b, 4, 3);
        raw_line(&mut b, bottom, R);
        b.segment(c);
        assert_ne!(b.edges(c) & 0x40, 0); // S4 bans R, S5 bans D
    }
    #[test]
    fn k1_k2_k3_diagonal_chain_and_clue_stop() {
        let mut b = board(&["....", "....", "..1.", "1..."]);
        let q = cell(&b, 2, 2);
        Arc::make_mut(&mut b.shared).southeast[q] = true;
        b.diagonal(q, R);
        assert!(b.consistent);
        assert_eq!(b.edges(q) & 15, 6);
        assert_eq!(b.edges(q) & 0x90, 0x90);
        let clue = cell(&b, 3, 3);
        assert_eq!(b.edges(clue) & 15, 0);
        assert_eq!(b.edges(clue) & 0x90, 0x90);
        let mut b = board(&["....", "....", "..1.", "1..."]);
        let q = cell(&b, 2, 2);
        b.set_noline(q, D);
        b.diagonal(q, R);
        assert!(!b.consistent);
    }
    #[test]
    fn g1_g2_alternating_ports_and_escape() {
        // A one-cell zero region with four alternating active ends.
        let mut b = board(&[".1.", "2.2", ".1."]);
        let c = cell(&b, 2, 2);
        // Boundary order U,R,D,L = 1,2,1',2'. Confine every exit to the center.
        for d in 0..4 {
            let q = b.neighbor(c, d);
            for t in 0..4 {
                if b.neighbor(q, t) != c {
                    b.set_noline(q, t);
                }
            }
        }
        quiet(&mut b);
        assert!(!b.regions());
        let mut b = board(&[".1.", "2.2", ".1."]);
        let c = cell(&b, 2, 2);
        // Keep one unaccounted exit; G2 must not infer a crossing.
        for d in 1..4 {
            let q = b.neighbor(c, d);
            for t in 0..4 {
                if b.neighbor(q, t) != c {
                    b.set_noline(q, t);
                }
            }
        }
        assert!(b.regions());
    }
    #[test]
    fn undo_roundtrip_including_thirteen_bit_positions() {
        let mut b = board(&["1..", "...", "..1"]);
        let before = b.edge.clone();
        let labels = b.label.clone();
        let c = cell(&b, 1, 1);
        b.set_line(c, R);
        b.rollback_to(0);
        assert_eq!(b.edge, before);
        assert_eq!(b.label, labels);
        b.end_pos[1] = 4355;
        b.log(END, 1, 4355);
        b.end_pos[1] = 7;
        b.rollback_to(0);
        assert_eq!(b.end_pos[1], 4355);
    }
    #[test]
    fn maximum_dimensions_and_thirteen_bit_end_positions() {
        let mut p = Puzzle {
            width: 64,
            height: 64,
            pairs: 1,
            cells: vec![0; 4096],
        };
        p.cells[4094] = 1;
        p.cells[4095] = 1;
        let out = solve_none(&p, 0);
        assert_eq!(out.solutions, 1);
        assert_eq!(out.steps, 0);
        assert_eq!(out.first_solution_has_empty, Some(true));
    }
    #[test]
    fn solution_restored_to_original_orientation() {
        let p = Puzzle::parse("---\n1.1\n...\n---\n").unwrap();
        for k in 0..8 {
            let out = solve_none(&p, k);
            assert_eq!(out.solutions, 1);
            assert_eq!(out.first_solution.unwrap(), vec![2, 10, 8, 0, 0, 0]);
        }
    }
}
