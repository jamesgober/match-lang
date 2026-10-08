//! The usefulness algorithm.
//!
//! This is Maranget's usefulness computation ("Warnings for pattern
//! matching", JFP 2007) in the form modern compilers use: every row of the
//! pattern matrix is analysed in one pass, constructors are *split* so that
//! a column only ever specializes by the constructors its patterns mention
//! plus one "missing" constructor standing for all the others, and the
//! witnesses of non-exhaustiveness are built on the way back up.
//!
//! Three things keep it safe on hostile input:
//!
//! - **No recursion.** The recursion over specialized matrices runs on an
//!   explicit stack of [`Frame`]s, so neither a deeply nested pattern nor a
//!   hundred-thousand-field tuple can overflow the call stack.
//! - **No copying of row tails.** A row is a persistent stack of cells:
//!   specializing pushes the head's fields onto the shared tail instead of
//!   copying the rest of the row, so a wide matrix costs time linear in its
//!   size rather than quadratic.
//! - **A step budget.** Exhaustiveness is NP-hard in general; every unit of
//!   work is charged against a limit, and running out is an error value.
//!
//! All arenas are stacks: a child frame's rows, cells, types and split data
//! sit above its parent's and are truncated when it finishes, so a check's
//! memory is bounded by the deepest path, and the arenas are reused by the
//! next check.

use alloc::{collections::BTreeSet, vec::Vec};

use crate::{
    Arm, Error, Model, Reason, Report, Signature,
    pat::{Pat, PatKind},
    report::Redundant,
    witness::{Ctor, WNode, Witness},
};

/// "No index": an empty stack, a wildcard cell, or no parent.
const NIL: u32 = u32::MAX;

/// Characters are analysed as a contiguous integer range by closing the
/// surrogate gap: scalar values from `0xE000` up are shifted down by the
/// gap's width, so every index maps back to a valid `char`.
const SURROGATE_START: u32 = 0xD800;
const SURROGATE_COUNT: u32 = 0x800;
const CHAR_MAX_INDEX: i128 = 0x10_FFFF - SURROGATE_COUNT as i128;

const SIGNATURE_CHANGED: &str = "a type's signature changed during the check";
const FIELDS_CHANGED: &str = "a constructor's field count changed during the check";
const BAD_INT: &str = "an Int signature has min > max";

fn char_index(c: char) -> i128 {
    let cp = c as u32;
    i128::from(if cp < SURROGATE_START {
        cp
    } else {
        cp - SURROGATE_COUNT
    })
}

fn index_char(index: i128) -> char {
    let i = u32::try_from(index).unwrap_or(0);
    let cp = if i < SURROGATE_START {
        i
    } else {
        i.saturating_add(SURROGATE_COUNT)
    };
    // Indices come from `char_index` or from segment bounds inside
    // `0..=CHAR_MAX_INDEX`, which all map to scalar values.
    char::from_u32(cp).unwrap_or(char::MAX)
}

/// Converts an arena length to an index, refusing the `NIL` sentinel.
fn index(len: usize) -> Result<u32, Error> {
    match u32::try_from(len) {
        Ok(i) if i != NIL => Ok(i),
        _ => Err(Error::TooLarge),
    }
}

/// A flattened pattern node's test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Wild,
    Or,
    Variant(u32),
    Product,
    Bool(bool),
    /// Integers, or (with `char`) character indices.
    Range {
        lo: i128,
        hi: i128,
        char: bool,
    },
    Lit(u64),
    Slice {
        prefix: u32,
        rest: bool,
    },
}

impl Kind {
    fn lower(kind: PatKind) -> Kind {
        match kind {
            PatKind::Wild => Kind::Wild,
            PatKind::Or => Kind::Or,
            PatKind::Variant(v) => Kind::Variant(v),
            PatKind::Product => Kind::Product,
            PatKind::Bool(b) => Kind::Bool(b),
            PatKind::Int { lo, hi } => Kind::Range {
                lo,
                hi,
                char: false,
            },
            PatKind::Char { lo, hi } => Kind::Range {
                lo: char_index(lo),
                hi: char_index(hi),
                char: true,
            },
            PatKind::Lit(id) => Kind::Lit(id),
            PatKind::Slice { prefix, rest } => Kind::Slice { prefix, rest },
        }
    }
}

/// A pattern node, numbered in preorder; its children are
/// `kids[start..start + len]`.
#[derive(Clone, Copy, Debug)]
struct Node {
    kind: Kind,
    start: u32,
    len: u32,
    /// The node is an alternative of an or-pattern.
    alt: bool,
}

/// A matrix row: a stack of cells (head first), the row it was specialized
/// from, and its usefulness so far.
#[derive(Clone, Copy, Debug)]
struct Row {
    stack: u32,
    parent: u32,
    guarded: bool,
    useful: bool,
    /// Whether this row's usefulness still needs computing here (see
    /// `push_child`).
    relevant: bool,
}

/// One column entry of a row: a pattern node, or `NIL` for a wildcard the
/// algorithm introduced.
#[derive(Clone, Copy, Debug)]
struct Cell {
    pat: u32,
    next: u32,
}

/// One column type; the matrix's column types form a stack like its rows.
struct TyCell<T> {
    ty: T,
    next: u32,
}

/// A constructor a column is specialized by, after splitting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Split {
    Variant(u32),
    Product,
    Bool(bool),
    /// A segment of an integer (or character-index) domain.
    Range(i128, i128),
    Lit(u64),
    /// Sequences of exactly this length.
    Slice(u32),
    /// Sequences of `len` elements or (with `open`) more, every one of them
    /// seen through its first `prefix` and last `suffix` elements, which do
    /// not overlap (`prefix + suffix <= len`).
    SliceVar {
        prefix: u32,
        suffix: u32,
        len: u32,
        open: bool,
    },
    /// Every constructor the column does not mention.
    Missing,
    /// Values no pattern can name. Only ever listed as missing.
    Other,
}

/// A constructor to specialize by, with the rows (relative to the frame)
/// that survive and, once known, its arity.
#[derive(Clone, Copy, Debug)]
struct SplitCtor {
    ctor: Split,
    members: (u32, u32),
    arity: u32,
}

/// The signature of a frame's head column, kept for building witnesses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SigKind {
    Sum,
    Product,
    Bool,
    Int,
    Char,
    BigInt,
    Opaque,
    Array,
    Slice,
}

/// Arena lengths when a frame was created; restored when it finishes.
#[derive(Clone, Copy, Debug)]
struct Mark {
    rows: usize,
    cells: usize,
    tys: usize,
    splits: usize,
    members: usize,
    missing: usize,
}

/// A witness under construction: one subtree per column, the head column's
/// last, each subtree stored root-last with its first field nearest the root.
type Stack<T> = Vec<WNode<T>>;

