//! Property tests against a brute-force reference.
//!
//! Random types and random matches over them are checked two ways: by the
//! crate, and by enumerating every value of the type and running first-match
//! semantics directly. Infinite and very large types are enumerated through
//! representatives: for integers and characters, every boundary any pattern
//! mentions (and its neighbours), which hits every segment the patterns can
//! tell apart; for opaque literals, every literal mentioned plus one fresh
//! value; for slices, every length up to the point past which all lengths
//! behave alike.
//!
//! The reference decides, for every case:
//!
//! - exhaustiveness: every value is matched by some unguarded arm;
//! - each witness: matches at least one value, and every value it matches is
//!   matched by no unguarded arm;
//! - arm reachability and or-alternative redundancy, from the definition: an
//!   expanded row (one choice per or-pattern, in left-to-right preorder) is
//!   useful when some value matches it and no earlier unguarded expanded row.

use std::collections::BTreeSet;

use match_lang::{Arm, Checker, Ctor, Model, Pat, Redundant, Signature, WitnessRef};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Ty {
    Bool,
    Unit,
    Int(i128, i128),
    BigInt,
    Char,
    Opaque,
    Never,
    Tri,
    Pair(Box<Ty>, Box<Ty>),
    Opt(Box<Ty>),
    Res(Box<Ty>, Box<Ty>),
    Arr(u32, Box<Ty>),
    Seq(Box<Ty>),
}

struct Lang;

impl Model for Lang {
    type Ty = Ty;

    fn signature(&self, ty: &Ty) -> Signature<Ty> {
        match ty {
            Ty::Bool => Signature::Bool,
            Ty::Unit | Ty::Pair(..) => Signature::Product,
            Ty::Int(min, max) => Signature::Int {
                min: *min,
                max: *max,
            },
            Ty::BigInt => Signature::BigInt,
            Ty::Char => Signature::Char,
            Ty::Opaque => Signature::Opaque,
            Ty::Never => Signature::Sum { count: 0 },
            Ty::Tri => Signature::Sum { count: 3 },
            Ty::Opt(_) | Ty::Res(..) => Signature::Sum { count: 2 },
            Ty::Arr(len, elem) => Signature::Array {
                len: *len,
                elem: (**elem).clone(),
            },
            Ty::Seq(elem) => Signature::Slice {
                elem: (**elem).clone(),
            },
        }
    }

    fn fields(&self, ty: &Ty, variant: u32, out: &mut Vec<Ty>) {
        match (ty, variant) {
            (Ty::Pair(a, b), _) => out.extend([(**a).clone(), (**b).clone()]),
            (Ty::Opt(t), 1) | (Ty::Res(t, _), 0) | (Ty::Res(_, t), 1) => out.push((**t).clone()),
            _ => {}
        }
    }

