# match-lang - Roadmap

> Path from scaffold to a stable 1.0. Hard parts are front-loaded; each phase has hard exit criteria.
> Master plan: ../_lexersketch/ROADMAP.md and ../_lexersketch/NEW-LIBS.md
>
> **Anti-deferral rule:** no listed hard task moves to a later phase unless this file records the move and the reason.

## v0.1.0 - Scaffold (DONE)
Compiles, CI green, structure correct, no domain logic.
- [x] Manifest, README, CHANGELOG, REPS, dual license, CI, deny, clippy, rustfmt, DIRECTIVES, ROADMAP.

## v0.2.0 - Foundation
- [ ] Constructor model trait, pattern matrix, usefulness algorithm, exhaustiveness with witnesses.
- [ ] Property tests against a brute-force reference over small value spaces.

## v0.5.0 - Implementation
- [ ] Decision-tree compilation with sharing; ranges, slices, or-patterns, guards; benchmarks.

## v0.9.0 - Hardening
- [ ] Adversarial matrices, budgets, fuzzing.

## v1.0.0 - Stable
- [ ] Frozen after typeck-lang and HIR lowering to SSA use it (D18).
