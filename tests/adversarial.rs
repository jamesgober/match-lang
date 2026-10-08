//! Hostile input: deep and wide matches, matrices that need exponential work,
//! or-pattern explosions, large arm counts, and models that lie.
//!
//! Every test here must finish quickly and never panic or overflow the stack:
//! the answer is either correct or a budget error.

use std::cell::Cell;
use std::time::{Duration, Instant};

use match_lang::{Arm, Checker, Ctor, Error, Model, Pat, Signature, check};
use proptest::prelude::*;

/// Types are numbers: `0` is `bool`; `n > 0` is `Option` of type `n - 1`;
/// `CHAIN | n` is the one-field tuple `(CHAIN | n - 1,)`, with `CHAIN | 0` a
/// `bool`; `WIDE` is a tuple of `width` bools; `SEQ` a slice of bools.
const CHAIN: u32 = 1 << 30;
const WIDE: u32 = u32::MAX;
const SEQ: u32 = u32::MAX - 1;

struct Lang {
    width: usize,
}

impl Model for Lang {
    type Ty = u32;

    fn signature(&self, ty: &u32) -> Signature<u32> {
        match *ty {
            0 | CHAIN => Signature::Bool,
            WIDE => Signature::Product,
            SEQ => Signature::Slice { elem: 0 },
            n if n & CHAIN != 0 => Signature::Product,
            _ => Signature::Sum { count: 2 },
        }
    }

    fn fields(&self, ty: &u32, variant: u32, out: &mut Vec<u32>) {
        match *ty {
            WIDE => out.extend(std::iter::repeat_n(0, self.width)),
            n if n & CHAIN != 0 => out.push(n - 1),
            n if variant == 1 => out.push(n - 1),
            _ => {}
        }
    }
}

fn nested(depth: u32, leaf: Pat) -> Pat {
    let mut pat = leaf;
    for _ in 0..depth {
        pat = Pat::variant(1, [pat]);
    }
    pat
}

fn chain(depth: u32, leaf: Pat) -> Pat {
    let mut pat = leaf;
    for _ in 0..depth {
        pat = Pat::product([pat]);
    }
    pat
}

#[test]
fn test_deep_nesting_does_not_overflow() {
    const DEPTH: u32 = 100_000;
    let model = Lang { width: 0 };
    let arms = [Arm::new(nested(DEPTH, Pat::bool(true)))];
    let report = check(&model, &DEPTH, &arms).unwrap();
    assert_eq!(report.missing()[0].to_string(), "#0");
    assert!(report.unreachable().is_empty());
}

#[test]
fn test_every_level_covered_leaves_the_deepest_case() {
    const DEPTH: u32 = 1_000;
    let model = Lang { width: 0 };
    let mut arms: Vec<Arm> = (0..DEPTH)
        .map(|k| Arm::new(nested(k, Pat::variant(0, []))))
        .collect();
    arms.push(Arm::new(nested(DEPTH, Pat::bool(true))));
    let report = check(&model, &DEPTH, &arms).unwrap();
    let expected = format!(
        "{}false{}",
        "#1(".repeat(DEPTH as usize),
        ")".repeat(DEPTH as usize)
    );
    assert_eq!(report.missing()[0].to_string(), expected);
}

#[test]
fn test_deep_witness_prints_walks_and_drops() {
    const DEPTH: u32 = 100_000;
    let model = Lang { width: 0 };
    let arms = [Arm::new(chain(DEPTH, Pat::bool(true)))];
    let report = check(&model, &(CHAIN | DEPTH), &arms).unwrap();
    let w = &report.missing()[0];
    let text = w.to_string();
    assert_eq!(text.len(), 5 + 3 * DEPTH as usize);
    assert!(text.starts_with("((("));
    let mut node = w.root();
    for _ in 0..DEPTH {
        node = node.fields().next().unwrap();
    }
    assert_eq!(node.ctor(), Ctor::Bool(false));
    let copy = report.clone();
    drop(report);
    drop(copy);
}

#[test]
fn test_wide_tuple_is_linear() {
    const WIDTH: usize = 100_000;
    let model = Lang { width: WIDTH };
    let row = |value: bool| {
        Pat::product((0..WIDTH).map(|i| {
            if i == WIDTH / 2 {
                Pat::bool(value)
            } else {
                Pat::wild()
            }
        }))
    };
    let arms = [
        Arm::new(row(true)),
        Arm::new(row(false)),
        Arm::new(Pat::wild()),
    ];
    let start = Instant::now();
    let report = check(&model, &WIDE, &arms).unwrap();
    assert!(report.is_exhaustive());
    assert_eq!(report.unreachable(), &[2]);
    assert!(start.elapsed() < Duration::from_secs(5));
    // Linear: a few steps per cell, not per cell squared.
    assert!(
        report.steps() < 40 * WIDTH as u64,
        "steps {}",
        report.steps()
    );
}

