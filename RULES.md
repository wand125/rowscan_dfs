# Numberlink pruning and propagation rule catalogue

This catalogue defines the pruning and propagation rules used by rowscan_dfs.
Paths connect equal clues, do not intersect, and may leave cells unused.
**D** rules are ordinary deductions from path validity. **K** rules select
shortcut normal forms and can remove valid raw routes. R1 and H1 choose search
order. The data layout and execution machinery are described in
[DESIGN.md](DESIGN.md); options and defaults are in [README.md](README.md).

## Definitions and canonical form

Directions `U,R,D,L` have bits `1,2,4,8`. With the solver's padded stride
`S = width + 2`, their neighbor offsets are `-S,+1,+S,-1`. Coordinates below
are 1-based interior `(y,x)`; row-major positions are `y*S+x`.
An edge word's low nibble is Line and high nibble is NoLine.
`F(c)` is the Undecided set, `B(c)` the NoLine set, and `L(c)` the Line set.
A wall **cell** is an unused vertex, distinct from a NoLine edge.

`state(c)` distinguishes:

- `0`: unclaimed, including line segments not yet reached from a clue;
- `n` / `n^0x100`: the two active ends of pair `n` (1–254);
- `id|0x200`: a claimed internal or finished vertex;
- `0x3ff`: unused or exterior.

`mate(a,b)` means `state(a) == (state(b)^0x100)`. Internal vertices do not
qualify as active mates. `known(a)` means `state(a)!=0`, including occupied
and unused vertices. A degree-one anonymous segment is still unknown.
`A(c)` is permission toward a clue southeast; `Z(c)` is permission toward a
clue southwest. Both are defined under diagonal permissions below.

The **shortcut normal form** is the set of completed paths accepted by the
ordered rules below, including the K restrictions in R2, W2–W4, C3–C6, E1/E3,
S2–S5 and K1–K3. These choose direct joins and prohibit local or clue-free
wedge detours. The form is relative to the transformed scan orientation, so
canonical solution counts can differ between directions. It is not an
enumeration of all geometrically distinct raw paths. `--diag-shortcut` adds
the stricter diagonal restriction defined below and may further reduce counts.
Cut checks and failed-frontier memoization preserve the count for a fixed
orientation and canonicalization setting. Unused cells remain valid throughout;
there is no full-coverage requirement.

## Scan decisions, degree rules, and path walks

| ID | Trigger | Condition and effect | Kind |
|---|---|---|---|
| R1 | Next row-major decision | Skip internal/finished cells and unclaimed cells with two Lines. At an unclaimed zero-line cell, try Right+Down, then unused. At a one-line cell or active end, try Right, then Down. These are choices, not deductions. | Heuristic |
| R2 | Zero-line Right+Down choice | After Right, forbid Down at the right neighbor and Right at the lower neighbor, preventing a three-edge detour around the elementary square. Require Down at the scanned cell to give degree two; reject if forbidden. | K for square bans; D for degree |
| R3 | Anonymous Line insertion | Follow existing Lines, excluding the edge just used. Reject a walk that returns to its starting vertex, forming a clue-less loop. Stop at an open end. | D |
| D1 | Unclaimed one-line worklist cell | If `|F|>1`, wait; if zero, reject; if one, add that Line, check incident shapes, then follow any attached labelled path or detect a loop. | D |
| D2 | Active-end worklist cell | The end needs one further Line: zero exits rejects, one forces extension, more than one waits. Directions are considered U,R,D,L; a blocked same-value direct join before the sole free direction rejects under E1. | D; E1 restriction is K |
| D3 | Immediately after Line placement | A clue with one Line, or a non-clue with two Lines, forbids every remaining Undecided incident edge. | D |
| W1 | Walking from an active end along Lines | Entering an occupied vertex succeeds only if its state is exactly the current ID's mate. Otherwise reject. A successful join finishes the pair. On unclaimed vertices, claim the segment and advance the end until no forward Line remains. | D |
| W2 | Before claiming an unclaimed vertex | If any neighbor other than the predecessor has `(state&0x2ff)==((id&255)\|0x200)`, reject: this route touches its own claimed interior and admits a shortcut. | K |
| W3 | After claiming a vertex | In each direction beginning with an unused cell, skip the maximal unused run. Reject if the first non-unused interior vertex is claimed and has the current pair's low-byte value. An unclaimed vertex stops the test without failure. | K |
| W4 | A cell becomes unused | In its row and column, skip unused cells in both directions. Reject if the first non-unused vertices on opposite sides are claimed and have equal low-byte pair values. Unknown state 0 stops the test. Forbid all remaining incident edges. | K for the shortcut; D for isolation |
| D4 | All pairs complete | Reject if any unclaimed cell still carries a Line. Otherwise record a solution, including ones with unused cells. Exhausting a recorded solution is not a contradiction. | D |

