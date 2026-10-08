//! Integration tests: the documented behaviour of each kind of type and
//! pattern, guards, or-patterns, witnesses, limits, and every error.

use std::cell::Cell;

use match_lang::{
    Arm, Checker, Ctor, DEFAULT_WITNESS_LIMIT, Error, Model, Pat, Reason, Redundant, Signature,
    check,
};

#[derive(Clone, Debug, PartialEq)]
enum Ty {
    Bool,
    U8,
    I128,
    BigInt,
    Char,
    Str,
    Never,
    Tuple(Vec<Ty>),
    Option(Box<Ty>),
    Result(Box<Ty>, Box<Ty>),
    Color,
    Array(u32, Box<Ty>),
    Slice(Box<Ty>),
    /// A sum with this many field-less variants.
    Wide(u32),
}

fn opt(t: Ty) -> Ty {
    Ty::Option(Box::new(t))
}

struct Lang;

impl Model for Lang {
    type Ty = Ty;

    fn signature(&self, ty: &Ty) -> Signature<Ty> {
        match ty {
            Ty::Bool => Signature::Bool,
            Ty::U8 => Signature::Int { min: 0, max: 255 },
            Ty::I128 => Signature::Int {
                min: i128::MIN,
                max: i128::MAX,
            },
            Ty::BigInt => Signature::BigInt,
            Ty::Char => Signature::Char,
            Ty::Str => Signature::Opaque,
            Ty::Never => Signature::Sum { count: 0 },
            Ty::Tuple(_) => Signature::Product,
            Ty::Option(_) | Ty::Result(..) => Signature::Sum { count: 2 },
            Ty::Color => Signature::Sum { count: 3 },
            Ty::Array(len, elem) => Signature::Array {
                len: *len,
                elem: (**elem).clone(),
            },
            Ty::Slice(elem) => Signature::Slice {
                elem: (**elem).clone(),
            },
            Ty::Wide(n) => Signature::Sum { count: *n },
        }
    }

    fn fields(&self, ty: &Ty, variant: u32, out: &mut Vec<Ty>) {
        match (ty, variant) {
            (Ty::Tuple(fields), _) => out.extend(fields.iter().cloned()),
            (Ty::Option(t), 1) | (Ty::Result(t, _), 0) | (Ty::Result(_, t), 1) => {
                out.push((**t).clone());
            }
            _ => {}
        }
    }

    fn is_empty(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Never | Ty::Wide(0) => true,
            Ty::Tuple(fields) => fields.iter().any(|f| self.is_empty(f)),
            _ => false,
        }
    }

    fn variant_name(&self, ty: &Ty, variant: u32) -> Option<&str> {
        match (ty, variant) {
            (Ty::Option(_), 0) => Some("None"),
            (Ty::Option(_), 1) => Some("Some"),
            (Ty::Result(..), 0) => Some("Ok"),
            (Ty::Result(..), 1) => Some("Err"),
            (Ty::Color, 0) => Some("Red"),
            (Ty::Color, 1) => Some("Green"),
            (Ty::Color, 2) => Some("Blue"),
            _ => None,
        }
    }
}

fn arms(pats: impl IntoIterator<Item = Pat>) -> Vec<Arm> {
    pats.into_iter().map(Arm::new).collect()
}

fn missing(ty: &Ty, arms: &[Arm]) -> Vec<String> {
    check(&Lang, ty, arms)
        .unwrap()
        .missing()
        .iter()
        .map(|w| w.display(&Lang).to_string())
        .collect()
}

fn none() -> Pat {
    Pat::variant(0, [])
}

fn some(p: Pat) -> Pat {
    Pat::variant(1, [p])
}

// ---------------------------------------------------------------------------
// Sums, products, booleans
// ---------------------------------------------------------------------------

#[test]
fn test_no_arms_lists_every_variant_at_the_top() {
    assert_eq!(missing(&Ty::Color, &[]), ["Red", "Green", "Blue"]);
    assert_eq!(missing(&opt(Ty::Bool), &[]), ["None", "Some(_)"]);
    assert_eq!(missing(&Ty::Bool, &[]), ["false", "true"]);
    assert_eq!(missing(&Ty::Tuple(vec![]), &[]), ["()"]);
}

#[test]
fn test_missing_variant_is_named() {
    let a = arms([Pat::variant(0, []), Pat::variant(2, [])]);
    assert_eq!(missing(&Ty::Color, &a), ["Green"]);
}

