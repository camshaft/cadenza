//! The **Cedar surface** — an authorization-policy document as a projection of the one canonical arena.
//!
//! Cedar (AWS's authorization policy language) is a first-class front-end syntax, exactly like the
//! s-expression, ML, markdown, JSON, and TOML surfaces: a parser (`read`) turns Cedar policy text into
//! the shared [`Arenas`], and a printer (`print`) turns a Cedar arena back into policy text. It is not
//! privileged (`spec/contracts/ast-encoding.md` §A Textual Syntax Parses To And Prints From The
//! Canonical Form) — a `.cedar` reads to the same binary AST any surface does, so `cdz convert
//! policy.cedar --to binary` yields a canonical arena.
//!
//! Like the other data surfaces, a Cedar policy is *data* the compiler never sees. We parse only its
//! SYNTAX into the arena; we do NOT evaluate policies, and Cadenza bakes in no authorization engine.
//! The point is that once a policy is in the arena, Cadenza's whole query/rewrite/structural-editing
//! toolchain applies — an agent can construct or modify a policy (add a `when` clause, change an
//! effect) with the same tools it uses on any other surface.
//!
//! ## Representation
//!
//! The arena mirrors Cedar's `pst` ("policy syntax tree, designed for programmatic manipulation"). The
//! reader parses via `cedar_policy::PolicySet::from_str` and converts to a `pst::PolicySet`
//! (`try_into_pst`), then walks it into arena nodes; the printer reconstructs a `pst::PolicySet` from
//! the arena and renders it via `cedar_policy::PolicySet::from_pst(...).to_string()`. The `cedar-policy`
//! crate is a REFERENCE-only convenience (like pulldown-cmark/toml_edit); the durable artifact is the
//! `cedar-*` node vocabulary + round-trip, not this parser.
//!
//! ## Node vocabulary (all `Name`-headed lists + leaves — no codec change)
//!
//! Root: `(cedar-policyset <policy>…)`. Each policy:
//! `(cedar-policy <annotations> <effect> <principal> <action> <resource> <clause>…)`.
//! - `(annotations (annotation <key:Str> <value:Str>)…)`
//! - `(effect permit|forbid)`
//! - principal/resource: `(principal any)` | `(eq <eos>)` | `(in <eos>)` | `(is <ty:Str>)` |
//!   `(is-in <ty:Str> <eos>)`; `<eos>` = `(entity <ty:Str> <eid:Str>)` | `(slot principal|resource)`.
//! - action: `(action any)` | `(eq <entity>)` | `(in <entity>…)` (concrete entities only).
//! - `(when <expr>)` / `(unless <expr>)`.
//! - `<expr>`: `(lit-bool …)`/`(lit-long …)`/`(lit-string …)`/`(lit-entity <ty> <eid>)`,
//!   `(var principal|action|resource|context)`, `(slot principal|resource)`,
//!   `(unary <op> <e>)`, `(binary <op> <l> <r>)`, `(get <e> <attr>)`, `(has <e> <attr>…)`,
//!   `(like <e> <pattern-elem>…)`, `(is-expr <e> <ty> [<in>])`, `(if <c> <t> <e>)`, `(set <e>…)`,
//!   `(record (field <key> <e>)…)`, `(unknown <name>)`.
//!
//! ## Round-trip
//!
//! The `pst` tree drops comments and exact formatting, so the guarantee is **arena-idempotence**
//! (`read(print(read(x))) == read(x)` via `structurally_eq`), like the JSON surface — NOT byte
//! identity. A rewrite of any arena node reflects in the printed output (the arena is the
//! representation), which is the whole point for structural editing.

use cadenza_syntax_core::arena_read::{bool_leaf, child_tail, int_leaf, list_items, str_leaf};
use cadenza_syntax_core::ast::{Arenas, Builder, Leaf, Radix, StructId};
use cadenza_syntax_core::span::Span;
use cadenza_syntax_core::spans::{FileId, SpanTable};
use cedar_policy::pst;
use std::sync::Arc;

/// A Cedar parse failure, with a human-readable message (mirrors `sexpr::ReadError`).
#[derive(Debug)]
pub struct ReadError(pub String);

/// Parse Cedar policy `src` into a `(cedar-policyset …)` arena, or a [`ReadError`] on malformed input.
/// Fallible (like sexpr/json/toml) — a bad policy set is a clean error, never a patched-up tree.
pub fn read(src: &str) -> Result<Arenas, ReadError> {
    let set = parse_pst(src)?;
    let mut b = Builder::new();
    let root = Cedar::new(&mut b, None).policyset(&set);
    Ok(b.finish(root))
}

/// Parse Cedar policy `src` into a `(cedar-policyset …)` arena, ALSO producing a [`SpanTable`] 1:1 with
/// the arena. The `pst` carries no source spans, so every node gets a `Span::new(0, 0)` placeholder
/// (matching how the toml/markdown surfaces span synthesized nodes) — the table stays total + ordered,
/// which is what the query/rewrite path needs (a `.cedar` is data, never handed to the compiler).
pub fn read_spanned(src: &str) -> Result<(Arenas, SpanTable), ReadError> {
    let set = parse_pst(src)?;
    let mut b = Builder::new();
    let mut c = Cedar::new(&mut b, Some(SpanTable::new(FileId::default())));
    let root = c.policyset(&set);
    let spans = c.spans.take().expect("span tracking on");
    Ok((b.finish(root), spans))
}

fn parse_pst(src: &str) -> Result<pst::PolicySet, ReadError> {
    let set: cedar_policy::PolicySet = src.parse().map_err(|e| {
        ReadError(
            format!("{e}")
                .lines()
                .next()
                .unwrap_or("parse error")
                .to_string(),
        )
    })?;
    set.try_into_pst()
        .map_err(|e| ReadError(format!("cedar pst conversion: {e}")))
}

// ============================================================================
// Reader: cedar pst -> arena
// ============================================================================

/// The pst walker. Recurses `pst::PolicySet` → arena via `Builder`, mirroring the `mk_*`/`push_span`
/// discipline of the other surface readers (one span per created `StructId`, in id order).
struct Cedar<'b> {
    b: &'b mut Builder,
    spans: Option<SpanTable>,
}

