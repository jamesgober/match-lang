//! A small language's match checker: named enums and structs, integers,
//! strings, slices, guards, and or-patterns, with compiler-style messages.
//!
//! The language keeps its own pattern tree (as a real front end keeps its
//! HIR), lowers it to `match_lang::Pat`, and maps the preorder node indices
//! in the report back to its own nodes.
//!
//! ```text
//! cargo run --example language
//! ```

use match_lang::{Arm, Checker, Model, Pat, Signature};

/// The language's types.
#[derive(Clone, Debug)]
enum Ty {
    U8,
    Str,
    Bool,
    /// `enum Shape { Circle(u8), Rect(u8, u8), Empty }`
    Shape,
    /// `struct Point { x: u8, y: u8 }`
    Point,
    /// `[T]`
    Slice(Box<Ty>),
}

struct Lang;

impl Model for Lang {
    type Ty = Ty;

    fn signature(&self, ty: &Ty) -> Signature<Ty> {
        match ty {
            Ty::U8 => Signature::Int { min: 0, max: 255 },
            Ty::Str => Signature::Opaque,
            Ty::Bool => Signature::Bool,
            Ty::Shape => Signature::Sum { count: 3 },
            Ty::Point => Signature::Product,
            Ty::Slice(elem) => Signature::Slice {
                elem: (**elem).clone(),
            },
        }
    }

    fn fields(&self, ty: &Ty, variant: u32, out: &mut Vec<Ty>) {
        match (ty, variant) {
            (Ty::Shape, 0) => out.push(Ty::U8),
            (Ty::Shape, 1) | (Ty::Point, _) => out.extend([Ty::U8, Ty::U8]),
            _ => {}
        }
    }

    fn variant_name(&self, ty: &Ty, variant: u32) -> Option<&str> {
        match (ty, variant) {
            (Ty::Shape, 0) => Some("Circle"),
            (Ty::Shape, 1) => Some("Rect"),
            (Ty::Shape, 2) => Some("Empty"),
            (Ty::Point, _) => Some("Point"),
            _ => None,
        }
    }
}