#[test]
fn test_nested_witness_is_concrete() {
    let ty = opt(opt(Ty::Bool));
    let a = arms([none(), some(none()), some(some(Pat::bool(false)))]);
    assert_eq!(missing(&ty, &a), ["Some(Some(true))"]);
}

#[test]
fn test_unmentioned_column_is_a_wildcard_below_the_top() {
    let ty = Ty::Tuple(vec![Ty::Bool, Ty::Color]);
    let a = arms([Pat::product([Pat::bool(true), Pat::wild()])]);
    assert_eq!(missing(&ty, &a), ["(false, _)"]);
}

#[test]
fn test_one_field_tuple_prints_with_comma() {
    let ty = Ty::Tuple(vec![Ty::Bool]);
    let a = arms([Pat::product([Pat::bool(true)])]);
    assert_eq!(missing(&ty, &a), ["(false,)"]);
}

#[test]
fn test_wildcard_after_full_coverage_is_unreachable() {
    let a = arms([Pat::bool(true), Pat::bool(false), Pat::wild()]);
    let report = check(&Lang, &Ty::Bool, &a).unwrap();
    assert!(report.is_exhaustive());
    assert_eq!(report.unreachable(), &[2]);
}

#[test]
fn test_unreachable_arm_in_the_middle() {
    let ty = opt(Ty::Bool);
    let a = arms([some(Pat::wild()), some(Pat::bool(true)), none()]);
    let report = check(&Lang, &ty, &a).unwrap();
    assert!(report.is_exhaustive());
    assert_eq!(report.unreachable(), &[1]);
}

#[test]
fn test_witness_accessors() {
    let ty = Ty::Tuple(vec![Ty::Bool, opt(Ty::Bool)]);
    let a = arms([
        Pat::product([Pat::wild(), none()]),
        Pat::product([Pat::bool(true), some(Pat::wild())]),
    ]);
    let report = check(&Lang, &ty, &a).unwrap();
    let w = &report.missing()[0];
    assert_eq!(w.display(&Lang).to_string(), "(false, Some(_))");
    assert_eq!(w.ctor(), Ctor::Product);
    assert_eq!(w.ty(), &ty);
    let fields: Vec<_> = w.fields().collect();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].ctor(), Ctor::Bool(false));
    assert_eq!(fields[0].ty(), &Ty::Bool);
    assert_eq!(fields[1].ctor(), Ctor::Variant(1));
    assert_eq!(fields[1].arity(), 1);
    assert_eq!(fields[1].fields().next().unwrap().ctor(), Ctor::Wild);
    assert_eq!(fields[1].to_string(), "#1(_)");
    assert_eq!(format!("{w:?}"), "Witness((false, #1(_)))");
}

// ---------------------------------------------------------------------------
// Empty types
// ---------------------------------------------------------------------------

#[test]
fn test_empty_type_needs_no_arms() {
    let report = check(&Lang, &Ty::Never, &[]).unwrap();
    assert!(report.is_exhaustive());
    // No value reaches a wildcard over an empty type.
    let report = check(&Lang, &Ty::Never, &arms([Pat::wild()])).unwrap();
    assert_eq!(report.unreachable(), &[0]);
}

#[test]
fn test_uninhabited_variant_is_neither_required_nor_reachable() {
    let ty = Ty::Result(Box::new(Ty::Bool), Box::new(Ty::Never));
    let a = arms([Pat::variant(0, [Pat::wild()])]);
    assert!(check(&Lang, &ty, &a).unwrap().is_exhaustive());
    let a = arms([
        Pat::variant(0, [Pat::wild()]),
        Pat::variant(1, [Pat::wild()]),
    ]);
    assert_eq!(check(&Lang, &ty, &a).unwrap().unreachable(), &[1]);
    assert_eq!(missing(&ty, &[]), ["Ok(_)"]);
}

#[test]
fn test_product_with_empty_field_is_empty() {
    let ty = Ty::Tuple(vec![Ty::Bool, Ty::Never]);
    assert!(check(&Lang, &ty, &[]).unwrap().is_exhaustive());
}

// ---------------------------------------------------------------------------
// Integers, characters, opaque literals
// ---------------------------------------------------------------------------