#[test]
fn test_diagonal_matrix_is_polynomial() {
    // Row i tests only column i. Without row relevancy every column's
    // `true` branch re-explores all later rows: 2^1000 paths.
    const N: usize = 1_000;
    let model = Lang { width: N };
    let mut arms: Vec<Arm> = (0..N)
        .map(|r| {
            Arm::new(Pat::product(
                (0..N).map(|c| if c == r { Pat::bool(true) } else { Pat::wild() }),
            ))
        })
        .collect();
    arms.push(Arm::new(Pat::wild()));
    let report = check(&model, &WIDE, &arms).unwrap();
    assert!(report.is_exhaustive());
    assert!(report.unreachable().is_empty());
    assert!(
        report.steps() < 10 * (N * N) as u64,
        "steps {}",
        report.steps()
    );
}

#[test]
fn test_wide_witness_has_every_field() {
    const WIDTH: usize = 50_000;
    let model = Lang { width: WIDTH };
    let row = Pat::product((0..WIDTH).map(|i| {
        if i + 1 == WIDTH {
            Pat::bool(true)
        } else {
            Pat::wild()
        }
    }));
    let report = check(&model, &WIDE, &[Arm::new(row)]).unwrap();
    let w = &report.missing()[0];
    assert_eq!(w.root().arity(), WIDTH);
    assert_eq!(w.fields().last().unwrap().ctor(), Ctor::Bool(false));
}

/// Rows from a random 3-CNF formula over `vars` booleans: the matrix is
/// exhaustive exactly when the formula is unsatisfiable, so near the
/// satisfiability threshold the checker has to work hard.
fn sat_arms(vars: usize, clauses: usize, seed: u64) -> Vec<Arm> {
    let mut state = seed;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    (0..clauses)
        .map(|_| {
            let mut cells: Vec<Pat> = (0..vars).map(|_| Pat::wild()).collect();
            for _ in 0..3 {
                let var = (next() % vars as u64) as usize;
                cells[var] = Pat::bool(next() % 2 == 0);
            }
            Arm::new(Pat::product(cells))
        })
        .collect()
}