## Zero-line worklist rules

These apply only to an unclaimed zero-line cell `c`. The high-nibble NoLine
mask selects the ordered dispatch below. Two remaining edges alone do not
force two Lines because the cell may be unused.

| ID | Condition and effect | Kind |
|---|---|---|
| C1 | In the non-diagonal-special cases below, all relevant usable neighbors are known and no pair are mates. A route through `c` would have to join two compatible active ends; none exist, so mark `c` unused and apply W4. | D; W4 remains K |
| C2 | At most one usable direction remains: mark `c` unused and apply W4. | D; W4 remains K |
| C3 | Usable directions form a downward corner (`0x30` or `0x90`). If its diagonal permission is zero, mark `c` unused. Otherwise do so only when both usable neighbors are known and are not mates. | K for missing permission; D for incompatible labels |
| C4 | Opposite usable neighbors are mates: force the straight connection through `c`, then apply the walk/end rules. | K |
| C5 | With three usable directions and two neighboring ends that are mates, forbid the third edge, selecting their direct two-edge connection if `c` is used. | K |
| C6 | With Up NoLine, known non-mate Left/Right neighbors and unknown Down: `Z(c)==0` bans Left; otherwise `A(c)==0` bans Right. With Right NoLine, known non-mate Left/Up and unknown Down: `Z(c)==0` bans Left. With Left NoLine, known non-mate Right/Up and unknown Down: `A(c)==0` bans Right. | K |

Here `X~Y` means the neighboring cells in those directions are mates.
Conditions are evaluated in the listed order. `wait` returns without changing
this cell; `empty` marks it unused and applies W4 and incident NoLine propagation.

| NoLine mask | Usable directions | Ordered conditions / action |
|---|---|---|
| `0x00` | U R D L | Wait if any neighbor is unknown, or any two are mates; otherwise C1 empty. |
| `0x10` | R D L | Wait if R or L unknown. If R~L: C4 force LR. If D known: R~D ⇒ C5 ban L; L~D ⇒ C5 ban R; otherwise C1 empty. If D unknown: C6 `Z==0` ⇒ ban L, else `A==0` ⇒ ban R, else wait. |
| `0x20` | U D L | Wait if L or U unknown. L~U ⇒ C5 ban D. If D unknown: C6 `Z==0` ⇒ ban L, else wait. If D known: L~D ⇒ C5 ban U; U~D ⇒ C4 force UD; otherwise C1 empty. |
| `0x30` | D L | C3: `Z==0` ⇒ empty. Otherwise wait if D or L unknown or D~L; otherwise empty. |
| `0x40` | U R L | Wait if R or L unknown. R~L ⇒ C4 force LR. Wait if U unknown. R~U ⇒ C5 ban L; L~U ⇒ C5 ban R; otherwise C1 empty. |
| `0x50` | R L | Wait if R or L unknown. R~L ⇒ C4 force LR; otherwise C1 empty. |
| `0x60` | U L | Wait if L or U unknown, or L~U; otherwise C1 empty. |
| `0x70` | L | C2 empty. |
| `0x80` | U R D | Wait if R or U unknown. R~U ⇒ C5 ban D. If D known: R~D ⇒ C5 ban U; U~D ⇒ C4 force UD; otherwise C1 empty. If D unknown: C6 `A==0` ⇒ ban R, else wait. |
| `0x90` | R D | C3: `A==0` ⇒ empty. Otherwise wait if D or R unknown or D~R; otherwise empty. |
| `0xa0` | U D | Wait if D or U unknown. U~D ⇒ C4 force UD; otherwise C1 empty. |
| `0xb0` | D | C2 empty. |
| `0xc0` | U R | Wait if R or U unknown, or R~U; otherwise C1 empty. |
| `0xd0` | R | C2 empty. |
| `0xe0` | U | C2 empty. |
| `0xf0` | none | C2 empty; no new edges, but W4 and region invalidation still apply. |