/// The language's own patterns, with the source text of each node.
enum Src {
    Wild,
    Bool(bool),
    Int(i128, i128, &'static str),
    Str(&'static str),
    Variant(u32, Vec<Src>, &'static str),
    Struct(Vec<Src>, &'static str),
    Or(Vec<Src>),
    Slice(Vec<Src>, Option<Vec<Src>>, &'static str),
}

impl Src {
    /// Lowers to the checker's pattern. String literals are interned by
    /// their text: equal strings get equal ids.
    fn lower(&self, strings: &mut Vec<&'static str>) -> Pat {
        match self {
            Src::Wild => Pat::wild(),
            Src::Bool(b) => Pat::bool(*b),
            Src::Int(lo, hi, _) => Pat::range(*lo, *hi),
            Src::Str(text) => {
                let id = strings.iter().position(|s| s == text).unwrap_or_else(|| {
                    strings.push(text);
                    strings.len() - 1
                });
                Pat::lit(id as u64)
            }
            Src::Variant(v, fields, _) => Pat::variant(*v, fields.iter().map(|f| f.lower(strings))),
            Src::Struct(fields, _) => Pat::product(fields.iter().map(|f| f.lower(strings))),
            Src::Or(alts) => Pat::or(alts.iter().map(|a| a.lower(strings))),
            Src::Slice(prefix, None, _) => Pat::slice(prefix.iter().map(|p| p.lower(strings))),
            Src::Slice(prefix, Some(suffix), _) => {
                let prefix: Vec<Pat> = prefix.iter().map(|p| p.lower(strings)).collect();
                let suffix: Vec<Pat> = suffix.iter().map(|p| p.lower(strings)).collect();
                Pat::slice_rest(prefix, suffix)
            }
        }
    }

    fn text(&self) -> String {
        match self {
            Src::Wild => "_".into(),
            Src::Bool(b) => b.to_string(),
            Src::Int(_, _, t) | Src::Variant(_, _, t) | Src::Struct(_, t) | Src::Slice(_, _, t) => {
                (*t).into()
            }
            Src::Str(t) => format!("{t:?}"),
            Src::Or(alts) => alts.iter().map(Src::text).collect::<Vec<_>>().join(" | "),
        }
    }

    fn children(&self) -> Vec<&Src> {
        match self {
            Src::Variant(_, kids, _) | Src::Struct(kids, _) | Src::Or(kids) => {
                kids.iter().collect()
            }
            Src::Slice(prefix, suffix, _) => prefix.iter().chain(suffix.iter().flatten()).collect(),
            _ => Vec::new(),
        }
    }

    /// The node at a preorder index: the numbering the report uses.
    fn node(&self, index: usize) -> Option<&Src> {
        let mut stack = vec![self];
        let mut seen = 0;
        while let Some(node) = stack.pop() {
            if seen == index {
                return Some(node);
            }
            seen += 1;
            stack.extend(node.children().into_iter().rev());
        }
        None
    }
}

fn int(v: i128, text: &'static str) -> Src {
    Src::Int(v, v, text)
}

fn report(checker: &mut Checker<Ty>, name: &str, ty: Ty, arms: &[(Src, bool)]) {
    let mut strings = Vec::new();
    let lowered: Vec<Arm> = arms
        .iter()
        .map(|(src, guarded)| {
            let pat = src.lower(&mut strings);
            if *guarded {
                Arm::guarded(pat)
            } else {
                Arm::new(pat)
            }
        })
        .collect();
    println!("match {name}:");
    match checker.check(&Lang, &ty, &lowered) {
        Err(err) => println!("  error: {err}"),
        Ok(report) => {
            for w in report.missing() {
                println!("  error: pattern `{}` not covered", w.display(&Lang));
            }
            for &arm in report.unreachable() {
                println!(
                    "  warning: arm {arm} `{}` is unreachable",
                    arms[arm].0.text()
                );
            }
            for r in report.redundant() {
                let text = arms[r.arm]
                    .0
                    .node(r.node)
                    .map_or_else(String::new, Src::text);
                println!(
                    "  warning: in arm {}, alternative `{text}` is unreachable",
                    r.arm
                );
            }
            if report.is_exhaustive()
                && report.unreachable().is_empty()
                && report.redundant().is_empty()
            {
                println!("  ok");
            }
        }
    }
}

fn main() {
    let mut checker = Checker::new().with_witness_limit(4);

    // match shape { Circle(0) => .., Circle(_) => .., Rect(w, h) if w == h => .. }
    report(
        &mut checker,
        "shape",
        Ty::Shape,
        &[
            (Src::Variant(0, vec![int(0, "0")], "Circle(0)"), false),
            (Src::Variant(0, vec![Src::Wild], "Circle(_)"), false),
            (
                Src::Variant(1, vec![Src::Wild, Src::Wild], "Rect(w, h)"),
                true,
            ),
        ],
    );

    // match p { Point { x: 0, .. } => .., Point { x: 1..=9, y: 7 | 7 } => .., _ => .. }
    report(
        &mut checker,
        "point",
        Ty::Point,
        &[
            (
                Src::Struct(vec![int(0, "0"), Src::Wild], "Point { x: 0, .. }"),
                false,
            ),
            (
                Src::Struct(
                    vec![
                        Src::Int(1, 9, "1..=9"),
                        Src::Or(vec![int(7, "7"), int(7, "7")]),
                    ],
                    "Point { x: 1..=9, y: 7 | 7 }",
                ),
                false,
            ),
            (Src::Wild, false),
        ],
    );

    // match command { "go" | "stop" => .., "go" => .. }
    report(
        &mut checker,
        "command",
        Ty::Str,
        &[
            (Src::Or(vec![Src::Str("go"), Src::Str("stop")]), false),
            (Src::Str("go"), false),
        ],
    );

    // match flags { [] => .., [true, ..] => .., [.., false] => .. }
    report(
        &mut checker,
        "flags",
        Ty::Slice(Box::new(Ty::Bool)),
        &[
            (Src::Slice(vec![], None, "[]"), false),
            (
                Src::Slice(vec![Src::Bool(true)], Some(vec![]), "[true, ..]"),
                false,
            ),
            (
                Src::Slice(vec![], Some(vec![Src::Bool(false)]), "[.., false]"),
                false,
            ),
        ],
    );
}
