//! Criterion benchmarks: checking large matches.
//!
//! ```text
//! cargo bench --bench bench
//! ```

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use match_lang::{Arm, Checker, Model, Pat, Signature, check};

#[derive(Clone, Copy, Debug)]
enum Ty {
    Bool,
    I64,
    /// A sum of this many field-less variants.
    Enum(u32),
    /// `Option` nested this deep around a `bool`.
    Opt(u32),
    /// A tuple of this many bools.
    Wide(u32),
    /// A slice of bools.
    Seq,
}

struct Lang;

impl Model for Lang {
    type Ty = Ty;

    fn signature(&self, ty: &Ty) -> Signature<Ty> {
        match *ty {
            Ty::Bool | Ty::Opt(0) => Signature::Bool,
            Ty::I64 => Signature::Int {
                min: i64::MIN.into(),
                max: i64::MAX.into(),
            },
            Ty::Enum(n) => Signature::Sum { count: n },
            Ty::Opt(_) => Signature::Sum { count: 2 },
            Ty::Wide(_) => Signature::Product,
            Ty::Seq => Signature::Slice { elem: Ty::Bool },
        }
    }

    fn fields(&self, ty: &Ty, variant: u32, out: &mut Vec<Ty>) {
        match *ty {
            Ty::Opt(d) if variant == 1 => out.push(Ty::Opt(d - 1)),
            Ty::Wide(n) => out.extend(std::iter::repeat_n(Ty::Bool, n as usize)),
            _ => {}
        }
    }
}

fn bench_arms(c: &mut Criterion, name: &str, ty: Ty, arms: &[Arm]) {
    let mut group = c.benchmark_group(name);
    group.throughput(Throughput::Elements(arms.len() as u64));
    group.sample_size(20);
    let mut checker = Checker::new().with_step_limit(u64::MAX);
    group.bench_function("check", |b| {
        b.iter(|| {
            black_box(
                checker
                    .check(&Lang, &ty, black_box(arms))
                    .map(|r| r.steps()),
            )
        })
    });
    group.finish();
}

fn large(c: &mut Criterion) {
    // 100k arms, one per variant of a 100k-variant enum.
    let arms: Vec<Arm> = (0..100_000)
        .map(|v| Arm::new(Pat::variant(v, [])))
        .collect();
    bench_arms(c, "enum_100k_variants", Ty::Enum(100_000), &arms);

    // 100k distinct integer literals and a wildcard.
    let mut arms: Vec<Arm> = (0..100_000).map(|i| Arm::new(Pat::int(i * 7))).collect();
    arms.push(Arm::new(Pat::wild()));
    bench_arms(c, "int_literals_100k", Ty::I64, &arms);

    // 100k adjacent ranges covering every i64.
    let step = (i128::from(i64::MAX) - i128::from(i64::MIN)) / 100_000;
    let arms: Vec<Arm> = (0..100_000)
        .map(|i| {
            let lo = i128::from(i64::MIN) + i * step;
            let hi = if i == 99_999 {
                i128::from(i64::MAX)
            } else {
                lo + step - 1
            };
            Arm::new(Pat::range(lo, hi))
        })
        .collect();
    bench_arms(c, "int_ranges_100k_exhaustive", Ty::I64, &arms);

    // Every row of a 16-column truth table: 65,536 arms, exhaustive.
    let arms: Vec<Arm> = (0..1u32 << 16)
        .map(|bits| {
            Arm::new(Pat::product(
                (0..16).map(|i| Pat::bool((bits >> i) & 1 == 1)),
            ))
        })
        .collect();
    bench_arms(c, "truth_table_16", Ty::Wide(16), &arms);

    // 1,000 columns by 1,000 rows, each row testing one column.
    let arms: Vec<Arm> = (0..1000)
        .map(|r| {
            Arm::new(Pat::product(
                (0..1000).map(|c| if c == r { Pat::bool(true) } else { Pat::wild() }),
            ))
        })
        .chain([Arm::new(Pat::wild())])
        .collect();
    bench_arms(c, "wide_1000x1000", Ty::Wide(1000), &arms);

    // None, Some(None), Some(Some(None)), ... 1,000 levels deep.
    let nested = |depth: u32, leaf: Pat| (0..depth).fold(leaf, |p, _| Pat::variant(1, [p]));
    let mut arms: Vec<Arm> = (0..1000)
        .map(|d| Arm::new(nested(d, Pat::variant(0, []))))
        .collect();
    arms.push(Arm::new(nested(1000, Pat::wild())));
    bench_arms(c, "nested_options_1000", Ty::Opt(1000), &arms);

    // 1,000 arms of 100 alternatives each: 100k or-pattern alternatives.
    let mut arms: Vec<Arm> = (0..1000)
        .map(|a| Arm::new(Pat::or((0..100).map(|i| Pat::int(a * 100 + i)))))
        .collect();
    arms.push(Arm::new(Pat::wild()));
    bench_arms(c, "or_alternatives_100k", Ty::I64, &arms);

    // Fixed-length slice patterns of every length up to 1,000, then `[..]`.
    let mut arms: Vec<Arm> = (0..1000)
        .map(|n| Arm::new(Pat::slice((0..n).map(|_| Pat::bool(true)))))
        .collect();
    arms.push(Arm::new(Pat::slice_rest([], [])));
    bench_arms(c, "slices_1000_lengths", Ty::Seq, &arms);
}

fn many_small(c: &mut Criterion) {
    // A program's worth of small matches, through one reused checker.
    let ty = Ty::Opt(1);
    let arms = [
        Arm::new(Pat::variant(1, [Pat::bool(true)])),
        Arm::new(Pat::variant(1, [Pat::bool(false)])),
        Arm::new(Pat::variant(0, [])),
    ];
    let mut group = c.benchmark_group("small_matches");
    group.throughput(Throughput::Elements(10_000));
    group.bench_function("reused_checker_10k", |b| {
        let mut checker = Checker::new();
        b.iter(|| {
            for _ in 0..10_000 {
                let report = checker.check(&Lang, &ty, black_box(&arms));
                black_box(report.map(|r| r.is_exhaustive()).unwrap_or(false));
            }
        })
    });
    group.bench_function("fresh_check_10k", |b| {
        b.iter(|| {
            for _ in 0..10_000 {
                let report = check(&Lang, &ty, black_box(&arms));
                black_box(report.map(|r| r.is_exhaustive()).unwrap_or(false));
            }
        })
    });
    group.finish();
}

criterion_group!(benches, large, many_small);
criterion_main!(benches);