    fn is_empty(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Never => true,
            Ty::Pair(a, b) => self.is_empty(a) || self.is_empty(b),
            Ty::Res(a, b) => self.is_empty(a) && self.is_empty(b),
            Ty::Arr(len, elem) => *len > 0 && self.is_empty(elem),
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

const FRESH: u64 = u64::MAX;

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Bool(bool),
    Int(i128),
    /// A big integer outside `i128`.
    Huge,
    Char(u32),
    Lit(u64),
    Ctor(u32, Vec<Val>),
    Seq(Vec<Val>),
}

/// What the patterns mention, so representatives can be chosen.
#[derive(Default)]
struct Ctx {
    ints: BTreeSet<i128>,
    chars: BTreeSet<u32>,
    lits: BTreeSet<u64>,
    max_fixed: usize,
    max_prefix: usize,
    max_suffix: usize,
}

const VALUE_CAP: usize = 3000;

fn values(ty: &Ty, ctx: &Ctx) -> Option<Vec<Val>> {
    let vals = match ty {
        Ty::Bool => vec![Val::Bool(false), Val::Bool(true)],
        Ty::Unit => vec![Val::Ctor(0, vec![])],
        Ty::Int(min, max) => {
            let mut points: BTreeSet<i128> = [*min, *max].into();
            points.extend(ctx.ints.iter().copied().filter(|p| min <= p && p <= max));
            points.into_iter().map(Val::Int).collect()
        }
        Ty::BigInt => {
            let mut points: BTreeSet<i128> = [0, i128::MIN, i128::MAX].into();
            points.extend(ctx.ints.iter().copied());
            let mut v: Vec<Val> = points.into_iter().map(Val::Int).collect();
            v.push(Val::Huge);
            v
        }
        Ty::Char => {
            let mut points: BTreeSet<u32> = [0, 0xD7FF, 0xE000, 0x10FFFF].into();
            points.extend(
                ctx.chars
                    .iter()
                    .copied()
                    .filter(|&c| char::from_u32(c).is_some()),
            );
            points.into_iter().map(Val::Char).collect()
        }
        Ty::Opaque => {
            let mut v: Vec<Val> = ctx.lits.iter().copied().map(Val::Lit).collect();
            v.push(Val::Lit(FRESH));
            v
        }
        Ty::Never => vec![],
        Ty::Tri => (0..3).map(|i| Val::Ctor(i, vec![])).collect(),
        Ty::Pair(a, b) => {
            let (a, b) = (values(a, ctx)?, values(b, ctx)?);
            if a.len() * b.len() > VALUE_CAP {
                return None;
            }
            let mut v = Vec::new();
            for x in &a {
                for y in &b {
                    v.push(Val::Ctor(0, vec![x.clone(), y.clone()]));
                }
            }
            v
        }
        Ty::Opt(t) => {
            let mut v = vec![Val::Ctor(0, vec![])];
            v.extend(values(t, ctx)?.into_iter().map(|x| Val::Ctor(1, vec![x])));
            v
        }
        Ty::Res(a, b) => {
            let mut v: Vec<Val> = values(a, ctx)?
                .into_iter()
                .map(|x| Val::Ctor(0, vec![x]))
                .collect();
            v.extend(values(b, ctx)?.into_iter().map(|x| Val::Ctor(1, vec![x])));
            v
        }
        Ty::Arr(len, elem) => sequences(&values(elem, ctx)?, *len as usize, *len as usize)?,
        Ty::Seq(elem) => {
            // Every length from `max(max_fixed + 1, max_prefix + max_suffix)`
            // up is matched alike; enumerate up to it.
            let longest = (ctx.max_fixed + 1).max(ctx.max_prefix + ctx.max_suffix);
            sequences(&values(elem, ctx)?, 0, longest)?
        }
    };
    (vals.len() <= VALUE_CAP).then_some(vals)
}

fn sequences(elems: &[Val], min: usize, max: usize) -> Option<Vec<Val>> {
    let mut out = Vec::new();
    let mut layer: Vec<Vec<Val>> = vec![vec![]];
    for len in 0..=max {
        if len >= min {
            out.extend(layer.iter().cloned().map(Val::Seq));
        }
        if out.len() > VALUE_CAP {
            return None;
        }
        if len < max {
            let mut next = Vec::new();
            for seq in &layer {
                for e in elems {
                    let mut s = seq.clone();
                    s.push(e.clone());
                    next.push(s);
                }
                if next.len() > VALUE_CAP {
                    return None;
                }
            }
            layer = next;
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Patterns, numbered in preorder
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum K {
    Wild,
    Or,
    Variant(u32),
    Product,
    Bool(bool),
    Int(i128, i128),
    Char(char, char),
    Lit(u64),
    /// Prefix length, and whether there is a rest.
    Slice(usize, bool),
}

#[derive(Clone, Debug)]
struct P {
    id: usize,
    kind: K,
    kids: Vec<P>,
}

/// A pattern before numbering.
fn p(kind: K, kids: Vec<P>) -> P {
    P { id: 0, kind, kids }
}

fn number(pat: &mut P, next: &mut usize) {
    pat.id = *next;
    *next += 1;
    for kid in &mut pat.kids {
        number(kid, next);
    }
}

fn to_pat(pat: &P) -> Pat {
    let kids = || pat.kids.iter().map(to_pat).collect::<Vec<_>>();
    match pat.kind {
        K::Wild => Pat::wild(),
        K::Or => Pat::or(kids()),
        K::Variant(v) => Pat::variant(v, kids()),
        K::Product => Pat::product(kids()),
        K::Bool(b) => Pat::bool(b),
        K::Int(lo, hi) => Pat::range(lo, hi),
        K::Char(lo, hi) => Pat::char_range(lo, hi),
        K::Lit(id) => Pat::lit(id),
        K::Slice(prefix, false) => {
            assert_eq!(prefix, pat.kids.len());
            Pat::slice(kids())
        }
        K::Slice(prefix, true) => {
            let all = kids();
            let (pre, post) = all.split_at(prefix);
            Pat::slice_rest(pre.to_vec(), post.to_vec())
        }
    }
}

fn matches(pat: &P, v: &Val) -> bool {
    match (&pat.kind, v) {
        (K::Wild, _) => true,
        (K::Or, _) => pat.kids.iter().any(|k| matches(k, v)),
        (K::Variant(i), Val::Ctor(j, vs)) if i == j => {
            pat.kids.iter().zip(vs).all(|(k, v)| matches(k, v))
        }
        (K::Product, Val::Ctor(0, vs)) => pat.kids.iter().zip(vs).all(|(k, v)| matches(k, v)),
        (K::Bool(b), Val::Bool(c)) => b == c,
        (K::Int(lo, hi), Val::Int(x)) => lo <= x && x <= hi,
        (K::Char(lo, hi), Val::Char(c)) => (*lo as u32) <= *c && *c <= (*hi as u32),
        (K::Lit(a), Val::Lit(b)) => a == b,
        (K::Slice(prefix, rest), Val::Seq(vs)) => {
            let (pre, post) = pat.kids.split_at(*prefix);
            let fits = if *rest {
                vs.len() >= pre.len() + post.len()
            } else {
                vs.len() == pre.len()
            };
            fits && pre.iter().zip(vs).all(|(k, v)| matches(k, v))
                && post
                    .iter()
                    .rev()
                    .zip(vs.iter().rev())
                    .all(|(k, v)| matches(k, v))
        }
        _ => false,
    }
}

/// Every or-free expansion of `pat`, in left-to-right preorder, with the ids
/// of the alternatives it takes.
fn expand(pat: &P) -> Vec<(P, Vec<usize>)> {
    if let K::Or = pat.kind {
        let mut out = Vec::new();
        for alt in &pat.kids {
            for (e, mut ids) in expand(alt) {
                ids.push(alt.id);
                out.push((e, ids));
            }
        }
        return out;
    }
    let mut rows: Vec<(Vec<P>, Vec<usize>)> = vec![(vec![], vec![])];
    for kid in &pat.kids {
        let options = expand(kid);
        let mut next = Vec::new();
        for (kids, ids) in &rows {
            for (e, eids) in &options {
                let mut k = kids.clone();
                k.push(e.clone());
                let mut i = ids.clone();
                i.extend(eids);
                next.push((k, i));
            }
        }
        rows = next;
    }
    rows.into_iter()
        .map(|(kids, ids)| {
            (
                P {
                    id: pat.id,
                    kind: pat.kind.clone(),
                    kids,
                },
                ids,
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Witnesses
// ---------------------------------------------------------------------------

fn witness_matches(w: WitnessRef<'_, Ty>, v: &Val) -> bool {
    let fields: Vec<WitnessRef<'_, Ty>> = w.fields().collect();
    let all = |vs: &[Val]| {
        fields.len() == vs.len() && fields.iter().zip(vs).all(|(f, v)| witness_matches(*f, v))
    };
    match (w.ctor(), v) {
        (Ctor::Wild, _) => true,
        (Ctor::Other, Val::Huge) => true,
        (Ctor::Other, Val::Lit(id)) => *id == FRESH,
        (Ctor::Variant(i), Val::Ctor(j, vs)) => i == *j && all(vs),
        (Ctor::Product, Val::Ctor(0, vs)) => all(vs),
        (Ctor::Bool(b), Val::Bool(c)) => b == *c,
        (Ctor::Int { lo, hi }, Val::Int(x)) => lo <= *x && *x <= hi,
        (Ctor::Char { lo, hi }, Val::Char(c)) => (lo as u32) <= *c && *c <= (hi as u32),
        (Ctor::Lit(a), Val::Lit(b)) => a == *b,
        (Ctor::Slice { len }, Val::Seq(vs)) => vs.len() == len as usize && all(vs),
        (Ctor::SliceRest { prefix, suffix }, Val::Seq(vs)) => {
            let (p, s) = (prefix as usize, suffix as usize);
            vs.len() >= p + s
                && fields.len() == p + s
                && fields[..p]
                    .iter()
                    .zip(vs)
                    .all(|(f, v)| witness_matches(*f, v))
                && fields[p..]
                    .iter()
                    .rev()
                    .zip(vs.iter().rev())
                    .all(|(f, v)| witness_matches(*f, v))
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Random generation
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }
}

fn gen_ty(rng: &mut Rng, depth: u32) -> Ty {
    let leaves = 9;
    let choice = if depth == 0 {
        rng.below(leaves)
    } else {
        rng.below(leaves + 6)
    };
    match choice {
        0 => Ty::Bool,
        1 => Ty::Unit,
        2 => {
            let lo = rng.below(5) as i128 - 2;
            Ty::Int(lo, lo + rng.below(5) as i128)
        }
        3 => [
            Ty::Int(0, 255),
            Ty::Int(i128::MIN, i128::MAX),
            Ty::Int(-128, 127),
        ][rng.below(3) as usize]
            .clone(),
        4 => Ty::BigInt,
        5 => Ty::Char,
        6 => Ty::Opaque,
        7 => Ty::Never,
        8 => Ty::Tri,
        9 | 10 => Ty::Pair(
            Box::new(gen_ty(rng, depth - 1)),
            Box::new(gen_ty(rng, depth - 1)),
        ),
        11 => Ty::Opt(Box::new(gen_ty(rng, depth - 1))),
        12 => Ty::Res(
            Box::new(gen_ty(rng, depth - 1)),
            Box::new(gen_ty(rng, depth - 1)),
        ),
        13 => Ty::Arr(rng.below(4) as u32, Box::new(gen_ty(rng, depth - 1))),
        _ => Ty::Seq(Box::new(gen_ty(rng, depth - 1))),
    }
}

const CHARS: [char; 8] = [
    '\0',
    'a',
    'b',
    'z',
    '\u{D7FF}',
    '\u{E000}',
    '\u{E001}',
    char::MAX,
];

fn gen_range(rng: &mut Rng, min: i128, max: i128) -> (i128, i128) {
    let mut candidates = vec![
        min,
        max,
        min.saturating_add(1).min(max),
        max.saturating_sub(1).max(min),
    ];
    for x in -3..=3i128 {
        if min <= x && x <= max {
            candidates.push(x);
        }
    }
    let a = rng.pick(&candidates);
    let b = rng.pick(&candidates);
    (a.min(b), a.max(b))
}

fn gen_pat(rng: &mut Rng, ty: &Ty, depth: u32) -> P {
    if rng.chance(18) || matches!(ty, Ty::Never) && !rng.chance(30) {
        return p(K::Wild, vec![]);
    }
    if depth > 0 && rng.chance(18) {
        let n = rng.below(3) as usize + 1;
        return p(K::Or, (0..n).map(|_| gen_pat(rng, ty, depth - 1)).collect());
    }
    let d = depth.saturating_sub(1);
    match ty {
        Ty::Bool => p(K::Bool(rng.chance(50)), vec![]),
        Ty::Unit => p(K::Product, vec![]),
        Ty::Int(min, max) => {
            let (lo, hi) = gen_range(rng, *min, *max);
            p(K::Int(lo, hi), vec![])
        }
        Ty::BigInt => {
            let (lo, hi) = gen_range(rng, i128::MIN, i128::MAX);
            p(K::Int(lo, hi), vec![])
        }
        Ty::Char => {
            let (a, b) = (rng.pick(&CHARS), rng.pick(&CHARS));
            p(K::Char(a.min(b), a.max(b)), vec![])
        }
        Ty::Opaque => p(K::Lit(rng.below(3)), vec![]),
        Ty::Never => p(K::Wild, vec![]),
        Ty::Tri => p(K::Variant(rng.below(3) as u32), vec![]),
        Ty::Pair(a, b) => p(K::Product, vec![gen_pat(rng, a, d), gen_pat(rng, b, d)]),
        Ty::Opt(t) => {
            if rng.chance(40) {
                p(K::Variant(0), vec![])
            } else {
                p(K::Variant(1), vec![gen_pat(rng, t, d)])
            }
        }
        Ty::Res(a, b) => {
            if rng.chance(50) {
                p(K::Variant(0), vec![gen_pat(rng, a, d)])
            } else {
                p(K::Variant(1), vec![gen_pat(rng, b, d)])
            }
        }
        Ty::Arr(len, elem) => {
            let len = *len as usize;
            if rng.chance(50) {
                p(
                    K::Slice(len, false),
                    (0..len).map(|_| gen_pat(rng, elem, d)).collect(),
                )
            } else {
                let pre = rng.below(len as u64 + 1) as usize;
                let post = rng.below((len - pre) as u64 + 1) as usize;
                p(
                    K::Slice(pre, true),
                    (0..pre + post).map(|_| gen_pat(rng, elem, d)).collect(),
                )
            }
        }
        Ty::Seq(elem) => {
            if rng.chance(50) {
                let len = rng.below(4) as usize;
                p(
                    K::Slice(len, false),
                    (0..len).map(|_| gen_pat(rng, elem, d)).collect(),
                )
            } else {
                let pre = rng.below(3) as usize;
                let post = rng.below(3) as usize;
                p(
                    K::Slice(pre, true),
                    (0..pre + post).map(|_| gen_pat(rng, elem, d)).collect(),
                )
            }
        }
    }
}

fn collect_ctx(pat: &P, ctx: &mut Ctx) {
    match pat.kind {
        K::Int(lo, hi) => {
            for x in [Some(lo), Some(hi), lo.checked_sub(1), hi.checked_add(1)]
                .into_iter()
                .flatten()
            {
                ctx.ints.insert(x);
            }
        }
        K::Char(lo, hi) => {
            for x in [
                lo as u32,
                hi as u32,
                (lo as u32).wrapping_sub(1),
                hi as u32 + 1,
            ] {
                ctx.chars.insert(x);
            }
        }
        K::Lit(id) => {
            ctx.lits.insert(id);
        }
        K::Slice(prefix, rest) => {
            if rest {
                ctx.max_prefix = ctx.max_prefix.max(prefix);
                ctx.max_suffix = ctx.max_suffix.max(pat.kids.len() - prefix);
            } else {
                ctx.max_fixed = ctx.max_fixed.max(pat.kids.len());
            }
        }
        _ => {}
    }
    for kid in &pat.kids {
        collect_ctx(kid, ctx);
    }
}

// ---------------------------------------------------------------------------
// The reference
// ---------------------------------------------------------------------------

struct Expected {
    exhaustive: bool,
    unreachable: Vec<usize>,
    redundant: Vec<Redundant>,
}

fn reference(arms: &[(P, bool)], vals: &[Val]) -> Expected {
    let expansions: Vec<Vec<(P, Vec<usize>)>> = arms.iter().map(|(pat, _)| expand(pat)).collect();
    let mut useful: Vec<Vec<bool>> = expansions.iter().map(|e| vec![false; e.len()]).collect();
    let mut exhaustive = true;
    for v in vals {
        let mut caught = false;
        'arms: for (i, (_, guarded)) in arms.iter().enumerate() {
            for (k, (row, _)) in expansions[i].iter().enumerate() {
                if matches(row, v) {
                    useful[i][k] = true;
                    if !guarded {
                        caught = true;
                        break 'arms;
                    }
                }
            }
        }
        exhaustive &= caught;
    }
    let mut unreachable = Vec::new();
    let mut redundant = Vec::new();
    for (i, (pat, _)) in arms.iter().enumerate() {
        if !useful[i].iter().any(|&u| u) {
            unreachable.push(i);
            continue;
        }
        let useful_alts: BTreeSet<usize> = expansions[i]
            .iter()
            .zip(&useful[i])
            .filter(|(_, u)| **u)
            .flat_map(|((_, ids), _)| ids.iter().copied())
            .collect();
        let mut stack = vec![pat];
        while let Some(node) = stack.pop() {
            for kid in node.kids.iter().rev() {
                if matches!(node.kind, K::Or) && !useful_alts.contains(&kid.id) {
                    redundant.push((kid.id, i));
                } else {
                    stack.push(kid);
                }
            }
        }
    }
    redundant.sort();
    let redundant = redundant
        .into_iter()
        .map(|(node, arm)| Redundant { arm, node })
        .collect::<Vec<_>>();
    Expected {
        exhaustive,
        unreachable,
        redundant,
    }
}

/// A random case: a type, arms (pattern, guarded), and the type's values.
type Case = (Ty, Vec<(P, bool)>, Vec<Val>);

/// One random case, or `None` when its value space is too large to enumerate.
fn case(seed: u64) -> Option<Case> {
    let mut rng = Rng(seed);
    let ty = gen_ty(&mut rng, 2);
    let count = rng.below(7) as usize;
    let mut arms = Vec::new();
    let mut ctx = Ctx::default();
    for _ in 0..count {
        let mut pat = gen_pat(&mut rng, &ty, 3);
        number(&mut pat, &mut 0);
        collect_ctx(&pat, &mut ctx);
        arms.push((pat, rng.chance(15)));
    }
    let vals = values(&ty, &ctx)?;
    Some((ty, arms, vals))
}

fn run_case(seed: u64, checker: &mut Checker<Ty>) -> Result<(), TestCaseError> {
    let Some((ty, arms, vals)) = case(seed) else {
        return Ok(());
    };
    let input: Vec<Arm> = arms
        .iter()
        .map(|(pat, guarded)| {
            if *guarded {
                Arm::guarded(to_pat(pat))
            } else {
                Arm::new(to_pat(pat))
            }
        })
        .collect();
    let report = checker
        .check(&Lang, &ty, &input)
        .map_err(|e| TestCaseError::fail(format!("{e}")))?;
    let expected = reference(&arms, &vals);

    prop_assert_eq!(
        report.is_exhaustive(),
        expected.exhaustive,
        "ty {:?} arms {:?}",
        ty,
        input
    );
    prop_assert_eq!(
        report.unreachable(),
        &expected.unreachable[..],
        "ty {:?} arms {:?}",
        ty,
        input
    );
    // Sort both by (node, arm) order is not what the crate promises; compare as sets
    // ordered by arm then preorder.
    let mut got = report.redundant().to_vec();
    got.sort_by_key(|r| (r.node, r.arm));
    let mut want = expected.redundant.clone();
    want.sort_by_key(|r| (r.node, r.arm));
    prop_assert_eq!(got, want, "ty {:?} arms {:?}", ty, input);
    let mut ordered = report.redundant().to_vec();
    ordered.sort();
    prop_assert_eq!(
        ordered,
        report.redundant().to_vec(),
        "redundant must be in arm, preorder order"
    );

    prop_assert!(report.missing().len() <= checker.witness_limit());
    for w in report.missing() {
        prop_assert_eq!(w.ty(), &ty);
        let covered: Vec<&Val> = vals
            .iter()
            .filter(|v| witness_matches(w.root(), v))
            .collect();
        prop_assert!(
            !covered.is_empty(),
            "witness {} matches no value of {:?}",
            w,
            ty
        );
        for v in covered {
            let caught = arms
                .iter()
                .any(|(pat, guarded)| !guarded && matches(pat, v));
            prop_assert!(
                !caught,
                "witness {} covers {:?}, which an arm matches; arms {:?}",
                w,
                v,
                input
            );
        }
    }
    Ok(())
}

/// The case count: 4,000 by default, or `PROPTEST_CASES` for deep runs.
fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4000)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), .. ProptestConfig::default() })]

    #[test]
    fn prop_agrees_with_brute_force(seed in any::<u64>()) {
        let mut checker = Checker::new();
        run_case(seed, &mut checker)?;
    }

    #[test]
    fn prop_reused_checker_agrees_with_fresh(seeds in proptest::collection::vec(any::<u64>(), 1..8)) {
        let mut reused = Checker::new().with_witness_limit(3);
        for seed in seeds {
            run_case(seed, &mut reused)?;
            let Some((ty, arms, _)) = case(seed) else { continue };
            let input: Vec<Arm> = arms.iter().map(|(pat, g)| if *g { Arm::guarded(to_pat(pat)) } else { Arm::new(to_pat(pat)) }).collect();
            let a = reused.check(&Lang, &ty, &input).map_err(|e| TestCaseError::fail(format!("{e}")))?;
            let b = Checker::new().with_witness_limit(3).check(&Lang, &ty, &input).map_err(|e| TestCaseError::fail(format!("{e}")))?;
            prop_assert_eq!(a.missing(), b.missing());
            prop_assert_eq!(a.unreachable(), b.unreachable());
            prop_assert_eq!(a.redundant(), b.redundant());
            prop_assert_eq!(a.steps(), b.steps());
        }
    }

    #[test]
    fn prop_witness_limit_is_respected_and_never_hides_non_exhaustiveness(seed in any::<u64>(), limit in 0usize..4) {
        let Some((ty, arms, _)) = case(seed) else { return Ok(()) };
        let input: Vec<Arm> = arms.iter().map(|(pat, g)| if *g { Arm::guarded(to_pat(pat)) } else { Arm::new(to_pat(pat)) }).collect();
        let full = Checker::new().check(&Lang, &ty, &input).map_err(|e| TestCaseError::fail(format!("{e}")))?;
        let capped = Checker::new().with_witness_limit(limit).check(&Lang, &ty, &input).map_err(|e| TestCaseError::fail(format!("{e}")))?;
        prop_assert_eq!(full.is_exhaustive(), capped.is_exhaustive());
        prop_assert!(capped.missing().len() <= limit.max(1));
        prop_assert_eq!(full.unreachable(), capped.unreachable());
    }

    #[test]
    fn prop_display_never_panics(seed in any::<u64>()) {
        let Some((ty, arms, _)) = case(seed) else { return Ok(()) };
        let input: Vec<Arm> = arms.iter().map(|(pat, _)| Arm::new(to_pat(pat))).collect();
        for arm in &input {
            prop_assert!(!arm.pat().to_string().is_empty());
        }
        let report = Checker::new().check(&Lang, &ty, &input).map_err(|e| TestCaseError::fail(format!("{e}")))?;
        for w in report.missing() {
            prop_assert!(!w.to_string().is_empty());
            prop_assert!(!w.display(&Lang).to_string().is_empty());
        }
    }
}