#[test]
fn test_u8_ranges_cover_exactly() {
    let a = arms([Pat::range(0, 127), Pat::range(128, 255)]);
    assert!(check(&Lang, &Ty::U8, &a).unwrap().is_exhaustive());
    let a = arms([Pat::range(0, 9), Pat::int(11), Pat::range(13, 254)]);
    assert_eq!(missing(&Ty::U8, &a), ["10", "12", "255"]);
}

#[test]
fn test_overlapping_ranges_and_unreachable_literal() {
    let a = arms([
        Pat::range(0, 100),
        Pat::range(50, 200),
        Pat::int(150),
        Pat::range(201, 255),
    ]);
    let report = check(&Lang, &Ty::U8, &a).unwrap();
    assert!(report.is_exhaustive());
    assert_eq!(report.unreachable(), &[2]);
}

#[test]
fn test_i128_extremes() {
    let a = arms([Pat::range(i128::MIN, -1), Pat::range(0, i128::MAX - 1)]);
    assert_eq!(missing(&Ty::I128, &a), [i128::MAX.to_string()]);
    let a = arms([Pat::range(i128::MIN + 1, i128::MAX)]);
    assert_eq!(missing(&Ty::I128, &a), [i128::MIN.to_string()]);
    let a = arms([Pat::range(i128::MIN, i128::MAX)]);
    assert!(check(&Lang, &Ty::I128, &a).unwrap().is_exhaustive());
    assert_eq!(
        missing(&Ty::I128, &[]),
        [format!("{}..={}", i128::MIN, i128::MAX)]
    );
}

#[test]
fn test_bigint_is_never_covered_by_ranges() {
    let a = arms([Pat::range(i128::MIN, i128::MAX)]);
    let report = check(&Lang, &Ty::BigInt, &a).unwrap();
    assert!(!report.is_exhaustive());
    assert_eq!(report.missing()[0].ctor(), Ctor::Other);
    assert_eq!(report.missing()[0].to_string(), "_");
}

#[test]
fn test_chars_skip_the_surrogates() {
    let a = arms([
        Pat::char_range('\0', '\u{D7FF}'),
        Pat::char_range('\u{E000}', char::MAX),
    ]);
    assert!(check(&Lang, &Ty::Char, &a).unwrap().is_exhaustive());
    // One range across the gap covers both sides.
    let a = arms([Pat::char_range('\0', char::MAX)]);
    assert!(check(&Lang, &Ty::Char, &a).unwrap().is_exhaustive());
    let a = arms([Pat::char_range('\0', '\u{D7FF}')]);
    let report = check(&Lang, &Ty::Char, &a).unwrap();
    assert_eq!(
        report.missing()[0].ctor(),
        Ctor::Char {
            lo: '\u{E000}',
            hi: char::MAX
        }
    );
    let a = arms([Pat::char_range('b', 'y')]);
    assert_eq!(
        missing(&Ty::Char, &a),
        ["'\\0'..='a'", "'z'..='\\u{10ffff}'"]
    );
}

#[test]
fn test_opaque_literals_need_a_wildcard() {
    let a = arms([Pat::lit(1), Pat::lit(2), Pat::lit(1)]);
    let report = check(&Lang, &Ty::Str, &a).unwrap();
    assert_eq!(report.missing()[0].ctor(), Ctor::Other);
    assert_eq!(report.unreachable(), &[2]);
    let a = arms([Pat::lit(1), Pat::wild()]);
    assert!(check(&Lang, &Ty::Str, &a).unwrap().is_exhaustive());
}

// ---------------------------------------------------------------------------
// Arrays and slices
// ---------------------------------------------------------------------------

#[test]
fn test_array_patterns() {
    let ty = Ty::Array(2, Box::new(Ty::Bool));
    let a = arms([
        Pat::slice([Pat::bool(true), Pat::wild()]),
        Pat::slice_rest([], [Pat::bool(false)]),
    ]);
    assert_eq!(missing(&ty, &a), ["[false, true]"]);
    let a = arms([
        Pat::slice_rest([Pat::bool(true)], []),
        Pat::slice_rest([Pat::bool(false)], []),
    ]);
    assert!(check(&Lang, &ty, &a).unwrap().is_exhaustive());
}