impl<'b> Cedar<'b> {
    fn new(b: &'b mut Builder, spans: Option<SpanTable>) -> Cedar<'b> {
        Cedar { b, spans }
    }

    /// `(cedar-policyset <policy>…)` — static policies then templates, in the set's stored order.
    fn policyset(&mut self, set: &pst::PolicySet) -> StructId {
        let head = self.mk_name("cedar-policyset");
        let mut items = vec![head];
        for policy in set.policies.values() {
            let node = self.policy(policy.body());
            items.push(node);
        }
        for template in set.templates.values() {
            let node = self.policy(template);
            items.push(node);
        }
        self.mk_list(items)
    }

    /// `(cedar-policy <annotations> <effect> <principal> <action> <resource> <clause>…)`.
    fn policy(&mut self, t: &pst::Template) -> StructId {
        let head = self.mk_name("cedar-policy");
        let annotations = self.annotations(t);
        let effect = self.effect(t.effect);
        let principal = self.principal(&t.principal);
        let action = self.action(&t.action);
        let resource = self.resource(&t.resource);
        let mut items = vec![head, annotations, effect, principal, action, resource];
        for clause in t.clauses() {
            let c = self.clause(clause);
            items.push(c);
        }
        self.mk_list(items)
    }

    /// `(annotations (annotation <key:Str> <value:Str>)…)` — from the template's annotation map.
    fn annotations(&mut self, t: &pst::Template) -> StructId {
        let head = self.mk_name("annotations");
        let mut items = vec![head];
        for (key, value) in &t.annotations {
            let ahead = self.mk_name("annotation");
            let k = self.mk_str(key.clone());
            let v = self.mk_str(value.to_string());
            let a = self.mk_list(vec![ahead, k, v]);
            items.push(a);
        }
        self.mk_list(items)
    }

    fn effect(&mut self, e: pst::Effect) -> StructId {
        let head = self.mk_name("effect");
        let name = self.mk_name(match e {
            pst::Effect::Permit => "permit",
            pst::Effect::Forbid => "forbid",
        });
        self.mk_list(vec![head, name])
    }

    /// `(principal …)` — Any/Eq/In/Is/IsIn over an entity-or-slot.
    fn principal(&mut self, c: &pst::PrincipalConstraint) -> StructId {
        let head = self.mk_name("principal");
        let body = match c {
            pst::PrincipalConstraint::Any => vec![],
            pst::PrincipalConstraint::Eq(eos) => {
                let kw = self.mk_name("eq");
                let e = self.entity_or_slot(eos);
                vec![kw, e]
            }
            pst::PrincipalConstraint::In(eos) => {
                let kw = self.mk_name("in");
                let e = self.entity_or_slot(eos);
                vec![kw, e]
            }
            pst::PrincipalConstraint::Is(ty) => {
                let kw = self.mk_name("is");
                let t = self.mk_str(ty.to_string());
                vec![kw, t]
            }
            pst::PrincipalConstraint::IsIn(ty, eos) => {
                let kw = self.mk_name("is-in");
                let t = self.mk_str(ty.to_string());
                let e = self.entity_or_slot(eos);
                vec![kw, t, e]
            }
        };
        self.mk_scope(head, body)
    }

    /// `(resource …)` — identical shape to principal.
    fn resource(&mut self, c: &pst::ResourceConstraint) -> StructId {
        let head = self.mk_name("resource");
        let body = match c {
            pst::ResourceConstraint::Any => vec![],
            pst::ResourceConstraint::Eq(eos) => {
                let kw = self.mk_name("eq");
                let e = self.entity_or_slot(eos);
                vec![kw, e]
            }
            pst::ResourceConstraint::In(eos) => {
                let kw = self.mk_name("in");
                let e = self.entity_or_slot(eos);
                vec![kw, e]
            }
            pst::ResourceConstraint::Is(ty) => {
                let kw = self.mk_name("is");
                let t = self.mk_str(ty.to_string());
                vec![kw, t]
            }
            pst::ResourceConstraint::IsIn(ty, eos) => {
                let kw = self.mk_name("is-in");
                let t = self.mk_str(ty.to_string());
                let e = self.entity_or_slot(eos);
                vec![kw, t, e]
            }
        };
        self.mk_scope(head, body)
    }

    /// `(action …)` — diverges: concrete entities only, `In` is a list, no `Is`/slots.
    fn action(&mut self, c: &pst::ActionConstraint) -> StructId {
        let head = self.mk_name("action");
        let body = match c {
            pst::ActionConstraint::Any => vec![],
            pst::ActionConstraint::Eq(uid) => {
                let kw = self.mk_name("eq");
                let e = self.entity_uid(uid);
                vec![kw, e]
            }
            pst::ActionConstraint::In(uids) => {
                let kw = self.mk_name("in");
                let mut v = vec![kw];
                for uid in uids {
                    let e = self.entity_uid(uid);
                    v.push(e);
                }
                v
            }
        };
        self.mk_scope(head, body)
    }

    /// Build a scope node from a head plus a possibly-empty body. An empty body (`Any`) renders as
    /// `(principal any)` — a `(<head> any)` marker so the printer can tell "any" from a malformed node.
    fn mk_scope(&mut self, head: StructId, body: Vec<StructId>) -> StructId {
        if body.is_empty() {
            let any = self.mk_name("any");
            self.mk_list(vec![head, any])
        } else {
            let mut items = vec![head];
            items.extend(body);
            self.mk_list(items)
        }
    }

    /// `(entity <ty:Str> <eid:Str>)` | `(slot principal|resource)`.
    fn entity_or_slot(&mut self, eos: &pst::EntityOrSlot) -> StructId {
        match eos {
            pst::EntityOrSlot::Entity(uid) => self.entity_uid(uid),
            pst::EntityOrSlot::Slot(s) => self.slot(*s),
        }
    }

    /// `(entity <ty:Str> <eid:Str>)` — the type Displays as its qualified name (`Group`, `NS::Type`).
    fn entity_uid(&mut self, uid: &pst::EntityUID) -> StructId {
        let head = self.mk_name("entity");
        let ty = self.mk_str(uid.ty.to_string());
        let eid = self.mk_str(uid.eid.to_string());
        self.mk_list(vec![head, ty, eid])
    }

    fn slot(&mut self, s: pst::SlotId) -> StructId {
        let head = self.mk_name("slot");
        let which = self.mk_name(slot_name(s));
        self.mk_list(vec![head, which])
    }

    fn clause(&mut self, c: &pst::Clause) -> StructId {
        let (head, expr) = match c {
            pst::Clause::When(e) => ("when", e),
            pst::Clause::Unless(e) => ("unless", e),
        };
        let h = self.mk_name(head);
        let ex = self.expr(expr);
        self.mk_list(vec![h, ex])
    }

    /// Walk a pst `Expr` into an arena expression node.
    fn expr(&mut self, e: &pst::Expr) -> StructId {
        match e {
            pst::Expr::Literal(lit) => self.literal(lit),
            pst::Expr::Var(v) => {
                let head = self.mk_name("var");
                let name = self.mk_name(var_name(v));
                self.mk_list(vec![head, name])
            }
            pst::Expr::Slot(s) => self.slot(*s),
            pst::Expr::UnaryOp { op, expr } => {
                let head = self.mk_name("unary");
                let opn = self.mk_name(unary_op_name(*op));
                let ex = self.expr(expr);
                self.mk_list(vec![head, opn, ex])
            }
            pst::Expr::BinaryOp { op, left, right } => {
                let head = self.mk_name("binary");
                let opn = self.mk_name(binary_op_name(*op));
                let l = self.expr(left);
                let r = self.expr(right);
                self.mk_list(vec![head, opn, l, r])
            }
            pst::Expr::GetAttr { expr, attr } => {
                let head = self.mk_name("get");
                let ex = self.expr(expr);
                let a = self.mk_str(attr.to_string());
                self.mk_list(vec![head, ex, a])
            }
            pst::Expr::HasAttr { expr, attrs } => {
                let head = self.mk_name("has");
                let ex = self.expr(expr);
                let mut items = vec![head, ex];
                for a in attrs.iter() {
                    let s = self.mk_str(a.to_string());
                    items.push(s);
                }
                self.mk_list(items)
            }
            pst::Expr::Like { expr, pattern } => {
                let head = self.mk_name("like");
                let ex = self.expr(expr);
                let mut items = vec![head, ex];
                for elem in pattern {
                    let pe = self.pattern_elem(elem);
                    items.push(pe);
                }
                self.mk_list(items)
            }
            pst::Expr::Is {
                expr,
                entity_type,
                in_expr,
            } => {
                let head = self.mk_name("is-expr");
                let ex = self.expr(expr);
                let ty = self.mk_str(entity_type.to_string());
                let mut items = vec![head, ex, ty];
                if let Some(ie) = in_expr {
                    let i = self.expr(ie);
                    items.push(i);
                }
                self.mk_list(items)
            }
            pst::Expr::IfThenElse {
                cond,
                then_expr,
                else_expr,
            } => {
                let head = self.mk_name("if");
                let c = self.expr(cond);
                let t = self.expr(then_expr);
                let e = self.expr(else_expr);
                self.mk_list(vec![head, c, t, e])
            }
            pst::Expr::Set(exprs) => {
                let head = self.mk_name("set");
                let mut items = vec![head];
                for e in exprs {
                    let x = self.expr(e);
                    items.push(x);
                }
                self.mk_list(items)
            }
            pst::Expr::Record(map) => {
                let head = self.mk_name("record");
                let mut items = vec![head];
                for (key, value) in map {
                    let fhead = self.mk_name("field");
                    let k = self.mk_str(key.clone());
                    let v = self.expr(value);
                    let f = self.mk_list(vec![fhead, k, v]);
                    items.push(f);
                }
                self.mk_list(items)
            }
            pst::Expr::Unknown { name } => {
                let head = self.mk_name("unknown");
                let n = self.mk_str(name.to_string());
                self.mk_list(vec![head, n])
            }
            // `Expr` is #[non_exhaustive]; a future variant (or the `tpe`-only ResidualError) becomes an
            // explicit `(cedar-unsupported)` marker rather than being silently dropped.
            _ => self.mk_leaf_form("cedar-unsupported"),
        }
    }

    fn literal(&mut self, lit: &pst::Literal) -> StructId {
        match lit {
            pst::Literal::Bool(b) => {
                let head = self.mk_name("lit-bool");
                let v = self.mk_atom_leaf(Leaf::Bool(*b));
                self.mk_list(vec![head, v])
            }
            pst::Literal::Long(n) => {
                let head = self.mk_name("lit-long");
                let v = self.mk_atom_leaf(Leaf::Int {
                    value: cadenza_syntax_core::ast::IntValue::from_i64(*n),
                    radix: Radix::Dec,
                });
                self.mk_list(vec![head, v])
            }
            pst::Literal::String(s) => {
                let head = self.mk_name("lit-string");
                let v = self.mk_str(s.to_string());
                self.mk_list(vec![head, v])
            }
            pst::Literal::EntityUID(uid) => {
                let head = self.mk_name("lit-entity");
                let ty = self.mk_str(uid.ty.to_string());
                let eid = self.mk_str(uid.eid.to_string());
                self.mk_list(vec![head, ty, eid])
            }
            // `Literal` is #[non_exhaustive].
            _ => self.mk_leaf_form("cedar-unsupported"),
        }
    }

    fn pattern_elem(&mut self, elem: &pst::PatternElem) -> StructId {
        match elem {
            pst::PatternElem::Char(c) => {
                let head = self.mk_name("char");
                let v = self.mk_str(c.to_string());
                self.mk_list(vec![head, v])
            }
            pst::PatternElem::Wildcard => self.mk_leaf_form("wildcard"),
        }
    }

    // ---- span-recording arena helpers (mirror the other surfaces; one span per StructId) ----

    fn push_span(&mut self) {
        if let Some(t) = self.spans.as_mut() {
            debug_assert_eq!(
                t.len() + 1,
                self.b.structure_len(),
                "cedar span table drifted from the arena"
            );
            t.push(Span::new(0, 0));
        }
    }

    fn mk_name(&mut self, name: &str) -> StructId {
        let id = self.b.name(name);
        self.push_span();
        id
    }

    fn mk_str(&mut self, s: String) -> StructId {
        self.mk_atom_leaf(Leaf::Str(s.into()))
    }

    fn mk_atom_leaf(&mut self, leaf: Leaf) -> StructId {
        let id = self.b.atom_leaf(leaf);
        self.push_span();
        id
    }

    fn mk_list(&mut self, items: Vec<StructId>) -> StructId {
        let id = self.b.list(items);
        self.push_span();
        id
    }

    fn mk_leaf_form(&mut self, head: &str) -> StructId {
        let h = self.mk_name(head);
        self.mk_list(vec![h])
    }
}

/// The 18 `UnaryOp` variants → arena head names (kebab, stable). Round-trips via [`unary_op_from`].
fn unary_op_name(op: pst::UnaryOp) -> &'static str {
    match op {
        pst::UnaryOp::Not => "not",
        pst::UnaryOp::Neg => "neg",
        pst::UnaryOp::IsEmpty => "is-empty",
        pst::UnaryOp::Datetime => "datetime",
        pst::UnaryOp::Decimal => "decimal",
        pst::UnaryOp::Duration => "duration",
        pst::UnaryOp::Ip => "ip",
        pst::UnaryOp::IsIPv4 => "is-ipv4",
        pst::UnaryOp::IsIPV6 => "is-ipv6",
        pst::UnaryOp::IsLoopback => "is-loopback",
        pst::UnaryOp::IsMulticast => "is-multicast",
        pst::UnaryOp::ToDate => "to-date",
        pst::UnaryOp::ToTime => "to-time",
        pst::UnaryOp::ToMilliseconds => "to-milliseconds",
        pst::UnaryOp::ToSeconds => "to-seconds",
        pst::UnaryOp::ToMinutes => "to-minutes",
        pst::UnaryOp::ToHours => "to-hours",
        pst::UnaryOp::ToDays => "to-days",
        // #[non_exhaustive]
        _ => "unsupported",
    }
}

