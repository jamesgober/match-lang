# match-lang - Roadmap

> Path from scaffold to a stable 1.0. Hard parts are front-loaded; each phase has hard exit criteria.
> Master plan: ../_lexersketch/ROADMAP.md and ../_lexersketch/NEW-LIBS.md
>
> **Anti-deferral rule:** no listed hard task moves to a later phase unless this file records the move and the reason.

## v0.1.0 - Scaffold (DONE)
Compiles, CI green, structure correct, no domain logic.
- [x] Manifest, README, CHANGELOG, REPS, dual license, CI, deny, clippy, rustfmt, DIRECTIVES, ROADMAP.

## v0.2.0 - Foundation (DONE)
- [x] Constructor model trait, pattern matrix, usefulness algorithm, exhaustiveness with witnesses.
- [x] Property tests against a brute-force reference over small value spaces.

Delivered:
- `Model` (constructor model: `signature`, `fields`, `is_empty`,
  `variant_name`) and `Signature` (`Sum`, `Product`, `Bool`, `Int`, `Char`,
  `BigInt`, `Opaque`, `Array`, `Slice`).
- `Pat` (wild, variant, product, bool, int, range, char, char_range, lit, or,
  slice, slice_rest) and `Arm` (with or without a guard).
- `check` (Tier 1) and `Checker` (configurable step and witness limits,
  buffers reused across checks); `Report` (missing witnesses, unreachable
  arms, redundant or-alternatives by preorder node, steps used); `Witness`
  trees of `Ctor` nodes with types, printable with the model's names;
  `Error` and `Reason`.
- The usefulness algorithm (Maranget 2007) with constructor splitting, as
  modern compilers run it: one pass over all rows, a single "missing"
  constructor per column, witnesses built on the way back, row relevancy
  (wildcard-headed rows not explored under constructors examined only for
  usefulness, trailing irrelevant rows dropped), so ordinary matrices such as
  one-test-per-row stay polynomial.
- Ranges: integer and character domains cut into disjoint segments in one
  sorted sweep; characters analysed over a gap-free index space so witnesses
  are always valid scalar values; unbounded integers and opaque literals
  carry an always-missing `Other`.
- Or-patterns expanded in place in left-to-right preorder; per-alternative
  usefulness with only the outermost redundant alternative reported. Guards
  possibly failing, re-tried per alternative (Rust's semantics).
- Empty types: constructors with an empty field are neither required nor
  reachable; slices of an empty type have only `[]`.
- Hardening: explicit frame stack (no recursion anywhere input reaches,
  including `Pat` and witness `Drop`/`Clone`/`Display`), rows as persistent
  stacks sharing tails, stack-disciplined arenas, a step budget on all work,
  model inconsistencies reported as `Error::Model`.
- Verified against a brute-force reference (exhaustiveness, every witness,
  unreachable arms, redundant alternatives) over random types and matches;
  200,000 cases per property in a one-off run, and every planted engine bug
  caught. Adversarial tests: 100k-deep patterns and witnesses, 100k-field
  tuples, 3-SAT matrices, or-explosions, a model answering at random.

Moved forward from later phases (done here, not deferred):
- **Slices** (listed for 0.5.0) are delivered in 0.2.0, fixed and variable
  length, with interval-grouped length splitting.
- **Budgets and adversarial matrices** (listed for 0.9.0) are delivered in
  0.2.0: the step budget is what makes the algorithm safe at all. 0.9.0 keeps
  the fuzzing work.

Dependency wiring (decided here, recorded per the anti-deferral rule):
- **No first-party dependency.** The crate is generic over the language's
  types through `Model`; `hir-lang` (and any other front end) implements the
  trait, so depending on it would invert the layering. `intern-lang` symbols
  fit `Pat::lit` ids directly without a dependency.
- **diag-lang: not wired.** Reports are plain data (arm indices, preorder
  node indices, witness trees); turning them into diagnostics needs the
  language's spans, which only the language has. A conversion helper can be
  added once a consumer shows the shape it needs.

## v0.5.0 - Implementation
- [ ] Decision-tree compilation with sharing over the same constructor model;
      trees agree with first-match semantics on every value (property-tested
      against the brute-force reference); tree size budgeted; benchmarks.
- [x] Ranges, slices, or-patterns, guards (delivered in 0.2.0 for checking;
      0.5.0 reuses the splitting for compilation).

## v0.9.0 - Hardening
- [x] Adversarial matrices, budgets (delivered in 0.2.0).
- [ ] Fuzzing (cargo-fuzz targets over random models and patterns, beyond the
      proptest-driven hostile-model tests).

## v1.0.0 - Stable
- [ ] Frozen after typeck-lang and HIR lowering to SSA use it (D18).