#[test]
fn test_slice_lengths() {
    let ty = Ty::Slice(Box::new(Ty::Bool));
    let a = arms([Pat::slice([]), Pat::slice_rest([Pat::wild()], [])]);
    assert!(check(&Lang, &ty, &a).unwrap().is_exhaustive());
    let a = arms([Pat::slice([]), Pat::slice([Pat::wild()])]);
    assert_eq!(missing(&ty, &a), ["[_, _, ..]"]);
    // Once a missing constructor proves the match non-exhaustive, the
    // present ones are not explored for more witnesses.
    let a = arms([
        Pat::slice_rest([Pat::bool(true)], []),
        Pat::slice_rest([], [Pat::bool(true)]),
    ]);
    assert_eq!(missing(&ty, &a), ["[]"]);
    let a = arms([
        Pat::slice([]),
        Pat::slice_rest([Pat::bool(true)], []),
        Pat::slice_rest([], [Pat::bool(true)]),
    ]);
    assert_eq!(missing(&ty, &a), ["[false]", "[false, .., false]"]);
}

#[test]
fn test_slice_rest_alignment_with_short_sequences() {
    // `[true, ..]` and `[.., false]` overlap on `[true, false]` but the
    // one-element sequences differ: `[false]` is matched by the second.
    let ty = Ty::Slice(Box::new(Ty::Bool));
    let a = arms([
        Pat::slice([]),
        Pat::slice_rest([Pat::bool(true)], []),
        Pat::slice_rest([], [Pat::bool(false)]),
        Pat::slice_rest([Pat::bool(false)], [Pat::bool(true)]),
    ]);
    assert!(check(&Lang, &ty, &a).unwrap().is_exhaustive());
}

#[test]
fn test_slice_of_empty_elements_has_only_the_empty_value() {
    let ty = Ty::Slice(Box::new(Ty::Never));
    let a = arms([Pat::slice([])]);
    assert!(check(&Lang, &ty, &a).unwrap().is_exhaustive());
    let a = arms([Pat::slice([]), Pat::slice_rest([Pat::wild()], [])]);
    assert_eq!(check(&Lang, &ty, &a).unwrap().unreachable(), &[1]);
}

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

#[test]
fn test_guarded_arm_does_not_cover() {
    let a = [Arm::guarded(Pat::wild())];
    assert_eq!(missing(&Ty::Bool, &a), ["false", "true"]);
}

#[test]
fn test_guarded_arm_does_not_shadow_later_arms() {
    let a = [
        Arm::guarded(Pat::bool(true)),
        Arm::new(Pat::bool(true)),
        Arm::new(Pat::bool(false)),
    ];
    let report = check(&Lang, &Ty::Bool, &a).unwrap();
    assert!(report.is_exhaustive());
    assert!(report.unreachable().is_empty());
}

#[test]
fn test_guarded_arm_after_cover_is_unreachable() {
    let a = [Arm::new(Pat::wild()), Arm::guarded(Pat::bool(true))];
    assert_eq!(check(&Lang, &Ty::Bool, &a).unwrap().unreachable(), &[1]);
}

#[test]
fn test_guarded_or_alternatives_do_not_shadow_each_other() {
    let a = [
        Arm::guarded(Pat::or([Pat::bool(true), Pat::bool(true)])),
        Arm::new(Pat::wild()),
    ];
    let report = check(&Lang, &Ty::Bool, &a).unwrap();
    assert!(report.redundant().is_empty());
}

// ---------------------------------------------------------------------------
// Or-patterns
// ---------------------------------------------------------------------------

#[test]
fn test_or_pattern_covers() {
    let a = arms([Pat::or([
        Pat::variant(0, []),
        Pat::variant(1, []),
        Pat::variant(2, []),
    ])]);
    let report = check(&Lang, &Ty::Color, &a).unwrap();
    assert!(report.is_exhaustive());
    assert!(report.redundant().is_empty());
}

#[test]
fn test_redundant_alternative_inside_a_field() {
    // Some(true | false | true): nodes 0 Some, 1 or, 2 true, 3 false, 4 true.
    let a = arms([
        some(Pat::or([
            Pat::bool(true),
            Pat::bool(false),
            Pat::bool(true),
        ])),
        none(),
    ]);
    let report = check(&Lang, &opt(Ty::Bool), &a).unwrap();
    assert!(report.is_exhaustive());
    assert_eq!(report.redundant(), &[Redundant { arm: 0, node: 4 }]);
}

#[test]
fn test_redundant_alternative_shadowed_by_earlier_arm() {
    let a = arms([none(), Pat::or([none(), some(Pat::wild())])]);
    let report = check(&Lang, &opt(Ty::Bool), &a).unwrap();
    assert_eq!(report.redundant(), &[Redundant { arm: 1, node: 1 }]);
}