/// The 24 `BinaryOp` variants → arena head names.
fn binary_op_name(op: pst::BinaryOp) -> &'static str {
    match op {
        pst::BinaryOp::Eq => "eq",
        pst::BinaryOp::NotEq => "noteq",
        pst::BinaryOp::Less => "less",
        pst::BinaryOp::LessEq => "lesseq",
        pst::BinaryOp::Greater => "greater",
        pst::BinaryOp::GreaterEq => "greatereq",
        pst::BinaryOp::And => "and",
        pst::BinaryOp::Or => "or",
        pst::BinaryOp::Add => "add",
        pst::BinaryOp::Sub => "sub",
        pst::BinaryOp::Mul => "mul",
        pst::BinaryOp::In => "in",
        pst::BinaryOp::Contains => "contains",
        pst::BinaryOp::ContainsAll => "contains-all",
        pst::BinaryOp::ContainsAny => "contains-any",
        pst::BinaryOp::GetTag => "get-tag",
        pst::BinaryOp::HasTag => "has-tag",
        pst::BinaryOp::IsInRange => "is-in-range",
        pst::BinaryOp::Offset => "offset",
        pst::BinaryOp::DurationSince => "duration-since",
        pst::BinaryOp::DecimalLessThan => "decimal-lt",
        pst::BinaryOp::DecimalLessEq => "decimal-le",
        pst::BinaryOp::DecimalGreater => "decimal-gt",
        pst::BinaryOp::DecimalGreaterEq => "decimal-ge",
        // #[non_exhaustive]
        _ => "unsupported",
    }
}