For example, C5 is not degree propagation:

```text
      #              #        # = NoLine side of c
 a -- c -- a'   => a--c--a'    a,a' = matching active ends
      ?              #        third edge becomes NoLine
```

## Active-end rules

Normal end checks run initially on clue ends and after a labelled walk reaches
an open end, considering that end and then its mate. Directions are examined
U,R,D,L. Single-exit worklist forcing is D2.

| ID | Condition and effect | Kind |
|---|---|---|
| E1 | An adjacent vertex has the same low-byte pair value and the edge is not already Line: force it Line. A NoLine conflict rejects. W1 still requires an actual mate, rejecting same-value internal vertices. | K |
| E2 | For each other Undecided direction, if its neighbor is occupied, forbid that edge. | D |
| E3 | For an unclaimed neighbor, if any neighbor of that cell other than the current end is an internal vertex of the current pair, forbid the proposed extension. This anticipates W2. | K |

Each extension applies S1–S5, W1–W3 and K1–K4 as appropriate. These shared
checks do not define extra rules for each caller.

## Clue-dependent precomputation

### Wedge limits

The immutable clue array is transformed before computing four limits at each
cell. Each is the first blocking coordinate, with an exclusive comparison in
consumers. The direction names below index the current `wedge[c]` array.
For blank `c=(y,x)`, consider candidate distances `k=2,3,...` inside the board:

| Index | Candidate coordinate | Clue samples at distance k |
|---|---|---|
| U | `(y,x+k)` | `(y+j,x+k-j)`, `j=1..floor(k/2)` |
| D | `(y,x+k)` | `(y-j,x+k-j)`, `j=1..floor(k/2)` |
| R | `(y+k,x)` | `(y+k-j,x-j)`, `j=1..floor(k/2)` |
| L | `(y+k,x)` | `(y+k-j,min(x+j,width+1))`, `j=1..floor(k/2)` |

The first candidate with a nonzero original-clue sample sets the limit.
If none blocks, use the right or bottom exterior coordinate. A starting clue
gets these exterior defaults without sampling. Sample loops for U, D and R
stop when they leave the bottom, top or left interior respectively. The L
samples instead read the nonzero right sentinel at `x=width+1`; reaching it
blocks that candidate. This right-boundary asymmetry is part of the definition.

The samples identify whether a bend can be pulled through a clue-free wedge.
Using row-major positions for aligned ray ends:

```text
clear(a,b,t):
    lo,hi = min(a,b),max(a,b)
    return hi < wedge[lo][opposite(t)]
```

The strict `<` matters: the first blocking coordinate prevents pruning.
Limits do not change when cells become unused or edges change. They enable
K deductions S3–S5 and generate no edges or contradictions by themselves.

### Diagonal permissions

Start with `A` and `Z` zero. From each original clue:

- Walk northwest from its NW neighbor. At blank `q`, stop if the East and
  South original clues are both nonzero and unequal. Otherwise set `A(q)=1`
  and continue northwest. Stop at a clue or exterior.