/// One level of the (explicit) recursion: a matrix and the progress made
/// specializing it.
struct Frame<T> {
    rows: (u32, u32),
    /// The head column type, or `NIL` when no columns are left.
    tys: u32,
    /// Whether witnesses found below this frame are wanted (see `push_child`).
    relevant: bool,
    /// Whether the head column is the scrutinee itself.
    top: bool,
    /// Whether the head column has been split (or the base case handled).
    ready: bool,
    splits: (u32, u32),
    /// The next split constructor to specialize by.
    next: u32,
    missing: (u32, u32),
    /// Report missing constructors one by one rather than as `_`.
    individual: bool,
    sig: SigKind,
    elem: Option<T>,
    witnesses: Vec<Stack<T>>,
    mark: Mark,
}

impl<T> Frame<T> {
    fn new(rows: (u32, u32), tys: u32, relevant: bool, top: bool, mark: Mark) -> Frame<T> {
        Frame {
            rows,
            tys,
            relevant,
            top,
            ready: false,
            splits: (0, 0),
            next: 0,
            missing: (0, 0),
            individual: false,
            sig: SigKind::Product,
            elem: None,
            witnesses: Vec::new(),
            mark,
        }
    }
}

/// Merges two ascending row lists into `out`.
fn merge(a: impl Iterator<Item = u32>, b: &[u32], out: &mut Vec<u32>) {
    let mut b = b.iter().copied().peekable();
    for x in a {
        while let Some(y) = b.next_if(|&y| y < x) {
            out.push(y);
        }
        out.push(x);
    }
    out.extend(b);
}

/// The checker's state: the flattened patterns, the matrix arenas, and
/// scratch buffers, all reused from one check to the next.
pub(crate) struct Engine<T> {
    nodes: Vec<Node>,
    kids: Vec<u32>,
    roots: Vec<u32>,
    alt_useful: Vec<bool>,
    rows: Vec<Row>,
    cells: Vec<Cell>,
    tys: Vec<TyCell<T>>,
    frames: Vec<Frame<T>>,
    splits: Vec<SplitCtor>,
    members: Vec<u32>,
    missing: Vec<Split>,
    missing_at: usize,
    field_tys: Vec<T>,
    field_pats: Vec<u32>,
    wild: Vec<u32>,
    heads: Vec<(u32, u32)>,
    keyed: Vec<(u64, u32)>,
    rest: Vec<(u64, u32, u32, u32)>,
    lens: Vec<u64>,
    present: Vec<u32>,
    ranges: Vec<(i128, i128, u32)>,
    ends: Vec<(i128, u32)>,
    points: Vec<i128>,
    merged: Vec<u32>,
    or_stack: Vec<u32>,
    pending: Vec<(u32, T)>,
    active: BTreeSet<u32>,
    steps: u64,
    limit: u64,
    witness_limit: usize,
}

impl<T: Clone> Engine<T> {
    pub(crate) fn new() -> Engine<T> {
        Engine {
            nodes: Vec::new(),
            kids: Vec::new(),
            roots: Vec::new(),
            alt_useful: Vec::new(),
            rows: Vec::new(),
            cells: Vec::new(),
            tys: Vec::new(),
            frames: Vec::new(),
            splits: Vec::new(),
            members: Vec::new(),
            missing: Vec::new(),
            missing_at: 0,
            field_tys: Vec::new(),
            field_pats: Vec::new(),
            wild: Vec::new(),
            heads: Vec::new(),
            keyed: Vec::new(),
            rest: Vec::new(),
            lens: Vec::new(),
            present: Vec::new(),
            ranges: Vec::new(),
            ends: Vec::new(),
            points: Vec::new(),
            merged: Vec::new(),
            or_stack: Vec::new(),
            pending: Vec::new(),
            active: BTreeSet::new(),
            steps: 0,
            limit: 0,
            witness_limit: 1,
        }
    }

    /// Checks `arms` against values of type `ty`.
    pub(crate) fn check<M: Model<Ty = T>>(
        &mut self,
        model: &M,
        ty: &T,
        arms: &[Arm],
        limit: u64,
        witness_limit: usize,
    ) -> Result<Report<T>, Error> {
        self.reset(limit, witness_limit);
        self.flatten(arms)?;
        for arm in 0..self.roots.len() {
            self.validate(model, ty, arm)?;
        }
        let witnesses = self.run(model, ty, arms)?;
        let unreachable: Vec<usize> = (0..arms.len())
            .filter(|&arm| !self.rows.get(arm).is_some_and(|row| row.useful))
            .collect();
        self.close_alternatives();
        let redundant = self.redundant(arms.len());
        let missing = witnesses.into_iter().map(Witness::from_nodes).collect();
        Ok(Report {
            missing,
            unreachable,
            redundant,
            steps: self.steps,
        })
    }

    fn reset(&mut self, limit: u64, witness_limit: usize) {
        self.nodes.clear();
        self.kids.clear();
        self.roots.clear();
        self.alt_useful.clear();
        self.rows.clear();
        self.cells.clear();
        self.tys.clear();
        self.frames.clear();
        self.splits.clear();
        self.members.clear();
        self.missing.clear();
        self.pending.clear();
        self.steps = 0;
        self.limit = limit;
        self.witness_limit = witness_limit.max(1);
    }

    /// Charges `n` units of work against the step limit.
    fn charge(&mut self, n: usize) -> Result<(), Error> {
        self.steps = self.steps.saturating_add(n as u64);
        if self.steps > self.limit {
            Err(Error::StepLimit { limit: self.limit })
        } else {
            Ok(())
        }
    }

    // -----------------------------------------------------------------------
    // Patterns
    // -----------------------------------------------------------------------

    /// Flattens every arm's pattern into `nodes`, numbering nodes in preorder.
    fn flatten(&mut self, arms: &[Arm]) -> Result<(), Error> {
        // (pattern, slot in `kids` waiting for its id, is an alternative)
        let mut stack: Vec<(&Pat, u32, bool)> = Vec::new();
        for arm in arms {
            self.roots.push(index(self.nodes.len())?);
            stack.push((arm.pat(), NIL, false));
            while let Some((pat, slot, alt)) = stack.pop() {
                let id = index(self.nodes.len())?;
                if let Some(slot) = self.kids.get_mut(slot as usize) {
                    *slot = id;
                }
                let start = index(self.kids.len())?;
                let len = index(pat.children.len())?;
                let end = index(self.kids.len().saturating_add(pat.children.len()))?;
                self.kids.resize(end as usize, NIL);
                self.nodes.push(Node {
                    kind: Kind::lower(pat.kind),
                    start,
                    len,
                    alt,
                });
                let child_alt = pat.kind == PatKind::Or;
                for (i, child) in pat.children.iter().enumerate().rev() {
                    // `start + i < end`, which `index` checked fits.
                    stack.push((child, start + i as u32, child_alt));
                }
            }
        }
        self.alt_useful.resize(self.nodes.len(), false);
        Ok(())
    }