fn var_name(v: &pst::Var) -> &'static str {
    match v {
        pst::Var::Principal => "principal",
        pst::Var::Action => "action",
        pst::Var::Resource => "resource",
        pst::Var::Context => "context",
    }
}

fn slot_name(s: pst::SlotId) -> &'static str {
    match s {
        pst::SlotId::Principal => "principal",
        pst::SlotId::Resource => "resource",
        _ => "principal", // #[non_exhaustive]; only two slots exist
    }
}

// ============================================================================
// Printer: arena -> cedar text (rebuild a pst::PolicySet, then Display)
// ============================================================================

/// Render a `(cedar-policyset …)` arena back to Cedar policy text by reconstructing a `pst::PolicySet`
/// and rendering it via `cedar_policy::PolicySet::from_pst(...).to_string()`. `width` is accepted for
/// surface-layer uniformity and ignored (Cedar's formatter fixes layout). A NON-Cedar root (a bare
/// program handed to `--to cedar`) becomes a single `//`-comment block over its ML rendering — a Cedar
/// file of only comments is valid and re-reads to an empty policy set, so `--to cedar` stays total.
///
/// `ml_print` renders an arbitrary arena as ML text — INJECTED (not called directly) so this crate
/// stays BELOW the ML surface: `cadenza-syntax-cedar` must not depend on the ML printer (the facade
/// re-exports this crate, so a dependency the other way would cycle). It is only ever invoked on the
/// non-Cedar fallback path; a genuine `(cedar-policyset …)` root never touches it. The facade passes
/// `cadenza_syntax::printer::print`.
pub fn print(arenas: &Arenas, width: usize, ml_print: fn(&Arenas, usize) -> String) -> String {
    if arenas.head_name(arenas.root) == Some("cedar-policyset") {
        match build_and_render(arenas) {
            Ok(s) => s,
            Err(_) => fallback(arenas, width, ml_print),
        }
    } else {
        fallback(arenas, width, ml_print)
    }
}