#[test]
fn test_hard_matrix_hits_the_budget_quickly() {
    let model = Lang { width: 60 };
    let arms = sat_arms(60, 255, 0x5EED);
    let start = Instant::now();
    let result = Checker::new()
        .with_step_limit(200_000)
        .check(&model, &WIDE, &arms);
    assert_eq!(result.unwrap_err(), Error::StepLimit { limit: 200_000 });
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn test_hard_matrix_with_default_budget_terminates() {
    let model = Lang { width: 60 };
    let arms = sat_arms(60, 255, 0xFACE);
    let start = Instant::now();
    match check(&model, &WIDE, &arms) {
        Ok(_) | Err(Error::StepLimit { .. }) => {}
        Err(other) => panic!("unexpected {other:?}"),
    }
    assert!(start.elapsed() < Duration::from_secs(20));
}

#[test]
fn test_or_explosion_hits_the_budget() {
    // (true | false, true | false, ...) over 40 columns: 2^40 expansions.
    let model = Lang { width: 40 };
    let either = || Pat::or([Pat::bool(true), Pat::bool(false)]);
    let arms = [
        Arm::new(Pat::product((0..40).map(|_| either()))),
        Arm::new(Pat::wild()),
    ];
    let start = Instant::now();
    let result = check(&model, &WIDE, &arms);
    assert!(matches!(result, Err(Error::StepLimit { .. })));
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[test]
fn test_or_explosion_in_one_column_is_linear() {
    // A single or with 100k alternatives is just 100k rows.
    let model = Lang { width: 0 };
    let alternatives = (0..100_000).map(|i| {
        if i % 2 == 0 {
            Pat::bool(true)
        } else {
            Pat::bool(false)
        }
    });
    let report = check(&model, &0, &[Arm::new(Pat::or(alternatives))]).unwrap();
    assert!(report.is_exhaustive());
    assert_eq!(report.redundant().len(), 99_998);
}

struct Ints;

impl Model for Ints {
    type Ty = ();
    fn signature(&self, _: &()) -> Signature<()> {
        Signature::Int {
            min: i64::MIN.into(),
            max: i64::MAX.into(),
        }
    }
    fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
}

#[test]
fn test_many_literal_arms() {
    let mut arms: Vec<Arm> = (0..100_000).map(|i| Arm::new(Pat::int(i * 3))).collect();
    arms.push(Arm::new(Pat::int(300)));
    let report = check(&Ints, &(), &arms).unwrap();
    assert_eq!(report.unreachable(), &[100_000]);
    assert_eq!(
        report.missing()[0].ctor(),
        Ctor::Int {
            lo: i64::MIN.into(),
            hi: -1
        }
    );
}

#[test]
fn test_many_overlapping_ranges_stay_within_budget() {
    // Nested ranges: every segment is covered by a different set of rows.
    let arms: Vec<Arm> = (0..2_000).map(|i| Arm::new(Pat::range(-i, i))).collect();
    let report = check(&Ints, &(), &arms).unwrap();
    assert!(report.unreachable().is_empty());
}

#[test]
fn test_long_slice_patterns() {
    let model = Lang { width: 0 };
    let long = Pat::slice((0..20_000).map(|_| Pat::wild()));
    let arms = [Arm::new(long), Arm::new(Pat::slice_rest([], []))];
    let report = check(&model, &SEQ, &arms).unwrap();
    assert!(report.is_exhaustive());
    // Many lengths, each of many elements: quadratic by nature, so budgeted.
    let arms = [
        Arm::new(Pat::slice_rest((0..20_000).map(|_| Pat::wild()), [])),
        Arm::new(Pat::slice_rest([], [])),
    ];
    match check(&model, &SEQ, &arms) {
        Ok(report) => assert!(report.is_exhaustive()),
        Err(err) => assert!(matches!(err, Error::StepLimit { .. })),
    }
}

// ---------------------------------------------------------------------------
// A model that answers at random
// ---------------------------------------------------------------------------

struct Liar {
    state: Cell<u64>,
}

impl Liar {
    fn roll(&self, n: u64) -> u64 {
        let mut s = self.state.get();
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        self.state.set(s);
        s % n
    }
}

impl Model for Liar {
    type Ty = u8;

    fn signature(&self, _: &u8) -> Signature<u8> {
        match self.roll(9) {
            0 => Signature::Bool,
            1 => Signature::Product,
            2 => Signature::Sum {
                count: self.roll(4) as u32,
            },
            3 => Signature::Int {
                min: -(self.roll(3) as i128),
                max: self.roll(3) as i128,
            },
            4 => Signature::Char,
            5 => Signature::Opaque,
            6 => Signature::Array {
                len: self.roll(3) as u32,
                elem: 0,
            },
            7 => Signature::Slice { elem: 0 },
            _ => Signature::BigInt,
        }
    }

    fn fields(&self, _: &u8, _: u32, out: &mut Vec<u8>) {
        for _ in 0..self.roll(3) {
            out.push(0);
        }
    }

    fn is_empty(&self, _: &u8) -> bool {
        self.roll(4) == 0
    }

    fn variant_name(&self, _: &u8, _: u32) -> Option<&str> {
        if self.roll(2) == 0 { Some("V") } else { None }
    }
}

fn random_pat(rng: &mut u64, depth: u32) -> Pat {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    let r = *rng;
    let kids = |rng: &mut u64, n: u64| -> Vec<Pat> {
        if depth == 0 {
            Vec::new()
        } else {
            (0..n).map(|_| random_pat(rng, depth - 1)).collect()
        }
    };
    match r % 11 {
        0 => Pat::wild(),
        1 => Pat::or(kids(rng, r / 11 % 3)),
        2 => Pat::variant((r / 11 % 3) as u32, kids(rng, r / 33 % 3)),
        3 => Pat::product(kids(rng, r / 11 % 3)),
        4 => Pat::bool(r & 1 == 0),
        5 => Pat::range(-((r / 11 % 3) as i128), (r / 33 % 3) as i128),
        6 => Pat::char_range('a', 'c'),
        7 => Pat::lit(r / 11 % 3),
        8 => Pat::slice(kids(rng, r / 11 % 3)),
        9 => {
            let all = kids(rng, r / 11 % 3);
            let half = all.len() / 2;
            let mut all = all;
            let post = all.split_off(half);
            Pat::slice_rest(all, post)
        }
        _ => Pat::int((r / 11 % 5) as i128 - 2),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 3000, .. ProptestConfig::default() })]

    #[test]
    fn prop_lying_model_never_panics(seed in 1u64.., pat_seed in 1u64.., arms in 0usize..6) {
        let model = Liar { state: Cell::new(seed) };
        let mut rng = pat_seed;
        let arms: Vec<Arm> = (0..arms).map(|i| {
            let pat = random_pat(&mut rng, 3);
            if i % 3 == 2 { Arm::guarded(pat) } else { Arm::new(pat) }
        }).collect();
        if let Ok(report) = Checker::new().with_step_limit(100_000).check(&model, &0, &arms) {
            for w in report.missing() {
                let _ = w.to_string();
                let _ = w.display(&model).to_string();
                let _ = w.fields().count();
            }
        }
    }

    #[test]
    fn prop_random_patterns_on_a_consistent_model_never_panic(pat_seed in 1u64.., arms in 0usize..6, ty in 0u32..4) {
        let model = Lang { width: 2 };
        let mut rng = pat_seed;
        let arms: Vec<Arm> = (0..arms).map(|_| Arm::new(random_pat(&mut rng, 3))).collect();
        match check(&model, &ty, &arms) {
            Ok(_) | Err(Error::InvalidPattern { .. }) => {}
            Err(other) => prop_assert!(false, "unexpected {other:?}"),
        }
    }
}