- Walk northeast from its NE neighbor. At blank `q`, stop if the West and
  South original clues are both nonzero and unequal. Otherwise set `Z(q)=1`
  and continue northeast. Stop at a clue or exterior.

A permission means that the downward diagonal can reach a clue without this
intervening unequal-clue obstruction. Missing permission enables the downward
turn bans in C3/C6 and S2. This is a K enabler, with no direct deductions.

## Segment rules

These run at incident one-line blank cells immediately after Line insertion;
clues and saturated cells use S1. Let `i` be the incident Line's direction.
The geometry helper is:

```text
ray(c,d):
    q = c
    repeat:
        q = neighbor(q,d)
        m = L(q) without opposite(d)
        if m == 0: return none
        if m != bit(d):
            if m has exactly one bit: return (q,direction(m))
            return none
```

This follows Line geometry, including unclaimed segments. It differs from the
unused-cell rays of W3/W4. The candidate edge is checked separately; `ray`
does not require matching labels or connected-component identity at its ends.

| ID | Condition and effect | Kind |
|---|---|---|
| S1 | A clue or two-line non-clue forbids all remaining Undecided incident directions and returns (D3). | D |
| S2 | At a one-line blank: `i=R` and `A(c)=0` ⇒ ban D; `i=L` and `Z(c)=0` ⇒ ban D; `i=D` and `A(c)=0` ⇒ ban R, then `Z(c)=0` ⇒ ban L. `i=U` has no initial ban. | K |
| S3 | For an Undecided perpendicular side `s`, if `ray(c,s)=(q,i)` and `clear(c,q,i)`, forbid `c--s`. This prevents joining a side ray whose distant turn points in the incident Line's direction. | K |
| S4 | Let `ray(c,i)=(q,t)`. If `c--t` is Undecided and `clear(c,q,t)`, forbid `c--t`: a turn toward the distant bend of the existing arm. | K |
| S5 | With that same `(q,t)`, let `f=opposite(i)`. If `c--f` is Undecided, `ray(c,f)=(r,t)`, and `clear(q,r,t)`, forbid `c--f`. Both arms bend in the same direction. | K |

S3 sides are checked U,D for horizontal `i`; L,R for `i=D`; R,L for `i=U`.
After S2, return if no free edge remains. After S3, skip S4/S5 if no free edge
remains, or if the only free side is `opposite(i)` and its neighbor has no
Line. These are evaluation gates. Each ban queues affected cells; a subsequent
zero-exit failure is D1/D2. Remote one-line origins are not globally rescanned.

## Diagonal corner chains

An RD corner northwest of `q` starts the RD chain. A DL corner northeast of
`q` starts its mirror. Chains may be triggered by scan decisions, forced
moves or end extensions.

| ID | Condition and effect | Kind |
|---|---|---|
| K1 | At entry, the RD chain forbids U and L at `q`; the DL chain forbids U and R. These point toward the existing corner's arms. | K |
| K2 | If `q` has an original clue, queue it and stop. If blank and either required outgoing edge (R,D for RD; L,D for DL) is NoLine, reject. An already complete required corner stops successfully. | K-derived failure |
| K3 | Otherwise install the backward bans at the next diagonal cell, force missing outgoing Lines at `q`, and apply segment/walk checks. Advance by `(1,1)` for RD or `(1,-1)` for DL and repeat K2. | K |
| K4 | If a newly formed anonymous segment closes back onto itself, reject the clue-less loop (R3). | D |

## Region-boundary crossing

### G1: dirty-end scheduling

After initial propagation and each successful propagation phase, visit dirty
ends in pair order. For incomplete pairs, consider dirty Undecided exits into
state-0 neighbors. For completed pairs, consider dirty directions into state-0
neighbors. Seed G2 on that neighbor's side facing the end. Clear visited dirty
flags. A contour already visited in the current pass is skipped. Moving ends,
changed incident edges, pair completion and newly unused cells invalidate the
relevant ends. G1 schedules checks and forces nothing.

### G2: alternating four-end rejection