#[test]
fn test_only_the_outermost_redundant_alternative_is_reported() {
    // Arm 1: true | (true | true): the whole inner or is redundant.
    let a = arms([
        Pat::or([
            Pat::bool(false),
            Pat::or([Pat::bool(false), Pat::bool(false)]),
        ]),
        Pat::bool(true),
    ]);
    let report = check(&Lang, &Ty::Bool, &a).unwrap();
    assert_eq!(report.redundant(), &[Redundant { arm: 0, node: 2 }]);
}

#[test]
fn test_unreachable_arm_reports_no_alternatives() {
    let a = arms([Pat::wild(), Pat::or([Pat::bool(true), Pat::bool(true)])]);
    let report = check(&Lang, &Ty::Bool, &a).unwrap();
    assert_eq!(report.unreachable(), &[1]);
    assert!(report.redundant().is_empty());
}

#[test]
fn test_empty_or_matches_nothing() {
    let a = arms([Pat::or([]), Pat::wild()]);
    let report = check(&Lang, &Ty::Bool, &a).unwrap();
    assert_eq!(report.unreachable(), &[0]);
}

#[test]
fn test_or_of_tuples_in_two_columns() {
    let ty = Ty::Tuple(vec![Ty::Bool, Ty::Bool]);
    let a = arms([
        Pat::product([
            Pat::or([Pat::bool(true), Pat::bool(false)]),
            Pat::bool(true),
        ]),
        Pat::product([
            Pat::bool(false),
            Pat::or([Pat::bool(false), Pat::bool(true)]),
        ]),
    ]);
    let report = check(&Lang, &ty, &a).unwrap();
    assert_eq!(report.missing()[0].to_string(), "(true, false)");
    // In arm 1, `true` (node 4) only adds `(false, true)`, already matched.
    assert_eq!(report.redundant(), &[Redundant { arm: 1, node: 4 }]);
}

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

#[test]
fn test_witness_limit_caps_the_list() {
    let report = check(&Lang, &Ty::Wide(1000), &[]).unwrap();
    assert_eq!(report.missing().len(), DEFAULT_WITNESS_LIMIT);
    let report = Checker::new()
        .with_witness_limit(3)
        .check(&Lang, &Ty::Wide(1000), &[])
        .unwrap();
    assert_eq!(report.missing().len(), 3);
}

#[test]
fn test_huge_sum_with_few_arms_is_cheap() {
    let a = arms([Pat::variant(0, []), Pat::variant(u32::MAX - 1, [])]);
    let report = check(&Lang, &Ty::Wide(u32::MAX), &a).unwrap();
    assert_eq!(report.missing()[0].ctor(), Ctor::Variant(1));
    assert!(report.steps() < 1_000);
}

