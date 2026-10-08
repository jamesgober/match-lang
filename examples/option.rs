//! The one-call path: check a match over `Option<bool>`.
//!
//! ```text
//! cargo run --example option
//! ```

use match_lang::{Arm, Model, Pat, Signature, check};

/// The language's types.
#[derive(Clone, Debug)]
enum Ty {
    Bool,
    Option(Box<Ty>),
}

/// How the language's types are built: `bool` from two constants, and
/// `Option<T>` from `None` (variant 0) and `Some(T)` (variant 1).
struct Lang;

impl Model for Lang {
    type Ty = Ty;

    fn signature(&self, ty: &Ty) -> Signature<Ty> {
        match ty {
            Ty::Bool => Signature::Bool,
            Ty::Option(_) => Signature::Sum { count: 2 },
        }
    }

    fn fields(&self, ty: &Ty, variant: u32, out: &mut Vec<Ty>) {
        if let (Ty::Option(inner), 1) = (ty, variant) {
            out.push((**inner).clone());
        }
    }

    fn variant_name(&self, _: &Ty, variant: u32) -> Option<&str> {
        Some(if variant == 0 { "None" } else { "Some" })
    }
}

fn main() -> Result<(), match_lang::Error> {
    let ty = Ty::Option(Box::new(Ty::Bool));
    // match x {
    //     Some(true) => ...,
    //     None => ...,
    //     Some(true) | None => ...,
    // }
    let arms = [
        Arm::new(Pat::variant(1, [Pat::bool(true)])),
        Arm::new(Pat::variant(0, [])),
        Arm::new(Pat::or([
            Pat::variant(1, [Pat::bool(true)]),
            Pat::variant(0, []),
        ])),
    ];

    let report = check(&Lang, &ty, &arms)?;
    for witness in report.missing() {
        println!("not covered: {}", witness.display(&Lang));
    }
    for arm in report.unreachable() {
        println!("arm {arm} is unreachable: {}", arms[*arm].pat());
    }
    Ok(())
}