    fn children(&self, node: Node) -> &[u32] {
        let start = node.start as usize;
        self.kids
            .get(start..start + node.len as usize)
            .unwrap_or(&[])
    }

    /// Checks that arm `arm`'s pattern fits the scrutinee type.
    fn validate<M: Model<Ty = T>>(&mut self, model: &M, ty: &T, arm: usize) -> Result<(), Error> {
        let root = self.roots[arm];
        let invalid = |id: u32, reason| Error::InvalidPattern {
            arm,
            node: (id - root) as usize,
            reason,
        };
        self.pending.clear();
        self.pending.push((root, ty.clone()));
        while let Some((id, ty)) = self.pending.pop() {
            self.charge(1)?;
            let node = self.nodes[id as usize];
            if node.kind == Kind::Wild {
                continue;
            }
            if node.kind == Kind::Or {
                for i in (node.start..node.start + node.len).rev() {
                    self.pending.push((self.kids[i as usize], ty.clone()));
                }
                continue;
            }
            match (node.kind, model.signature(&ty)) {
                (Kind::Variant(v), Signature::Sum { count }) => {
                    if v >= count {
                        return Err(invalid(id, Reason::Variant { index: v, count }));
                    }
                    self.push_fields(model, &ty, v, node)
                        .map_err(|r| invalid(id, r))?;
                }
                (Kind::Product, Signature::Product) => {
                    self.push_fields(model, &ty, 0, node)
                        .map_err(|r| invalid(id, r))?;
                }
                (Kind::Bool(_), Signature::Bool) | (Kind::Lit(_), Signature::Opaque) => {}
                (
                    Kind::Range {
                        lo,
                        hi,
                        char: false,
                    },
                    Signature::Int { min, max },
                ) => {
                    if min > max {
                        return Err(Error::Model { reason: BAD_INT });
                    }
                    if lo > hi {
                        return Err(invalid(id, Reason::EmptyRange));
                    }
                    if lo < min || hi > max {
                        return Err(invalid(id, Reason::OutOfDomain));
                    }
                }
                (
                    Kind::Range {
                        lo,
                        hi,
                        char: false,
                    },
                    Signature::BigInt,
                )
                | (Kind::Range { lo, hi, char: true }, Signature::Char) => {
                    if lo > hi {
                        return Err(invalid(id, Reason::EmptyRange));
                    }
                }
                (Kind::Slice { rest, .. }, Signature::Array { len, elem }) => {
                    if (!rest && node.len != len) || (rest && node.len > len) {
                        let found = node.len as usize;
                        return Err(invalid(
                            id,
                            Reason::Length {
                                expected: len,
                                found,
                            },
                        ));
                    }
                    self.push_elems(node, &elem)?;
                }
                (Kind::Slice { .. }, Signature::Slice { elem }) => self.push_elems(node, &elem)?,
                _ => return Err(invalid(id, Reason::Kind)),
            }
        }
        Ok(())
    }

    /// Queues the field patterns of `node` with the field types of its
    /// constructor, or says why they do not fit.
    fn push_fields<M: Model<Ty = T>>(
        &mut self,
        model: &M,
        ty: &T,
        variant: u32,
        node: Node,
    ) -> Result<(), Reason> {
        self.field_tys.clear();
        model.fields(ty, variant, &mut self.field_tys);
        if self.field_tys.len() != node.len as usize {
            return Err(Reason::Fields {
                expected: self.field_tys.len(),
                found: node.len as usize,
            });
        }
        for (i, field) in self.field_tys.drain(..).enumerate().rev() {
            self.pending
                .push((self.kids[node.start as usize + i], field));
        }
        Ok(())
    }