#[test]
fn test_step_limit_is_an_error() {
    let a: Vec<Arm> = (0..50).map(|i| Arm::new(Pat::int(i))).collect();
    let err = Checker::new()
        .with_step_limit(20)
        .check(&Lang, &Ty::U8, &a)
        .unwrap_err();
    assert_eq!(err, Error::StepLimit { limit: 20 });
    // The same checker works again with a sane match afterwards.
    let mut checker = Checker::new().with_step_limit(20);
    assert!(checker.check(&Lang, &Ty::U8, &a).is_err());
    assert!(
        checker
            .check(&Lang, &Ty::Bool, &arms([Pat::wild()]))
            .unwrap()
            .is_exhaustive()
    );
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

fn invalid(ty: &Ty, pat: Pat) -> Error {
    check(&Lang, ty, &[Arm::new(Pat::wild()), Arm::new(pat)]).unwrap_err()
}

#[test]
fn test_invalid_patterns_are_reported_with_position() {
    let at = |node, reason| Error::InvalidPattern {
        arm: 1,
        node,
        reason,
    };
    assert_eq!(invalid(&Ty::Bool, Pat::int(1)), at(0, Reason::Kind));
    assert_eq!(
        invalid(&Ty::Color, Pat::variant(3, [])),
        at(0, Reason::Variant { index: 3, count: 3 })
    );
    assert_eq!(
        invalid(&opt(Ty::Bool), Pat::variant(1, [])),
        at(
            0,
            Reason::Fields {
                expected: 1,
                found: 0
            }
        )
    );
    assert_eq!(
        invalid(
            &Ty::Tuple(vec![Ty::Bool]),
            Pat::product([Pat::wild(), Pat::wild()])
        ),
        at(
            0,
            Reason::Fields {
                expected: 1,
                found: 2
            }
        )
    );
    assert_eq!(
        invalid(&Ty::U8, Pat::range(5, 4)),
        at(0, Reason::EmptyRange)
    );
    assert_eq!(invalid(&Ty::U8, Pat::int(256)), at(0, Reason::OutOfDomain));
    assert_eq!(invalid(&Ty::U8, Pat::int(-1)), at(0, Reason::OutOfDomain));
    assert_eq!(
        invalid(&Ty::BigInt, Pat::range(1, 0)),
        at(0, Reason::EmptyRange)
    );
    assert_eq!(
        invalid(&Ty::Char, Pat::char_range('b', 'a')),
        at(0, Reason::EmptyRange)
    );
    assert_eq!(invalid(&Ty::Char, Pat::int(97)), at(0, Reason::Kind));
    assert_eq!(invalid(&Ty::U8, Pat::char('a')), at(0, Reason::Kind));
    let arr = Ty::Array(2, Box::new(Ty::Bool));
    assert_eq!(
        invalid(&arr, Pat::slice([Pat::wild()])),
        at(
            0,
            Reason::Length {
                expected: 2,
                found: 1
            }
        )
    );
    assert_eq!(
        invalid(
            &arr,
            Pat::slice_rest([Pat::wild(), Pat::wild()], [Pat::wild()])
        ),
        at(
            0,
            Reason::Length {
                expected: 2,
                found: 3
            }
        )
    );
    // Nested: node numbering is preorder within the arm.
    let ty = Ty::Tuple(vec![Ty::Bool, opt(Ty::U8)]);
    let pat = Pat::product([Pat::bool(true), some(Pat::or([Pat::int(1), Pat::int(300)]))]);
    assert_eq!(invalid(&ty, pat), at(5, Reason::OutOfDomain));
    // Wildcards never fail, whatever the type.
    assert!(check(&Lang, &Ty::Never, &arms([Pat::or([Pat::wild()])])).is_ok());
}

/// A model whose answers change after a number of calls.
struct Fickle {
    calls: Cell<u32>,
    flip_after: u32,
}

impl Model for Fickle {
    type Ty = u8;

    fn signature(&self, ty: &u8) -> Signature<u8> {
        self.calls.set(self.calls.get() + 1);
        let flipped = self.calls.get() > self.flip_after;
        match (ty, flipped) {
            (0, false) => Signature::Sum { count: 2 },
            (0, true) => Signature::Int { min: 0, max: 1 },
            (_, false) => Signature::Bool,
            (_, true) => Signature::Product,
        }
    }

    fn fields(&self, _: &u8, variant: u32, out: &mut Vec<u8>) {
        self.calls.set(self.calls.get() + 1);
        if variant == 1 && self.calls.get() <= self.flip_after {
            out.push(1);
        }
    }
}

#[test]
fn test_inconsistent_model_is_an_error_not_a_panic() {
    let a = arms([Pat::variant(1, [Pat::bool(true)]), Pat::variant(0, [])]);
    let mut saw_error = false;
    for flip_after in 0..20 {
        let model = Fickle {
            calls: Cell::new(0),
            flip_after,
        };
        match check(&model, &0, &a) {
            Ok(_) => {}
            Err(Error::Model { .. } | Error::InvalidPattern { .. }) => saw_error = true,
            Err(other) => panic!("unexpected error {other:?}"),
        }
    }
    assert!(saw_error);
}

struct BackwardsInt;

impl Model for BackwardsInt {
    type Ty = ();
    fn signature(&self, _: &()) -> Signature<()> {
        Signature::Int { min: 5, max: 4 }
    }
    fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
}

#[test]
fn test_backwards_int_signature_is_a_model_error() {
    assert!(matches!(
        check(&BackwardsInt, &(), &[]),
        Err(Error::Model { .. })
    ));
    assert!(matches!(
        check(&BackwardsInt, &(), &arms([Pat::int(5)])),
        Err(Error::Model { .. })
    ));
}

#[test]
fn test_checker_debug_and_default() {
    let checker: Checker<Ty> = Checker::default();
    let text = format!("{checker:?}");
    assert!(text.contains("step_limit"));
    assert!(text.contains("witness_limit"));
}