Trace a contour of the geometric union of state-0 cells, turning at
occupied/unused/exterior cells. Holes have separate contours. Record each
boundary contact `(id,d)` at an active labelled end whose contacting edge is
not NoLine; `d` points from the end into the region. Internal and unused cells
are not contacts. Retain repeated contacts and their cyclic order.
Only test pairs whose two end IDs both appear on this contour.

For every two such pairs `a,a'` and `b,b'`:

1. Initialize four residual masks to `F(end(id))`.
2. Extract their contacts in boundary order, retaining repetitions. XOR each
   contact direction into that end's residual mask.
3. Starting at the first extracted contact, count transitions between distinct
   IDs, ignoring consecutive repeats. Abandon this pair-of-pairs test if a
   transition directly joins mate IDs, or if more than four transitions occur.
4. Reject iff all four residual masks are zero, the last ID is not the first
   ID's mate, and either the last ID equals the first or fewer than four
   transitions occurred.

This is a D rule: alternating connections `a b a' b'` confined to the same
face must cross. The all-exits condition is necessary; an end with another
exit can escape the region. G2 generates no forced edges.

## Optional cut and diagonal restrictions

| Option | Condition and effect | Kind |
|---|---|---|
| `--cut N` | At zero-based scan columns 0, N, 2N, … (`0` disables), reject if any row/column boundary has more incomplete pairs with current ends on opposite sides than available crossings. Count Undecided crossing edges and anonymous Line bridges, including above the scan frontier. Completed pairs contribute no demand. | D |
| `--diag-shortcut` | Around an unused cell, reject if two orthogonal neighbors belong to the same final line and their route must exceed two edges. Preserve existing two-edge corners and possible corner joins between separate halves of a clue. Recognize connected anonymous segments too. At early completion, unclaimed zero-degree cells also qualify as unused. | K |

Cut capacities are recomputed at each check; both options read the board
without extra undo state. Failed cut checks add one search step. Diagonal
checks run after propagation/G2 and before solution recording; they preserve
existence and the first-solution empty-cell classification, while counts may
decrease. Defaults are specified in README.md.

## Failed-frontier memo semantics

A memo hit rejects a state whose failed subtree has already been exhausted.
Keys are recomputed when the cursor reaches a deeper row after successful
propagation. The key includes the cursor, all edge/label state from two rows
above through the bottom, earlier anonymous cells with positions, upward
column rays through blank/unused cells to the first labelled cell, incomplete
end positions, and all completion flags.

An entry closes when the cursor retreats above its row, including after
failure, or when search ends. Store only if no solution was found beneath it
and no part of it was donated to another worker. Donation taints open donor
entries; a recipient starts with no open entries and shares the same table.
An exact 64-bit key match is a hit; a slot-index collision replaces the entry.
Memo changes work and counters, preserving canonical counts. With memo off,
parallel work partitions the serial alternatives and preserves their total
step count; with memo on, scheduling can change hits and steps.

## Direction recommendation (H1)

H1 is a heuristic with no forced edges or contradictions. For transforms 0–7,
bit 0 transposes, bit 1 reflects original x, and bit 2 reflects original y;
reflections occur before transposition. Let `sx` and `sy` be the sums of
1-based coordinates of all `2*pairs` clues.

A non-transposed candidate requires `width<=height` and a clue centroid in
the first half of the scan: unflipped y requires
`sy<=(height+1)*pairs`, flipped y the reverse inequality. A transposed
candidate requires `height<=width` and the analogous x inequality controlled
by bit 1. Equality admits both orientations.

For each eligible transform, scan only the first
`floor(transformed_height/2)` rows. At each cell, add the cumulative number of
clues encountered to `score`, and the number of pairs seen exactly once to
`tie`. Select the lexicographically greatest `(score,tie)` using strict
improvement, initial best `(0,0)` and initial direction 0. Exact ties retain
the earlier transform. The bundled ex1 and ex3 examples select direction 6.