/// Carry a non-Cedar program's ML text as Cedar line-comments (total, re-reads to an empty set).
fn fallback(arenas: &Arenas, width: usize, ml_print: fn(&Arenas, usize) -> String) -> String {
    let ml = ml_print(arenas, width);
    let mut out = String::new();
    for line in ml.lines() {
        out.push_str("// ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Reconstruct each policy from the arena and render it via `pst::Template`'s own `Display` (which
/// produces valid Cedar for both static and slotted/template policies, unlike the top-level
/// `PolicySet::Display`, which only renders static policies). Policies are joined by a blank line — the
/// same separation `PolicySet::from_str` accepts. Any reconstruction error (a malformed tree, an
/// invalid identifier, an invalid `Display`) is returned so `print` can fall back rather than panic.
fn build_and_render(a: &Arenas) -> Result<String, String> {
    let mut rendered: Vec<String> = Vec::new();
    for (i, &p) in child_tail(a, a.root).iter().enumerate() {
        let id = pst::PolicyID(pst::SmolStr::from(format!("policy{i}")));
        let template = build_template(a, p, id)?;
        let text = template.to_string();
        // `Template::Display` writes `<invalid policy: …>` rather than erroring on a tree it can't
        // render; treat that as a reconstruction failure so we fall back instead of emitting garbage.
        if text.starts_with("<invalid policy") {
            return Err(text);
        }
        rendered.push(text);
    }
    Ok(rendered.join("\n\n"))
}

/// Reconstruct one `pst::Template` from a `(cedar-policy …)` node.
fn build_template(
    a: &Arenas,
    id: StructId,
    policy_id: pst::PolicyID,
) -> Result<pst::Template, String> {
    let items = list_items(a, id);
    // items[0]=head, [1]=annotations, [2]=effect, [3]=principal, [4]=action, [5]=resource, [6..]=clauses
    let effect = build_effect(a, *items.get(2).ok_or("missing effect")?)?;
    let principal = build_principal(a, *items.get(3).ok_or("missing principal")?)?;
    let action = build_action(a, *items.get(4).ok_or("missing action")?)?;
    let resource = build_resource(a, *items.get(5).ok_or("missing resource")?)?;
    let mut template = pst::Template::new(policy_id, effect, principal, action, resource);
    // Clauses.
    let mut clauses = Vec::new();
    for &c in &items[6.min(items.len())..] {
        clauses.push(build_clause(a, c)?);
    }
    template = template
        .try_with_clauses(clauses)
        .map_err(|e| format!("clauses: {e}"))?;
    // Annotations.
    let annotations = build_annotations(a, *items.get(1).ok_or("missing annotations")?)?;
    template = template.with_annotations(annotations);
    Ok(template)
}

fn build_annotations(
    a: &Arenas,
    id: StructId,
) -> Result<std::collections::BTreeMap<String, pst::SmolStr>, String> {
    let mut map = std::collections::BTreeMap::new();
    for &ann in &child_tail(a, id) {
        let it = list_items(a, ann);
        let key = str_leaf(a, *it.get(1).ok_or("annotation key")?).ok_or("annotation key str")?;
        let value = str_leaf(a, *it.get(2).ok_or("annotation value")?).unwrap_or_default();
        map.insert(key, pst::SmolStr::from(value));
    }
    Ok(map)
}

fn build_effect(a: &Arenas, id: StructId) -> Result<pst::Effect, String> {
    match a.as_name(list_items(a, id).get(1).copied().ok_or("effect kw")?) {
        Some("permit") => Ok(pst::Effect::Permit),
        Some("forbid") => Ok(pst::Effect::Forbid),
        other => Err(format!("bad effect: {other:?}")),
    }
}

fn build_principal(a: &Arenas, id: StructId) -> Result<pst::PrincipalConstraint, String> {
    let it = list_items(a, id);
    match a.as_name(*it.get(1).ok_or("principal body")?) {
        Some("any") => Ok(pst::PrincipalConstraint::Any),
        Some("eq") => Ok(pst::PrincipalConstraint::Eq(build_eos(
            a,
            *it.get(2).ok_or("eq eos")?,
        )?)),
        Some("in") => Ok(pst::PrincipalConstraint::In(build_eos(
            a,
            *it.get(2).ok_or("in eos")?,
        )?)),
        Some("is") => Ok(pst::PrincipalConstraint::Is(build_entity_type(
            &str_leaf(a, *it.get(2).ok_or("is ty")?).ok_or("is ty str")?,
        )?)),
        Some("is-in") => Ok(pst::PrincipalConstraint::IsIn(
            build_entity_type(&str_leaf(a, *it.get(2).ok_or("is-in ty")?).ok_or("is-in ty str")?)?,
            build_eos(a, *it.get(3).ok_or("is-in eos")?)?,
        )),
        other => Err(format!("bad principal: {other:?}")),
    }
}

fn build_resource(a: &Arenas, id: StructId) -> Result<pst::ResourceConstraint, String> {
    let it = list_items(a, id);
    match a.as_name(*it.get(1).ok_or("resource body")?) {
        Some("any") => Ok(pst::ResourceConstraint::Any),
        Some("eq") => Ok(pst::ResourceConstraint::Eq(build_eos(
            a,
            *it.get(2).ok_or("eq eos")?,
        )?)),
        Some("in") => Ok(pst::ResourceConstraint::In(build_eos(
            a,
            *it.get(2).ok_or("in eos")?,
        )?)),
        Some("is") => Ok(pst::ResourceConstraint::Is(build_entity_type(
            &str_leaf(a, *it.get(2).ok_or("is ty")?).ok_or("is ty str")?,
        )?)),
        Some("is-in") => Ok(pst::ResourceConstraint::IsIn(
            build_entity_type(&str_leaf(a, *it.get(2).ok_or("is-in ty")?).ok_or("is-in ty str")?)?,
            build_eos(a, *it.get(3).ok_or("is-in eos")?)?,
        )),
        other => Err(format!("bad resource: {other:?}")),
    }
}

fn build_action(a: &Arenas, id: StructId) -> Result<pst::ActionConstraint, String> {
    let it = list_items(a, id);
    match a.as_name(*it.get(1).ok_or("action body")?) {
        Some("any") => Ok(pst::ActionConstraint::Any),
        Some("eq") => Ok(pst::ActionConstraint::Eq(build_entity_uid(
            a,
            *it.get(2).ok_or("action eq entity")?,
        )?)),
        Some("in") => {
            let mut uids = Vec::new();
            for &e in &it[2.min(it.len())..] {
                uids.push(build_entity_uid(a, e)?);
            }
            Ok(pst::ActionConstraint::In(uids))
        }
        other => Err(format!("bad action: {other:?}")),
    }
}

fn build_eos(a: &Arenas, id: StructId) -> Result<pst::EntityOrSlot, String> {
    match a.head_name(id) {
        Some("entity") => Ok(pst::EntityOrSlot::Entity(build_entity_uid(a, id)?)),
        Some("slot") => Ok(pst::EntityOrSlot::Slot(build_slot(a, id)?)),
        other => Err(format!("bad entity-or-slot: {other:?}")),
    }
}

fn build_entity_uid(a: &Arenas, id: StructId) -> Result<pst::EntityUID, String> {
    let it = list_items(a, id);
    let ty = str_leaf(a, *it.get(1).ok_or("entity ty")?).ok_or("entity ty str")?;
    let eid = str_leaf(a, *it.get(2).ok_or("entity eid")?).ok_or("entity eid str")?;
    Ok(pst::EntityUID {
        ty: build_entity_type(&ty)?,
        eid: pst::SmolStr::from(eid),
    })
}

/// Build an `EntityType` from its display string (`Group`, `NS::Type`) by splitting on `::`.
fn build_entity_type(s: &str) -> Result<pst::EntityType, String> {
    let mut parts: Vec<&str> = s.split("::").collect();
    let base = parts.pop().ok_or("empty entity type")?;
    let name = if parts.is_empty() {
        pst::Name::unqualified(base).map_err(|e| format!("entity type: {e}"))?
    } else {
        pst::Name::qualified(parts, base).map_err(|e| format!("entity type: {e}"))?
    };
    Ok(pst::EntityType::from_name(name))
}

fn build_slot(a: &Arenas, id: StructId) -> Result<pst::SlotId, String> {
    match a.as_name(list_items(a, id).get(1).copied().ok_or("slot which")?) {
        Some("principal") => Ok(pst::SlotId::Principal),
        Some("resource") => Ok(pst::SlotId::Resource),
        other => Err(format!("bad slot: {other:?}")),
    }
}

fn build_clause(a: &Arenas, id: StructId) -> Result<pst::Clause, String> {
    let it = list_items(a, id);
    let expr = Arc::new(build_expr(a, *it.get(1).ok_or("clause expr")?)?);
    match a.head_name(id) {
        Some("when") => Ok(pst::Clause::When(expr)),
        Some("unless") => Ok(pst::Clause::Unless(expr)),
        other => Err(format!("bad clause: {other:?}")),
    }
}

/// Reconstruct a pst `Expr` from an arena expression node.
fn build_expr(a: &Arenas, id: StructId) -> Result<pst::Expr, String> {
    let it = list_items(a, id);
    match a.head_name(id) {
        Some("lit-bool") => Ok(pst::Expr::Literal(pst::Literal::Bool(
            bool_leaf(a, *it.get(1).ok_or("bool")?).ok_or("bool leaf")?,
        ))),
        Some("lit-long") => Ok(pst::Expr::Literal(pst::Literal::Long(
            int_leaf(a, *it.get(1).ok_or("long")?).ok_or("long leaf")?,
        ))),
        Some("lit-string") => Ok(pst::Expr::Literal(pst::Literal::String(
            pst::SmolStr::from(str_leaf(a, *it.get(1).ok_or("string")?).ok_or("string leaf")?),
        ))),
        Some("lit-entity") => Ok(pst::Expr::Literal(pst::Literal::EntityUID(
            build_entity_uid(a, id)?,
        ))),
        Some("var") => Ok(pst::Expr::Var(build_var(a, id)?)),
        Some("slot") => Ok(pst::Expr::Slot(build_slot(a, id)?)),
        Some("unary") => Ok(pst::Expr::UnaryOp {
            op: unary_op_from(
                a.as_name(*it.get(1).ok_or("unary op")?)
                    .ok_or("unary op name")?,
            )?,
            expr: Arc::new(build_expr(a, *it.get(2).ok_or("unary operand")?)?),
        }),
        Some("binary") => Ok(pst::Expr::BinaryOp {
            op: binary_op_from(
                a.as_name(*it.get(1).ok_or("binary op")?)
                    .ok_or("binary op name")?,
            )?,
            left: Arc::new(build_expr(a, *it.get(2).ok_or("binary left")?)?),
            right: Arc::new(build_expr(a, *it.get(3).ok_or("binary right")?)?),
        }),
        Some("get") => Ok(pst::Expr::GetAttr {
            expr: Arc::new(build_expr(a, *it.get(1).ok_or("get expr")?)?),
            attr: pst::SmolStr::from(
                str_leaf(a, *it.get(2).ok_or("get attr")?).ok_or("get attr str")?,
            ),
        }),
        Some("has") => {
            let expr = Arc::new(build_expr(a, *it.get(1).ok_or("has expr")?)?);
            let attrs: Vec<pst::SmolStr> = it[2.min(it.len())..]
                .iter()
                .filter_map(|&s| str_leaf(a, s).map(pst::SmolStr::from))
                .collect();
            let attrs = pst::NonEmpty::from_vec(attrs).ok_or("has needs >=1 attr")?;
            Ok(pst::Expr::HasAttr { expr, attrs })
        }
        Some("like") => {
            let expr = Arc::new(build_expr(a, *it.get(1).ok_or("like expr")?)?);
            let mut pattern = Vec::new();
            for &pe in &it[2.min(it.len())..] {
                pattern.push(build_pattern_elem(a, pe)?);
            }
            Ok(pst::Expr::Like { expr, pattern })
        }
        Some("is-expr") => {
            let expr = Arc::new(build_expr(a, *it.get(1).ok_or("is expr")?)?);
            let entity_type =
                build_entity_type(&str_leaf(a, *it.get(2).ok_or("is ty")?).ok_or("is ty str")?)?;
            let in_expr = match it.get(3) {
                Some(&ie) => Some(Arc::new(build_expr(a, ie)?)),
                None => None,
            };
            Ok(pst::Expr::Is {
                expr,
                entity_type,
                in_expr,
            })
        }
        Some("if") => Ok(pst::Expr::IfThenElse {
            cond: Arc::new(build_expr(a, *it.get(1).ok_or("if cond")?)?),
            then_expr: Arc::new(build_expr(a, *it.get(2).ok_or("if then")?)?),
            else_expr: Arc::new(build_expr(a, *it.get(3).ok_or("if else")?)?),
        }),
        Some("set") => {
            let mut exprs = Vec::new();
            for &e in &it[1.min(it.len())..] {
                exprs.push(Arc::new(build_expr(a, e)?));
            }
            Ok(pst::Expr::Set(exprs))
        }
        Some("record") => {
            let mut map = std::collections::BTreeMap::new();
            for &f in &it[1.min(it.len())..] {
                let fi = list_items(a, f);
                let key = str_leaf(a, *fi.get(1).ok_or("field key")?).ok_or("field key str")?;
                let value = Arc::new(build_expr(a, *fi.get(2).ok_or("field value")?)?);
                map.insert(key, value);
            }
            Ok(pst::Expr::Record(map))
        }
        Some("unknown") => Ok(pst::Expr::Unknown {
            name: pst::SmolStr::from(
                str_leaf(a, *it.get(1).ok_or("unknown name")?).ok_or("unknown name str")?,
            ),
        }),
        other => Err(format!("unsupported cedar expr node: {other:?}")),
    }
}

fn build_var(a: &Arenas, id: StructId) -> Result<pst::Var, String> {
    match a.as_name(list_items(a, id).get(1).copied().ok_or("var which")?) {
        Some("principal") => Ok(pst::Var::Principal),
        Some("action") => Ok(pst::Var::Action),
        Some("resource") => Ok(pst::Var::Resource),
        Some("context") => Ok(pst::Var::Context),
        other => Err(format!("bad var: {other:?}")),
    }
}

fn build_pattern_elem(a: &Arenas, id: StructId) -> Result<pst::PatternElem, String> {
    match a.head_name(id) {
        Some("wildcard") => Ok(pst::PatternElem::Wildcard),
        Some("char") => {
            let s = str_leaf(a, *list_items(a, id).get(1).ok_or("char")?).ok_or("char str")?;
            let c = s.chars().next().ok_or("empty char")?;
            Ok(pst::PatternElem::Char(c))
        }
        other => Err(format!("bad pattern elem: {other:?}")),
    }
}

/// Arena head name → pst `UnaryOp` (inverse of [`unary_op_name`]).
fn unary_op_from(name: &str) -> Result<pst::UnaryOp, String> {
    Ok(match name {
        "not" => pst::UnaryOp::Not,
        "neg" => pst::UnaryOp::Neg,
        "is-empty" => pst::UnaryOp::IsEmpty,
        "datetime" => pst::UnaryOp::Datetime,
        "decimal" => pst::UnaryOp::Decimal,
        "duration" => pst::UnaryOp::Duration,
        "ip" => pst::UnaryOp::Ip,
        "is-ipv4" => pst::UnaryOp::IsIPv4,
        "is-ipv6" => pst::UnaryOp::IsIPV6,
        "is-loopback" => pst::UnaryOp::IsLoopback,
        "is-multicast" => pst::UnaryOp::IsMulticast,
        "to-date" => pst::UnaryOp::ToDate,
        "to-time" => pst::UnaryOp::ToTime,
        "to-milliseconds" => pst::UnaryOp::ToMilliseconds,
        "to-seconds" => pst::UnaryOp::ToSeconds,
        "to-minutes" => pst::UnaryOp::ToMinutes,
        "to-hours" => pst::UnaryOp::ToHours,
        "to-days" => pst::UnaryOp::ToDays,
        other => return Err(format!("bad unary op: {other}")),
    })
}

/// Arena head name → pst `BinaryOp` (inverse of [`binary_op_name`]).
fn binary_op_from(name: &str) -> Result<pst::BinaryOp, String> {
    Ok(match name {
        "eq" => pst::BinaryOp::Eq,
        "noteq" => pst::BinaryOp::NotEq,
        "less" => pst::BinaryOp::Less,
        "lesseq" => pst::BinaryOp::LessEq,
        "greater" => pst::BinaryOp::Greater,
        "greatereq" => pst::BinaryOp::GreaterEq,
        "and" => pst::BinaryOp::And,
        "or" => pst::BinaryOp::Or,
        "add" => pst::BinaryOp::Add,
        "sub" => pst::BinaryOp::Sub,
        "mul" => pst::BinaryOp::Mul,
        "in" => pst::BinaryOp::In,
        "contains" => pst::BinaryOp::Contains,
        "contains-all" => pst::BinaryOp::ContainsAll,
        "contains-any" => pst::BinaryOp::ContainsAny,
        "get-tag" => pst::BinaryOp::GetTag,
        "has-tag" => pst::BinaryOp::HasTag,
        "is-in-range" => pst::BinaryOp::IsInRange,
        "offset" => pst::BinaryOp::Offset,
        "duration-since" => pst::BinaryOp::DurationSince,
        "decimal-lt" => pst::BinaryOp::DecimalLessThan,
        "decimal-le" => pst::BinaryOp::DecimalLessEq,
        "decimal-gt" => pst::BinaryOp::DecimalGreater,
        "decimal-ge" => pst::BinaryOp::DecimalGreaterEq,
        other => return Err(format!("bad binary op: {other}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The core surface contract: parse → print → parse is a fixed point (arena-idempotent). Cedar's
    /// pst drops comments/formatting, so byte identity is NOT required — but the TREE is stable.
    fn assert_idempotent(src: &str) {
        let a1 = read(src).expect("valid cedar");
        let printed = print(&a1, 100, |_, _| String::new());
        let a2 =
            read(&printed).unwrap_or_else(|e| panic!("reprint did not parse: {}\n{printed}", e.0));
        assert!(
            a1.structurally_eq(&a2),
            "not arena-idempotent\n--- src ---\n{src}\n--- printed ---\n{printed}"
        );
    }

    #[test]
    fn simplest_policy() {
        assert_idempotent("permit (principal, action, resource);");
    }

    #[test]
    fn effect_forbid() {
        assert_idempotent("forbid (principal, action, resource);");
    }

    #[test]
    fn scope_constraints() {
        assert_idempotent("permit (principal == User::\"alice\", action, resource);");
        assert_idempotent("permit (principal in Group::\"admins\", action, resource);");
        assert_idempotent("permit (principal is User, action, resource);");
        assert_idempotent("permit (principal is User in Group::\"admins\", action, resource);");
        assert_idempotent("permit (principal, action == Action::\"read\", resource);");
        assert_idempotent(
            "permit (principal, action in [Action::\"read\", Action::\"write\"], resource);",
        );
        assert_idempotent("permit (principal, action, resource in Folder::\"public\");");
    }

    #[test]
    fn template_slots() {
        assert_idempotent("permit (principal == ?principal, action, resource in ?resource);");
    }

    #[test]
    fn annotations() {
        assert_idempotent("@id(\"policy1\")\npermit (principal, action, resource);");
        assert_idempotent(
            "@advice(\"be careful\")\n@id(\"p2\")\nforbid (principal, action, resource);",
        );
    }

    #[test]
    fn when_unless_conditions() {
        assert_idempotent(
            "permit (principal, action, resource) when { resource.owner == principal };",
        );
        assert_idempotent("permit (principal, action, resource) unless { context.blocked };");
        assert_idempotent(
            "permit (principal, action, resource) when { context.mfa == true && principal.age >= 18 };",
        );
    }

    #[test]
    fn expression_kinds() {
        // has, .attr chain, in, records, sets, if-then-else, like with wildcard, method ops.
        assert_idempotent(
            "permit (principal, action, resource) when { principal has department };",
        );
        assert_idempotent(
            "permit (principal, action, resource) when { resource.tags.contains(\"x\") };",
        );
        assert_idempotent(
            "permit (principal, action, resource) when { principal in resource.owners };",
        );
        assert_idempotent(
            "permit (principal, action, resource) when { {a: 1, b: \"x\"} == context.info };",
        );
        assert_idempotent(
            "permit (principal, action, resource) when { [1, 2, 3].contains(context.n) };",
        );
        assert_idempotent(
            "permit (principal, action, resource) when { (if context.admin then 1 else 2) == 1 };",
        );
        assert_idempotent(
            "permit (principal, action, resource) when { resource.name like \"*.jpg\" };",
        );
    }

    #[test]
    fn extension_ops() {
        // decimal/ip constructors (UnaryOp) + method ops (BinaryOp).
        assert_idempotent(
            "permit (principal, action, resource) when { context.score.greaterThan(decimal(\"1.5\")) };",
        );
        assert_idempotent(
            "permit (principal, action, resource) when { ip(\"10.0.0.1\").isInRange(ip(\"10.0.0.0/24\")) };",
        );
    }

    #[test]
    fn multi_policy_set() {
        assert_idempotent(
            "permit (principal, action, resource);\nforbid (principal in Group::\"banned\", action, resource);",
        );
    }

    #[test]
    fn rewrite_reflects_in_output() {
        // Structurally flip an effect from permit to forbid in the arena; the printed policy changes.
        // This is the whole point: the arena is the rewritable representation.
        let mut a = read("permit (principal, action, resource);").unwrap();
        let mut changed = false;
        for id in (0..a.structure.len() as u32).map(StructId) {
            // find the (effect permit) node's `permit` name leaf and rename it to `forbid`
            if a.head_name(id) == Some("effect") {
                let items = list_items(&a, id);
                if let cadenza_syntax_core::ast::Struct::Atom(l) = *a.get(items[1])
                    && a.leaf(l) == &Leaf::Name("permit".into())
                {
                    a.leaves[l.0 as usize] = Leaf::Name("forbid".into());
                    changed = true;
                }
            }
        }
        assert!(changed, "found and flipped the effect name leaf");
        let printed = print(&a, 100, |_, _| String::new());
        assert!(
            printed.contains("forbid") && !printed.contains("permit"),
            "the rewrite reflects in output: {printed}"
        );
    }

    #[test]
    fn errors_are_refused() {
        for bad in [
            "permit",                                        // truncated
            "permit (principal, action);",                   // missing resource
            "permit (principal, action, resource)",          // missing semicolon
            "allow (principal, action, resource);",          // not a keyword
            "permit (principal, action, resource) when { }", // empty condition
            "garbage",
        ] {
            assert!(
                read(bad).is_err(),
                "expected a parse error for {bad:?}, got Ok"
            );
        }
    }

    #[test]
    fn cedar_to_binary_round_trips() {
        let src = "@id(\"p\")\npermit (principal in Group::\"g\", action == Action::\"read\", resource) when { resource.public == true };";
        let a1 = read(src).unwrap();
        let bin = cadenza_ast::codec::encode(&a1);
        let a2 = cadenza_ast::codec::decode(&bin).expect("decodes");
        assert!(a1.structurally_eq(&a2));
        // A cedar-policyset root never touches `ml_print`, so a stub suffices here.
        let printed = print(&a2, 100, |_, _| String::new());
        let a3 = read(&printed).unwrap();
        assert!(a1.structurally_eq(&a3));
    }

    // NOTE: `non_cedar_root_falls_back_to_comments` moved to `cadenza-syntax/tests/cedar_surface.rs` —
    // it exercises the ML-printer fallback (a non-Cedar root → `//`-comment block) which needs the ML
    // printer + the sexpr reader, neither of which this below-the-surface crate may depend on.

    #[test]
    fn span_table_is_total_and_ordered() {
        let (a, spans) =
            read_spanned("permit (principal, action, resource) when { principal.x == 1 };")
                .unwrap();
        assert_eq!(spans.len(), a.structure.len());
        for id in (0..a.structure.len() as u32).map(StructId) {
            assert!(spans.get(id).is_some(), "node {id:?} has a span");
        }
    }

    /// A tiny deterministic PRNG (SplitMix64) — reproducible generation without a dependency (mirrors
    /// the unit-test PRNGs in `codec.rs`/`lexer.rs`).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// Generate a random VALID Cedar policy from the language grammar: an optional `@id("…")` annotation,
    /// a `permit`/`forbid` effect, the three scope constraints (each a variant — bare, `==`, `in`, `is`,
    /// or an action set), and 0..=2 `when`/`unless` condition clauses. Every piece is a form Cedar's
    /// parser accepts, so the generated text always parses; the sweep asserts arena-idempotence over the
    /// COMBINATIONS (the fixed tests exercise each variant once, not their products).
    fn gen_cedar(rng: &mut Rng) -> String {
        let mut s = String::new();
        // optional annotation
        if rng.below(3) == 0 {
            s.push_str(&format!("@id(\"p{}\")\n", rng.below(1000)));
        }
        s.push_str(if rng.below(2) == 0 {
            "permit"
        } else {
            "forbid"
        });
        s.push_str(" (");
        // principal scope
        s.push_str(
            match rng.below(4) {
                0 => "principal".to_string(),
                1 => format!("principal == User::\"u{}\"", rng.below(100)),
                2 => format!("principal in Group::\"g{}\"", rng.below(100)),
                _ => "principal is User".to_string(),
            }
            .as_str(),
        );
        s.push_str(", ");
        // action scope
        s.push_str(
            match rng.below(3) {
                0 => "action".to_string(),
                1 => format!("action == Action::\"a{}\"", rng.below(100)),
                _ => "action in [Action::\"read\", Action::\"write\"]".to_string(),
            }
            .as_str(),
        );
        s.push_str(", ");
        // resource scope
        s.push_str(
            match rng.below(3) {
                0 => "resource".to_string(),
                1 => format!("resource == File::\"f{}\"", rng.below(100)),
                _ => format!("resource in Folder::\"d{}\"", rng.below(100)),
            }
            .as_str(),
        );
        s.push(')');
        // 0..=2 condition clauses
        for _ in 0..rng.below(3) {
            let kw = if rng.below(2) == 0 { "when" } else { "unless" };
            let cond = match rng.below(4) {
                0 => format!("context.n == {}", rng.below(100)),
                1 => format!("principal.age > {}", rng.below(100)),
                2 => "resource.public".to_string(),
                _ => format!("context.role == \"r{}\"", rng.below(10)),
            };
            s.push_str(&format!(" {kw} {{ {cond} }}"));
        }
        s.push(';');
        s
    }

    #[test]
    fn cedar_surface_is_idempotent_over_generated_policies() {
        // The surface contract (arena-idempotence: read(print(read(src))) == read(src)) swept over random
        // valid Cedar, complementing the hand-picked cases. The generator explores scope-variant +
        // condition-clause + annotation COMBINATIONS the fixed tests (one variant each) don't, so a
        // printer/parser asymmetry no hand-written case hits is caught. Fixed seeds → reproducible.
        let seeds: [u64; 3] = [
            0x0bad_c0de_dead_beef,
            0x5eed_1234_5678_9abc,
            0xfeed_face_cafe_babe,
        ];
        let mut total = 0usize;
        for &seed in &seeds {
            let mut rng = Rng(seed);
            for _ in 0..600 {
                assert_idempotent(&gen_cedar(&mut rng));
                total += 1;
            }
        }
        assert!(total >= 1500, "swept a meaningful space, got {total}");
    }

    #[test]
    fn a_generated_cedar_policy_survives_the_binary_codec_round_trip() {
        // The binary codec is the canonical STORED form for a Cedar policyset too, so it must faithfully
        // preserve the policy arena — `cdz convert policy.cedar --to binary` and back must be lossless.
        // `cedar_to_binary_round_trips` pins ONE hand policy; this sweeps it (the json/toml codec-sweep
        // analogue). Cedar is arena-idempotent (not byte-exact, like JSON), so: for random Cedar,
        // read → encode → decode is structurally identical, encode is deterministic (the bijection), and
        // printing the DECODED arena re-reads to the same tree. A codec/Cedar-surface mismatch on some
        // generated shape (scope variants, condition clauses, annotations, entity/action refs) would
        // silently corrupt a stored policy — the case the hand policy can't reach.
        let seeds: [u64; 3] = [
            0xceda_c0de_0bad_f00d,
            0x5eed_ace0_1234_abcd,
            0xb01d_face_dead_10ff,
        ];
        let mut total = 0usize;
        for &seed in &seeds {
            let mut rng = Rng(seed);
            for _ in 0..600 {
                let src = gen_cedar(&mut rng);
                let a1 = read(&src).expect("generated Cedar parses");
                let bin = cadenza_ast::codec::encode(&a1);
                let a2 = cadenza_ast::codec::decode(&bin)
                    .expect("a Cedar arena decodes from its own encoding");
                assert!(
                    a1.structurally_eq(&a2),
                    "Cedar arena survives the binary round-trip for:\n{src}"
                );
                // Determinism: re-encoding the decoded arena reproduces the exact bytes (the bijection).
                assert_eq!(
                    bin,
                    cadenza_ast::codec::encode(&a2),
                    "binary encode is deterministic for:\n{src}"
                );
                // The decoded arena prints back to a tree that re-reads identically (arena-idempotence
                // through the codec).
                let a3 = read(&print(&a2, 100, |_, _| String::new()))
                    .expect("decoded-then-printed Cedar re-reads");
                assert!(
                    a1.structurally_eq(&a3),
                    "Cedar survives binary → print → re-read for:\n{src}"
                );
                total += 1;
            }
        }
        assert!(total >= 1500, "swept a meaningful codec space, got {total}");
    }

    #[test]
    fn cedar_read_never_panics_on_arbitrary_input() {
        // `read` operates on UNTRUSTED policy text; it must return a diagnostic, never panic. Sweep
        // random Cedar-ish strings (keywords + structural chars) plus truncated fragments. On a
        // SUCCESSFUL read the arena must be well-formed with a TOTAL span table — see
        // `assert_cedar_read_invariants`.
        let alphabet: Vec<char> = "permitforbidactionresource(){}[];,=<>.\"@?: \n"
            .chars()
            .collect();
        let mut rng = Rng(0x1357_9bdf_2468_ace0);
        for len in 0..=40usize {
            for _ in 0..50 {
                let s: String = (0..len)
                    .map(|_| alphabet[rng.below(alphabet.len())])
                    .collect();
                assert_cedar_read_invariants(&s);
            }
        }
        for s in [
            "permit",
            "permit (",
            "permit (principal",
            "forbid (principal, action,",
            "@",
            "when {",
        ] {
            assert_cedar_read_invariants(s);
        }
    }

    /// `read` must not panic on arbitrary input, and on a SUCCESSFUL read the arena is well-formed with
    /// a TOTAL span table: `read`/`read_spanned` agree structurally, the arena is non-empty with root in
    /// range, `spans` is exactly 1:1 with the structure vector, and every reachable child id is in range.
    /// A clean `ReadError` on malformed input is fine. Mirrors the ML/s-expr/markdown/json/toml fuzzes.
    fn assert_cedar_read_invariants(src: &str) {
        let plain = read(src); // must not panic
        let Ok((a, spans)) = read_spanned(src) else {
            assert!(plain.is_err(), "read_spanned Err but read Ok for {src:?}");
            return;
        };
        assert!(
            plain.is_ok_and(|p| p.structurally_eq(&a)),
            "read and read_spanned disagree for {src:?}"
        );
        let n = a.structure.len();
        assert!(
            n > 0 && (a.root.0 as usize) < n,
            "root id in range for {src:?}"
        );
        assert_eq!(spans.len(), n, "span table is total for {src:?}");
        // Every span is a GEOMETRICALLY VALID slice of the source — ordered, in-bounds, on UTF-8 char
        // boundaries — even on malformed input. Totality only says a span EXISTS per node; this says
        // `&src[sp.start..sp.end]` (an LSP hover / diagnostic underline / span-based edit) can be taken
        // WITHOUT panicking. The reader synthesizes spans for structural nodes (policies/conditions), so
        // an off-by-one or a span past a truncated source is a real risk on the error path.
        for id in (0..n as u32).map(StructId) {
            let sp = spans.get(id).expect("total span table");
            assert!(
                sp.start <= sp.end
                    && sp.end <= src.len()
                    && src.is_char_boundary(sp.start)
                    && src.is_char_boundary(sp.end),
                "span {sp:?} for node {id:?} is not a valid slice of {src:?}"
            );
        }
        fn walk(a: &Arenas, id: StructId) {
            if let cadenza_syntax_core::ast::Struct::List(kids) = a.get(id) {
                for &c in kids {
                    assert!(
                        (c.0 as usize) < a.structure.len(),
                        "child id {} in range",
                        c.0
                    );
                    walk(a, c);
                }
            }
        }
        walk(&a, a.root);
    }
}
