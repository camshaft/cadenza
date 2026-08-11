//! A minimal wasm-component reducer FIXTURE (§19b/§21c interim, concierge Option A) — a wit-bindgen Rust
//! guest implementing the `cadenza:agent-kernel` reducer world under the BYTES fold boundary
//! (DESIGN-binary-ast-abi §3a). Its whole job is to prove the component path end-to-end: the kernel loads
//! this as a component, calls `fold.apply(event: list<u8>) -> list<u8>`, and gets an effect-list AST back —
//! exercising `ComponentReducer::apply` against a REAL guest (not just compiling it).
//!
//! BYTES BOUNDARY (§3a), CANONICAL value-form: `apply` takes ONE `list<u8>` — the Event as the deterministic
//! `cadenza-ast` value-form `value-encode` produces / `value-decode` consumes (record-type Phase B), which is
//! ALSO exactly what the kernel's `ast_marshal::build_event_document` emits and `parse_effect_list` reads:
//!   Event         = `(record (= content-type (record (= family <str>) (= version <int>))) (= payload <opt>)
//!                    (= resumes <opt>))`   — NAME head `record`, fields as `(= name value)`, sorted by name.
//!   <opt>         = `(Some <bytes>)` | `(None unit)`   — CAPITAL ctor; a nullary `None`'s payload is the
//!                    `unit` atom (value-decode's Sum arm needs exactly two children).
//! and returns ONE `list<u8>` effect-list document:
//!   effect-list   = `(list <effect-request>…)`   — NAME head `list` (empty = `(list)`).
//!   effect-request= `(record (= correlation <opt>) (= kind <str>) (= payload <opt>) (= target <bytes>))`
//!                    — a BARE canonical record (NOT a nominal ctor); `kind` is the effect FAMILY STRING (a
//!                    `<str>` leaf, register-by-string — never a closed enum); `target` is opaque bytes.
//! A Rust guest speaks `cadenza-ast` DIRECTLY (decodes/encodes the documents itself — no value-heap runtime,
//! no handle ABI); it needs `cadenza-ast` and nothing else (§3d "would actually work with a rust guest").
//!
//! Behavior (deliberately tiny + observable): on an inbound `message` it (a) bumps a counter in KV via the
//! `kv` import (proving the guest reads/writes its own KV directly, §4b) and (b) emits ONE effect-request
//! whose `kind` is the family STRING `http` with a `correlation` token; on any other event (a result echoing
//! that token via `resumes`, or a family it doesn't handle) it emits an empty effect list. Total: never
//! traps (§17 can't-brick) — an undecodable or unexpected event yields `(list)`.

wit_bindgen::generate!({
    world: "reducer",
    path: "../../../wit/reducer.wit",
});

use cadenza::agent_kernel::kv;
use cadenza_ast::ast::{Arenas, Builder, Leaf, StructId};
use cadenza_ast::codec;
use exports::cadenza::agent_kernel::fold::Guest;

struct Guest0;

/// Look a canonical record field up by name: `fields` are the children of a `(record …)`, each a
/// `(= name value)` 3-list; return the `value` node of the field whose name matches (order-independent,
/// mirroring how the kernel's `read_canonical_record` keys on the field name).
fn field(a: &Arenas, fields: &[StructId], name: &str) -> Option<StructId> {
    fields.iter().copied().find_map(|f| {
        let kv = a.as_form(f, "=")?;
        (kv.len() == 2 && a.as_name(kv[0]) == Some(name)).then(|| kv[1])
    })
}

/// Build a canonical `(= name value)` record field; `value` builds the value node.
fn eq_field(b: &mut Builder, name: &str, value: impl FnOnce(&mut Builder) -> StructId) -> StructId {
    let eq = b.name("=");
    let n = b.name(name);
    let v = value(b);
    b.list(vec![eq, n, v])
}

/// The empty effect-list document `(list)` — the no-effects fold result.
fn empty_effects() -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("list");
    let root = b.list(vec![head]);
    codec::encode(&b.finish(root))
}

impl Guest for Guest0 {
    fn apply(event: Vec<u8>) -> Vec<u8> {
        // TOTALITY (§17): an undecodable / unexpected-shape event yields an empty effect list, never a trap.
        let Some(a) = codec::decode(&event) else {
            return empty_effects();
        };
        // Canonical Event: NAME-head `(record (= content-type …) (= payload …) (= resumes …))`.
        let Some(fields) = a.as_form(a.root, "record") else {
            return empty_effects();
        };
        // A result/timer event carries `resumes = (Some <token>)` — the effect it correlates to completed;
        // this tiny fixture just stops (a real reducer would resume). Absent = `(None unit)`.
        let resumes_some =
            field(&a, fields, "resumes").and_then(|r| a.head_name(r)) == Some("Some");
        if resumes_some {
            return empty_effects();
        }
        // content-type family: `(= content-type (record (= family <str>) …))`.
        let family = field(&a, fields, "content-type")
            .and_then(|ct| a.as_form(ct, "record"))
            .and_then(|ct_fields| field(&a, ct_fields, "family"))
            .and_then(|f| a.as_str(f));
        if family != Some("message") {
            return empty_effects();
        }
        // Inbound message → bump a KV counter (direct kv import, §4b) and request one effect.
        let prev = kv::get(b"count")
            .and_then(|v| v.first().copied())
            .unwrap_or(0);
        kv::put(b"count", &[prev.wrapping_add(1)]);
        // Emit `(list (record (= correlation (Some <bytes>)) (= kind http) (= payload (None unit))
        // (= target <bytes>)))` — one effect-request in the canonical value-form the kernel's
        // `parse_effect_list` reads (fields keyed by name; emitted sorted to match `value-encode`). `kind`
        // rides as a family STRING leaf (register-by-string, §3a) → `new_with_family` kernel-side.
        let mut b = Builder::new();
        let req = {
            let rec = b.name("record");
            let correlation = eq_field(&mut b, "correlation", |b| {
                let some = b.name("Some");
                let v = b.atom_leaf(Leaf::Bytes(b"step-1".to_vec().into()));
                b.list(vec![some, v])
            });
            let kind = eq_field(&mut b, "kind", |b| b.atom_leaf(Leaf::Str("http".into())));
            let payload = eq_field(&mut b, "payload", |b| {
                let none = b.name("None");
                let unit = b.name("unit");
                b.list(vec![none, unit])
            });
            let target = eq_field(&mut b, "target", |b| {
                b.atom_leaf(Leaf::Bytes(b"https://ok.host/x".to_vec().into()))
            });
            b.list(vec![rec, correlation, kind, payload, target])
        };
        let head = b.name("list");
        let root = b.list(vec![head, req]);
        codec::encode(&b.finish(root))
    }
}

export!(Guest0);