    fn push_elems(&mut self, node: Node, elem: &T) -> Result<(), Error> {
        self.charge(node.len as usize)?;
        for i in (node.start..node.start + node.len).rev() {
            self.pending.push((self.kids[i as usize], elem.clone()));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // The matrix
    // -----------------------------------------------------------------------

    fn mark(&self) -> Mark {
        Mark {
            rows: self.rows.len(),
            cells: self.cells.len(),
            tys: self.tys.len(),
            splits: self.splits.len(),
            members: self.members.len(),
            missing: self.missing.len(),
        }
    }

    fn release(&mut self, mark: Mark) {
        self.rows.truncate(mark.rows);
        self.cells.truncate(mark.cells);
        self.tys.truncate(mark.tys);
        self.splits.truncate(mark.splits);
        self.members.truncate(mark.members);
        self.missing.truncate(mark.missing);
    }

    fn push_cell(&mut self, pat: u32, next: u32) -> Result<u32, Error> {
        let at = index(self.cells.len())?;
        self.cells.push(Cell { pat, next });
        Ok(at)
    }

    /// Adds a row whose cells start at `stack`. A head or-pattern is
    /// expanded in place into one row per alternative, nested or-patterns
    /// flattened, in left-to-right order.
    fn push_row(
        &mut self,
        stack: u32,
        parent: u32,
        guarded: bool,
        relevant: bool,
    ) -> Result<(), Error> {
        let head = self.cells.get(stack as usize).map_or(NIL, |cell| cell.pat);
        let is_or = self
            .nodes
            .get(head as usize)
            .is_some_and(|n| n.kind == Kind::Or);
        if !is_or {
            let _row = index(self.rows.len())?;
            self.rows.push(Row {
                stack,
                parent,
                guarded,
                useful: false,
                relevant,
            });
            return Ok(());
        }
        let rest = self.cells[stack as usize].next;
        self.or_stack.clear();
        self.or_stack.push(head);
        while let Some(id) = self.or_stack.pop() {
            let node = self.nodes[id as usize];
            if node.kind == Kind::Or {
                for i in (node.start..node.start + node.len).rev() {
                    self.or_stack.push(self.kids[i as usize]);
                }
                continue;
            }
            self.charge(1)?;
            let cell = self.push_cell(id, rest)?;
            let _row = index(self.rows.len())?;
            self.rows.push(Row {
                stack: cell,
                parent,
                guarded,
                useful: false,
                relevant,
            });
        }
        Ok(())
    }

    /// Runs the algorithm, returning the scrutinee's witnesses. Row
    /// usefulness ends up on the arm rows, `rows[..arms.len()]`.
    fn run<M: Model<Ty = T>>(
        &mut self,
        model: &M,
        ty: &T,
        arms: &[Arm],
    ) -> Result<Vec<Stack<T>>, Error> {
        for arm in arms {
            let guarded = arm.is_guarded();
            self.rows.push(Row {
                stack: NIL,
                parent: NIL,
                guarded,
                useful: false,
                relevant: true,
            });
        }
        let arm_count = index(arms.len())?;
        let mark = self.mark();
        self.tys.push(TyCell {
            ty: ty.clone(),
            next: NIL,
        });
        for (i, arm) in arms.iter().enumerate() {
            let cell = self.push_cell(self.roots[i], NIL)?;
            // `i < arm_count`, which fits in a `u32`.
            self.push_row(cell, i as u32, arm.is_guarded(), true)?;
        }
        let rows = (arm_count, index(self.rows.len())?);
        self.frames.push(Frame::new(rows, 0, true, true, mark));
        loop {
            let Some(top) = self.frames.len().checked_sub(1) else {
                return Ok(Vec::new());
            };
            if !self.frames[top].ready {
                self.frames[top].ready = true;
                if self.frames[top].tys == NIL {
                    self.base_case(top)?;
                } else {
                    self.split(model, top)?;
                }
            }
            let frame = &self.frames[top];
            if frame.next < frame.splits.1 - frame.splits.0 {
                self.push_child(model, top)?;
            } else if let Some(witnesses) = self.pop_frame(model)? {
                return Ok(witnesses);
            }
        }
    }

    /// No columns left: a row is useful when every row above it is guarded,
    /// and the wildcard row (the values not yet matched) survives when every
    /// row is.
    fn base_case(&mut self, top: usize) -> Result<(), Error> {
        let (start, end) = self.frames[top].rows;
        self.charge((end - start) as usize)?;
        let mut open = true;
        for row in &mut self.rows[start as usize..end as usize] {
            if !open {
                break;
            }
            row.useful = true;
            open = row.guarded;
        }
        let at = index(self.splits.len())?;
        let frame = &mut self.frames[top];
        frame.splits = (at, at);
        if open && frame.relevant {
            frame.witnesses.push(Vec::new());
        }
        Ok(())
    }

    /// Finishes the top frame: passes usefulness to the parent rows and to
    /// or-alternatives, frees its arenas, and hands its witnesses up. Returns
    /// the witnesses when the finished frame was the scrutinee's.
    fn pop_frame<M: Model<Ty = T>>(&mut self, model: &M) -> Result<Option<Vec<Stack<T>>>, Error> {
        let Some(frame) = self.frames.pop() else {
            return Ok(Some(Vec::new()));
        };
        let (start, end) = frame.rows;
        self.charge((end - start) as usize)?;
        for r in start..end {
            let row = self.rows[r as usize];
            if !row.useful {
                continue;
            }
            if let Some(parent) = self.rows.get_mut(row.parent as usize) {
                parent.useful = true;
            }
            let head = self
                .cells
                .get(row.stack as usize)
                .map_or(NIL, |cell| cell.pat);
            if self.nodes.get(head as usize).is_some_and(|node| node.alt) {
                self.alt_useful[head as usize] = true;
            }
        }
        self.release(frame.mark);
        let Some(parent) = self.frames.len().checked_sub(1) else {
            return Ok(Some(frame.witnesses));
        };
        self.apply(model, parent, frame.witnesses)?;
        self.frames[parent].next += 1;
        Ok(None)
    }

    // -----------------------------------------------------------------------
    // Splitting a column
    // -----------------------------------------------------------------------

    /// Splits the head column of frame `top` into the constructors to
    /// specialize by, and the constructors no row mentions.
    fn split<M: Model<Ty = T>>(&mut self, model: &M, top: usize) -> Result<(), Error> {
        let frame = &self.frames[top];
        let (start, end) = frame.rows;
        let is_top = frame.top;
        let ty = self.tys[frame.tys as usize].ty.clone();
        self.wild.clear();
        self.heads.clear();
        for r in start..end {
            let stack = self.rows[r as usize].stack;
            let cell = self.cells.get(stack as usize).ok_or(Error::Model {
                reason: FIELDS_CHANGED,
            })?;
            let wild = cell.pat == NIL || self.nodes[cell.pat as usize].kind == Kind::Wild;
            if wild {
                self.wild.push(r - start);
            } else {
                self.heads.push((r - start, cell.pat));
            }
        }
        self.charge((end - start) as usize)?;
        let splits_at = index(self.splits.len())?;
        self.missing_at = self.missing.len();
        let (sig, elem) = match model.signature(&ty) {
            Signature::Sum { count } => {
                self.split_sum(model, &ty, count, false)?;
                (SigKind::Sum, None)
            }
            Signature::Bool => {
                self.split_sum(model, &ty, 2, true)?;
                (SigKind::Bool, None)
            }
            Signature::Product => {
                self.field_tys.clear();
                model.fields(&ty, 0, &mut self.field_tys);
                self.charge(self.field_tys.len())?;
                let inhabited = self.field_tys.iter().all(|t| !model.is_empty(t));
                self.split_single(Split::Product, inhabited)?;
                (SigKind::Product, None)
            }
            Signature::Int { min, max } => {
                if min > max {
                    return Err(Error::Model { reason: BAD_INT });
                }
                self.split_ranges(min, max, false, false)?;
                (SigKind::Int, None)
            }
            Signature::Char => {
                self.split_ranges(0, CHAR_MAX_INDEX, true, false)?;
                (SigKind::Char, None)
            }
            Signature::BigInt => {
                self.split_ranges(i128::MIN, i128::MAX, false, true)?;
                (SigKind::BigInt, None)
            }
            Signature::Opaque => {
                self.split_lits()?;
                (SigKind::Opaque, None)
            }
            Signature::Array { len, elem } => {
                let inhabited = len == 0 || !model.is_empty(&elem);
                self.split_single(Split::Slice(len), inhabited)?;
                (SigKind::Array, Some(elem))
            }
            Signature::Slice { elem } => {
                let elem_empty = model.is_empty(&elem);
                self.split_slices(elem_empty)?;
                (SigKind::Slice, Some(elem))
            }
        };
        let all_missing = self.splits.len() == splits_at as usize;
        if self.missing.len() > self.missing_at {
            let at = index(self.members.len())?;
            self.members.extend_from_slice(&self.wild);
            let members = (at, index(self.members.len())?);
            self.splits.push(SplitCtor {
                ctor: Split::Missing,
                members,
                arity: 0,
            });
        }
        let splits = (splits_at, index(self.splits.len())?);
        let missing = (index(self.missing_at)?, index(self.missing.len())?);
        let frame = &mut self.frames[top];
        frame.splits = splits;
        frame.missing = missing;
        // At the top every missing constructor is worth naming; deeper down,
        // a column no row mentions is just `_`.
        frame.individual = is_top || !all_missing;
        frame.sig = sig;
        frame.elem = elem;
        Ok(())
    }

    /// Records a missing constructor, up to the witness limit.
    fn push_missing(&mut self, ctor: Split) {
        if self.missing.len() - self.missing_at < self.witness_limit {
            self.missing.push(ctor);
        }
    }

    /// Records a constructor to specialize by, with the rows in `merged`.
    fn push_split(&mut self, ctor: Split) -> Result<(), Error> {
        self.charge(self.merged.len())?;
        let at = index(self.members.len())?;
        self.members.extend_from_slice(&self.merged);
        let members = (at, index(self.members.len())?);
        self.splits.push(SplitCtor {
            ctor,
            members,
            arity: 0,
        });
        Ok(())
    }

    /// Sums (and booleans, as the sum `{false, true}`): one constructor per
    /// variant the column mentions, the unmentioned inhabited ones missing.
    fn split_sum<M: Model<Ty = T>>(
        &mut self,
        model: &M,
        ty: &T,
        count: u32,
        bool: bool,
    ) -> Result<(), Error> {
        let ctor = |variant: u32| {
            if bool {
                Split::Bool(variant == 1)
            } else {
                Split::Variant(variant)
            }
        };
        self.keyed.clear();
        for &(row, node) in &self.heads {
            let key = match self.nodes[node as usize].kind {
                Kind::Variant(v) if !bool && v < count => v,
                Kind::Bool(b) if bool => u32::from(b),
                _ => {
                    return Err(Error::Model {
                        reason: SIGNATURE_CHANGED,
                    });
                }
            };
            self.keyed.push((u64::from(key), row));
        }
        self.keyed.sort_unstable();
        self.present.clear();
        let mut i = 0;
        while i < self.keyed.len() {
            let key = self.keyed[i].0;
            let mut j = i + 1;
            while j < self.keyed.len() && self.keyed[j].0 == key {
                j += 1;
            }
            // Keys come from `u32` variants.
            let variant = key as u32;
            self.present.push(variant);
            if bool || self.variant_inhabited(model, ty, variant)? {
                self.merged.clear();
                merge(
                    self.keyed[i..j].iter().map(|k| k.1),
                    &self.wild,
                    &mut self.merged,
                );
                self.push_split(ctor(variant))?;
            }
            i = j;
        }
        // Scan for unmentioned variants, stopping at the witness limit: the
        // sum may have billions of variants.
        let mut present = 0;
        let mut variant = 0;
        while variant < count && self.missing.len() - self.missing_at < self.witness_limit {
            self.charge(1)?;
            if self.present.get(present) == Some(&variant) {
                present += 1;
            } else if bool || self.variant_inhabited(model, ty, variant)? {
                self.missing.push(ctor(variant));
            }
            variant += 1;
        }
        Ok(())
    }

    /// Whether variant `variant` of `ty` has values: none of its fields is
    /// an empty type.
    fn variant_inhabited<M: Model<Ty = T>>(
        &mut self,
        model: &M,
        ty: &T,
        variant: u32,
    ) -> Result<bool, Error> {
        self.field_tys.clear();
        model.fields(ty, variant, &mut self.field_tys);
        self.charge(self.field_tys.len())?;
        Ok(self.field_tys.iter().all(|t| !model.is_empty(t)))
    }

    /// Products and arrays: a single constructor.
    fn split_single(&mut self, ctor: Split, inhabited: bool) -> Result<(), Error> {
        for &(_, node) in &self.heads {
            let fits = matches!(
                (self.nodes[node as usize].kind, ctor),
                (Kind::Product, Split::Product) | (Kind::Slice { .. }, Split::Slice(_))
            );
            if !fits {
                return Err(Error::Model {
                    reason: SIGNATURE_CHANGED,
                });
            }
        }
        if !inhabited {
            return Ok(());
        }
        if self.heads.is_empty() {
            self.push_missing(ctor);
            return Ok(());
        }
        self.merged.clear();
        merge(self.heads.iter().map(|h| h.0), &self.wild, &mut self.merged);
        self.push_split(ctor)
    }

    /// Integers and characters: the domain `lo..=hi` is cut at every range
    /// boundary the column mentions, so that each segment is either inside
    /// or outside every range. Covered segments are specialized by;
    /// maximal runs of uncovered ones are missing.
    fn split_ranges(&mut self, lo: i128, hi: i128, char: bool, other: bool) -> Result<(), Error> {
        self.ranges.clear();
        for &(row, node) in &self.heads {
            match self.nodes[node as usize].kind {
                Kind::Range {
                    lo: a,
                    hi: b,
                    char: c,
                } if c == char && lo <= a && a <= b && b <= hi => {
                    self.ranges.push((a, b, row));
                }
                _ => {
                    return Err(Error::Model {
                        reason: SIGNATURE_CHANGED,
                    });
                }
            }
        }
        self.points.clear();
        self.points.push(lo);
        for &(a, b, _) in &self.ranges {
            self.points.push(a);
            if b < hi {
                self.points.push(b + 1);
            }
        }
        self.points.sort_unstable();
        self.points.dedup();
        self.ranges.sort_unstable_by_key(|r| r.0);
        self.ends.clear();
        self.ends.extend(self.ranges.iter().map(|r| (r.1, r.2)));
        self.ends.sort_unstable();
        self.active.clear();
        let (mut next_start, mut next_end) = (0, 0);
        let mut gap: Option<i128> = None;
        for j in 0..self.points.len() {
            self.charge(1)?;
            let seg_lo = self.points[j];
            let seg_hi = self.points.get(j + 1).map_or(hi, |next| next - 1);
            while let Some(&(_, _, row)) = self.ranges.get(next_start).filter(|r| r.0 <= seg_lo) {
                let _new = self.active.insert(row);
                next_start += 1;
            }
            while let Some(&(_, row)) = self.ends.get(next_end).filter(|e| e.0 < seg_lo) {
                let _was = self.active.remove(&row);
                next_end += 1;
            }
            if self.active.is_empty() {
                let _gap = gap.get_or_insert(seg_lo);
                continue;
            }
            if let Some(gap_lo) = gap.take() {
                self.push_missing(Split::Range(gap_lo, seg_lo - 1));
            }
            self.merged.clear();
            merge(self.active.iter().copied(), &self.wild, &mut self.merged);
            self.push_split(Split::Range(seg_lo, seg_hi))?;
        }
        if let Some(gap_lo) = gap {
            self.push_missing(Split::Range(gap_lo, hi));
        }
        if other {
            self.push_missing(Split::Other);
        }
        Ok(())
    }

    /// Opaque literals: one constructor per literal mentioned; every other
    /// value is missing.
    fn split_lits(&mut self) -> Result<(), Error> {
        self.keyed.clear();
        for &(row, node) in &self.heads {
            match self.nodes[node as usize].kind {
                Kind::Lit(id) => self.keyed.push((id, row)),
                _ => {
                    return Err(Error::Model {
                        reason: SIGNATURE_CHANGED,
                    });
                }
            }
        }
        self.keyed.sort_unstable();
        let mut i = 0;
        while i < self.keyed.len() {
            let id = self.keyed[i].0;
            let mut j = i + 1;
            while j < self.keyed.len() && self.keyed[j].0 == id {
                j += 1;
            }
            self.merged.clear();
            merge(
                self.keyed[i..j].iter().map(|k| k.1),
                &self.wild,
                &mut self.merged,
            );
            self.push_split(Split::Lit(id))?;
            i = j;
        }
        self.push_missing(Split::Other);
        Ok(())
    }

    /// Variable-length slices.
    ///
    /// Lengths are grouped into intervals, cut at every length a pattern
    /// singles out: each fixed-length pattern's length `f` (and `f + 1`) and
    /// each rest pattern's minimum length `a + b`. Inside one interval the
    /// same rest patterns apply to every length. Once the length reaches
    /// `pa + pb`, the longest prefix and suffix among them, prefixes and
    /// suffixes no longer overlap, so every such length is analysed the same
    /// way, through its first `pa` and last `pb` elements: the whole run is
    /// one constructor. Below `pa + pb` the alignment differs from length to
    /// length, so those lengths are constructors of their own. This is the
    /// slice splitting compilers use, without enumerating every length up to
    /// the longest pattern: one long pattern beside `[..]` costs its length,
    /// not its length squared.
    fn split_slices(&mut self, elem_empty: bool) -> Result<(), Error> {
        self.keyed.clear();
        self.rest.clear();
        for &(row, node) in &self.heads {
            let node = self.nodes[node as usize];
            let Kind::Slice { prefix, rest } = node.kind else {
                return Err(Error::Model {
                    reason: SIGNATURE_CHANGED,
                });
            };
            if rest {
                let prefix = prefix.min(node.len);
                self.rest
                    .push((u64::from(node.len), prefix, node.len - prefix, row));
            } else {
                self.keyed.push((u64::from(node.len), row));
            }
        }
        self.keyed.sort_unstable();
        self.rest.sort_unstable();
        if elem_empty {
            // Only the empty sequence exists.
            self.merged.clear();
            self.merged
                .extend(self.keyed.iter().take_while(|f| f.0 == 0).map(|f| f.1));
            self.merged
                .extend(self.rest.iter().take_while(|r| r.0 == 0).map(|r| r.3));
            if self.merged.is_empty() {
                self.push_missing(Split::Slice(0));
                return Ok(());
            }
            self.merged.extend_from_slice(&self.wild);
            self.merged.sort_unstable();
            return self.push_split(Split::Slice(0));
        }
        self.lens.clear();
        self.lens.push(0);
        self.lens.extend(self.rest.iter().map(|r| r.0));
        for &(len, _) in &self.keyed {
            self.lens.extend([len, len + 1]);
        }
        self.lens.sort_unstable();
        self.lens.dedup();
        self.active.clear();
        let (mut fixed, mut rest) = (0, 0);
        let (mut pa, mut pb) = (0u32, 0u32);
        for i in 0..self.lens.len() {
            self.charge(1)?;
            let lo = self.lens[i];
            let hi = self.lens.get(i + 1).map(|next| next - 1);
            while let Some(&(_, a, b, row)) = self.rest.get(rest).filter(|r| r.0 <= lo) {
                let _new = self.active.insert(row);
                pa = pa.max(a);
                pb = pb.max(b);
                rest += 1;
            }
            let group = fixed;
            while self.keyed.get(fixed).is_some_and(|f| f.0 == lo) {
                fixed += 1;
            }
            if fixed > group {
                // A fixed length is an interval of its own: `f + 1` is a cut.
                self.merged.clear();
                self.merged
                    .extend(self.keyed[group..fixed].iter().map(|f| f.1));
                self.merged.extend(self.active.iter().copied());
                self.merged.extend_from_slice(&self.wild);
                self.merged.sort_unstable();
                self.push_split(Split::Slice(length(lo)?))?;
                continue;
            }
            if self.active.is_empty() {
                match hi {
                    Some(hi) => {
                        let mut n = lo;
                        while n <= hi && self.missing.len() - self.missing_at < self.witness_limit {
                            self.charge(1)?;
                            self.push_missing(Split::Slice(length(n)?));
                            n += 1;
                        }
                    }
                    None => {
                        let len = length(lo)?;
                        self.push_missing(Split::SliceVar {
                            prefix: 0,
                            suffix: 0,
                            len,
                            open: true,
                        });
                    }
                }
                continue;
            }
            let aligned = u64::from(pa) + u64::from(pb);
            let mut n = lo;
            while n < aligned && hi.is_none_or(|hi| n <= hi) {
                self.charge(1)?;
                self.merged.clear();
                merge(self.active.iter().copied(), &self.wild, &mut self.merged);
                self.push_split(Split::Slice(length(n)?))?;
                n += 1;
            }
            if hi.is_none_or(|hi| n <= hi) {
                self.merged.clear();
                merge(self.active.iter().copied(), &self.wild, &mut self.merged);
                let (len, open) = (length(n)?, hi.is_none());
                self.push_split(Split::SliceVar {
                    prefix: pa,
                    suffix: pb,
                    len,
                    open,
                })?;
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Specializing
    // -----------------------------------------------------------------------

    /// Builds the matrix for frame `top`'s next split constructor and pushes
    /// it as a new frame.
    fn push_child<M: Model<Ty = T>>(&mut self, model: &M, top: usize) -> Result<(), Error> {
        let frame = &self.frames[top];
        let at = (frame.splits.0 + frame.next) as usize;
        let split = self.splits[at];
        let row_base = frame.rows.0;
        // Relevancy, which is what keeps the algorithm from exploring an
        // exponential number of branches on ordinary matches. When some
        // constructor is missing, a present constructor `c` is specialized
        // by only to learn which rows are useful:
        //
        // - The witnesses below `c` are not wanted: the missing constructors
        //   already prove the match non-exhaustive.
        // - A row with a wildcard head learns nothing below `c`. It is also
        //   in the missing branch with the same tail, after a subset of the
        //   rows it follows here, so any value reaching it below `c` has a
        //   counterpart reaching it there.
        //
        // Such rows stay in the matrix, since they still shadow the rows
        // after them, but rows after the last relevant one can no longer
        // matter and are dropped, and a branch with no relevant row left is
        // skipped. Marking an irrelevant row useful anyway would be correct:
        // this only saves work.
        let ctor_relevant = split.ctor == Split::Missing || frame.missing.0 == frame.missing.1;
        let relevant = frame.relevant && ctor_relevant;
        let elem = frame.elem.clone();
        let head = &self.tys[frame.tys as usize];
        let (head_ty, rest_ty) = (head.ty.clone(), head.next);
        let mark = self.mark();

        self.field_tys.clear();
        match split.ctor {
            Split::Variant(v) => model.fields(&head_ty, v, &mut self.field_tys),
            Split::Product => model.fields(&head_ty, 0, &mut self.field_tys),
            Split::Slice(n) => self.repeat_elem(elem.as_ref(), u64::from(n))?,
            Split::SliceVar { prefix, suffix, .. } => {
                self.repeat_elem(elem.as_ref(), u64::from(prefix) + u64::from(suffix))?;
            }
            _ => {}
        }
        let arity = index(self.field_tys.len())?;
        self.charge(self.field_tys.len())?;
        self.splits[at].arity = arity;
        let mut tys = rest_ty;
        while let Some(ty) = self.field_tys.pop() {
            let cell = index(self.tys.len())?;
            self.tys.push(TyCell { ty, next: tys });
            tys = cell;
        }

        let child_start = index(self.rows.len())?;
        for k in split.members.0..split.members.1 {
            let parent = row_base + self.members[k as usize];
            let row = self.rows[parent as usize];
            let cell = *self.cells.get(row.stack as usize).ok_or(Error::Model {
                reason: FIELDS_CHANGED,
            })?;
            self.field_pats.clear();
            if split.ctor != Split::Missing {
                self.fields_of(cell.pat, split.ctor, arity)?;
            }
            self.charge(1 + self.field_pats.len())?;
            let mut stack = cell.next;
            for i in (0..self.field_pats.len()).rev() {
                stack = self.push_cell(self.field_pats[i], stack)?;
            }
            let wild_head = self
                .nodes
                .get(cell.pat as usize)
                .is_none_or(|n| n.kind == Kind::Wild);
            let row_relevant = row.relevant && (ctor_relevant || !wild_head);
            self.push_row(stack, parent, row.guarded, row_relevant)?;
        }
        if !relevant {
            let last = self.rows[child_start as usize..]
                .iter()
                .rposition(|row| row.relevant);
            match last {
                Some(last) => self.rows.truncate(child_start as usize + last + 1),
                None => {
                    // Nothing to learn below this constructor.
                    self.release(mark);
                    self.frames[top].next += 1;
                    return Ok(());
                }
            }
        }
        let rows = (child_start, index(self.rows.len())?);
        self.frames
            .push(Frame::new(rows, tys, relevant, false, mark));
        Ok(())
    }

    /// Fills `field_tys` with `count` copies of the element type.
    fn repeat_elem(&mut self, elem: Option<&T>, count: u64) -> Result<(), Error> {
        let elem = elem.ok_or(Error::Model {
            reason: SIGNATURE_CHANGED,
        })?;
        let count = usize::try_from(count).map_err(|_| Error::TooLarge)?;
        self.charge(count)?;
        self.field_tys
            .extend(core::iter::repeat_n(elem, count).cloned());
        Ok(())
    }

    /// Fills `field_pats` with the fields a row's head pattern `pat` has
    /// under constructor `ctor`: its own fields, or wildcards.
    fn fields_of(&mut self, pat: u32, ctor: Split, arity: u32) -> Result<(), Error> {
        let changed = Error::Model {
            reason: FIELDS_CHANGED,
        };
        let Some(&node) = self
            .nodes
            .get(pat as usize)
            .filter(|n| n.kind != Kind::Wild)
        else {
            self.field_pats.resize(arity as usize, NIL);
            return Ok(());
        };
        let start = node.start as usize;
        let kids = self
            .kids
            .get(start..start + node.len as usize)
            .ok_or(changed.clone())?;
        match (node.kind, ctor) {
            (Kind::Variant(a), Split::Variant(b)) if a == b && node.len == arity => {
                self.field_pats.extend_from_slice(kids);
            }
            (Kind::Product, Split::Product) if node.len == arity => {
                self.field_pats.extend_from_slice(kids);
            }
            (Kind::Bool(_), Split::Bool(_))
            | (Kind::Range { .. }, Split::Range(..))
            | (Kind::Lit(_), Split::Lit(_)) => {}
            (Kind::Slice { rest: false, .. }, Split::Slice(n)) if node.len == n => {
                self.field_pats.extend_from_slice(kids);
            }
            (Kind::Slice { prefix, rest: true }, Split::Slice(n)) if node.len <= n => {
                let (pre, post) = kids.split_at((prefix as usize).min(kids.len()));
                self.field_pats.extend_from_slice(pre);
                self.field_pats
                    .resize(self.field_pats.len() + (n - node.len) as usize, NIL);
                self.field_pats.extend_from_slice(post);
            }
            (
                Kind::Slice { prefix, rest: true },
                Split::SliceVar {
                    prefix: p,
                    suffix: s,
                    ..
                },
            ) => {
                let (pre, post) = kids.split_at((prefix as usize).min(kids.len()));
                if pre.len() > p as usize || post.len() > s as usize {
                    return Err(changed);
                }
                self.field_pats.extend_from_slice(pre);
                let gap = (p as usize - pre.len()) + (s as usize - post.len());
                self.field_pats.resize(self.field_pats.len() + gap, NIL);
                self.field_pats.extend_from_slice(post);
            }
            _ => return Err(changed),
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Witnesses
    // -----------------------------------------------------------------------

    /// Turns witnesses of a child frame into witnesses of frame `parent`, by
    /// applying the constructor the child was specialized by.
    fn apply<M: Model<Ty = T>>(
        &mut self,
        model: &M,
        parent: usize,
        found: Vec<Stack<T>>,
    ) -> Result<(), Error> {
        if found.is_empty() {
            return Ok(());
        }
        let frame = &self.frames[parent];
        let split = self.splits[(frame.splits.0 + frame.next) as usize];
        let head_ty = self.tys[frame.tys as usize].ty.clone();
        let (sig, individual, missing, elem) = (
            frame.sig,
            frame.individual,
            frame.missing,
            frame.elem.clone(),
        );
        let mut out = core::mem::take(&mut self.frames[parent].witnesses);
        let limit = self.witness_limit;
        for mut stack in found {
            if out.len() >= limit {
                break;
            }
            match split.ctor {
                Split::Missing if individual => {
                    for k in missing.0..missing.1 {
                        if out.len() >= limit {
                            break;
                        }
                        let mut copy = stack.clone();
                        self.charge(copy.len())?;
                        let ctor = self.missing[k as usize];
                        self.push_missing_node(
                            model,
                            &mut copy,
                            ctor,
                            &head_ty,
                            sig,
                            elem.as_ref(),
                        )?;
                        out.push(copy);
                    }
                }
                Split::Missing => {
                    self.charge(1)?;
                    stack.push(WNode {
                        ctor: Ctor::Wild,
                        ty: head_ty.clone(),
                        arity: 0,
                        size: 1,
                    });
                    out.push(stack);
                }
                Split::SliceVar {
                    prefix,
                    suffix,
                    len,
                    ..
                } => {
                    // The child saw `prefix + suffix` elements; the witness
                    // shows the run's shortest length, wildcards between.
                    let gap = len - prefix - suffix;
                    self.charge(len as usize + 1)?;
                    let elem = elem.clone().ok_or(Error::Model {
                        reason: SIGNATURE_CHANGED,
                    })?;
                    insert_gap(&mut stack, prefix, gap, elem)?;
                    push_node(
                        &mut stack,
                        public_ctor(split.ctor, sig),
                        head_ty.clone(),
                        len,
                    )?;
                    out.push(stack);
                }
                ctor => {
                    self.charge(split.arity as usize + 1)?;
                    push_node(
                        &mut stack,
                        public_ctor(ctor, sig),
                        head_ty.clone(),
                        split.arity,
                    )?;
                    out.push(stack);
                }
            }
        }
        self.frames[parent].witnesses = out;
        Ok(())
    }

    /// Pushes a missing constructor with wildcard fields onto a witness.
    fn push_missing_node<M: Model<Ty = T>>(
        &mut self,
        model: &M,
        stack: &mut Stack<T>,
        ctor: Split,
        ty: &T,
        sig: SigKind,
        elem: Option<&T>,
    ) -> Result<(), Error> {
        self.field_tys.clear();
        match ctor {
            Split::Variant(v) => model.fields(ty, v, &mut self.field_tys),
            Split::Product => model.fields(ty, 0, &mut self.field_tys),
            Split::Slice(n) => self.repeat_elem(elem, u64::from(n))?,
            // A missing run of lengths is shown at its shortest, all wildcards.
            Split::SliceVar { len, .. } => self.repeat_elem(elem, u64::from(len))?,
            _ => {}
        }
        let arity = index(self.field_tys.len())?;
        self.charge(self.field_tys.len() + 1)?;
        // Fields go in last-first, so the first field ends up nearest the root.
        while let Some(field) = self.field_tys.pop() {
            stack.push(WNode {
                ctor: Ctor::Wild,
                ty: field,
                arity: 0,
                size: 1,
            });
        }
        stack.push(WNode {
            ctor: public_ctor(ctor, sig),
            ty: ty.clone(),
            arity,
            size: arity + 1,
        });
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Or-pattern alternatives
    // -----------------------------------------------------------------------

    /// An alternative that is itself an or-pattern was flattened away when
    /// expanded, so it is useful when one of its own alternatives is.
    fn close_alternatives(&mut self) {
        for id in (0..self.nodes.len()).rev() {
            let node = self.nodes[id];
            if node.kind == Kind::Or && node.alt {
                let useful = self
                    .children(node)
                    .iter()
                    .any(|&c| self.alt_useful[c as usize]);
                self.alt_useful[id] = useful;
            }
        }
    }

    /// The outermost useless alternatives of reachable arms, in preorder.
    fn redundant(&mut self, arms: usize) -> Vec<Redundant> {
        let mut found = Vec::new();
        for arm in 0..arms {
            if !self.rows.get(arm).is_some_and(|row| row.useful) {
                continue;
            }
            let root = self.roots[arm];
            self.or_stack.clear();
            self.or_stack.push(root);
            while let Some(id) = self.or_stack.pop() {
                let node = self.nodes[id as usize];
                if node.alt && !self.alt_useful[id as usize] {
                    found.push(Redundant {
                        arm,
                        node: (id - root) as usize,
                    });
                    continue;
                }
                let start = node.start as usize;
                for &child in self.kids[start..start + node.len as usize].iter().rev() {
                    self.or_stack.push(child);
                }
            }
        }
        found
    }
}

/// Inserts `gap` wildcard subtrees under the top `prefix` subtrees of a
/// witness: between a slice's prefix fields and its suffix fields.
fn insert_gap<T: Clone>(stack: &mut Stack<T>, prefix: u32, gap: u32, ty: T) -> Result<(), Error> {
    let mut cut = stack.len();
    for _ in 0..prefix {
        let root = cut.checked_sub(1).ok_or(Error::Model {
            reason: FIELDS_CHANGED,
        })?;
        cut = (root + 1)
            .checked_sub(stack[root].size as usize)
            .ok_or(Error::Model {
                reason: FIELDS_CHANGED,
            })?;
    }
    let wild = WNode {
        ctor: Ctor::Wild,
        ty,
        arity: 0,
        size: 1,
    };
    drop(stack.splice(cut..cut, core::iter::repeat_n(wild, gap as usize)));
    Ok(())
}

/// Converts a slice length to the `u32` constructors carry.
fn length(len: u64) -> Result<u32, Error> {
    u32::try_from(len).map_err(|_| Error::TooLarge)
}

/// Pushes a constructor node over the top `arity` subtrees of a witness.
fn push_node<T>(stack: &mut Stack<T>, ctor: Ctor, ty: T, arity: u32) -> Result<(), Error> {
    let short = Error::Model {
        reason: FIELDS_CHANGED,
    };
    let mut end = stack.len();
    let mut size: u32 = 1;
    for _ in 0..arity {
        let root = end.checked_sub(1).ok_or(short.clone())?;
        let field = stack[root].size;
        size = size.checked_add(field).ok_or(Error::TooLarge)?;
        end = end.checked_sub(field as usize).ok_or(short.clone())?;
    }
    stack.push(WNode {
        ctor,
        ty,
        arity,
        size,
    });
    Ok(())
}

/// The public form of a split constructor.
fn public_ctor(ctor: Split, sig: SigKind) -> Ctor {
    match ctor {
        Split::Variant(v) => Ctor::Variant(v),
        Split::Product => Ctor::Product,
        Split::Bool(b) => Ctor::Bool(b),
        Split::Range(lo, hi) if sig == SigKind::Char => Ctor::Char {
            lo: index_char(lo),
            hi: index_char(hi),
        },
        Split::Range(lo, hi) => Ctor::Int { lo, hi },
        Split::Lit(id) => Ctor::Lit(id),
        Split::Slice(len) => Ctor::Slice { len },
        Split::SliceVar {
            suffix,
            len,
            open: true,
            ..
        } => Ctor::SliceRest {
            prefix: len - suffix,
            suffix,
        },
        Split::SliceVar {
            len, open: false, ..
        } => Ctor::Slice { len },
        Split::Other => Ctor::Other,
        Split::Missing => Ctor::Wild,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_char_index_round_trips_around_the_surrogate_gap() {
        for c in ['\0', 'a', '\u{D7FF}', '\u{E000}', '\u{FFFF}', char::MAX] {
            assert_eq!(index_char(char_index(c)), c);
        }
        assert_eq!(char_index('\u{E000}'), char_index('\u{D7FF}') + 1);
        assert_eq!(char_index(char::MAX), CHAR_MAX_INDEX);
    }

    #[test]
    fn test_merge_interleaves_in_order() {
        let mut out = Vec::new();
        merge([1, 4, 6].into_iter(), &[0, 2, 3, 7], &mut out);
        assert_eq!(out, [0, 1, 2, 3, 4, 6, 7]);
        out.clear();
        merge(core::iter::empty(), &[5], &mut out);
        assert_eq!(out, [5]);
    }

    #[test]
    fn test_index_refuses_the_sentinel() {
        assert_eq!(index(5), Ok(5));
        assert_eq!(index(NIL as usize), Err(Error::TooLarge));
    }

    #[test]
    fn test_push_node_rejects_missing_fields() {
        let mut stack: Stack<()> = Vec::new();
        assert!(push_node(&mut stack, Ctor::Product, (), 1).is_err());
        push_node(&mut stack, Ctor::Wild, (), 0).unwrap();
        push_node(&mut stack, Ctor::Product, (), 1).unwrap();
        assert_eq!(stack[1].size, 2);
    }
}
