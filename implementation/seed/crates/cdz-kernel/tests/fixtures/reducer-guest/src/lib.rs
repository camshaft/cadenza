//! A minimal wasm-component reducer FIXTURE (§19b/§21c interim, concierge Option A) — a wit-bindgen Rust
//! guest implementing the `cadenza:agent-kernel` reducer world. Its whole job is to prove the component
//! path end-to-end: the kernel loads this as a component, calls `fold.apply`, and gets effect-requests
//! back — exercising `ComponentReducer::apply` against a REAL guest (not just compiling it).
//!
//! B2 BINARY-AST fold boundary (design-binary-ast-abi): `fold.apply` is `apply(list<u8>) -> list<u8>`.
//! The wire is the canonical `cadenza-ast` VALUE-FORM (the shape a real rcdzc reducer's `value-decode`/
//! `value-encode` speak, B3). The event arrives as the value-form of the event record:
//!   `(record (= content_type (record (= family <Str>)(= version <Int>))) (= payload <opt>)(= resumes <opt>))`
//! and the guest returns its effects as the value-form list:
//!   `(list (record (= correlation <opt>)(= kind <Str>)(= payload <opt>)(= target <Bytes>))…)`
//! where `<opt>` = `(Some <Bytes>)` | `(None unit)`, record fields in canonical SORTED-KEY order (the shared
//! protocol descriptor both sides bake — the kernel's `ast_marshal` builds/parses the same shape). It
//! declares no content-addressed component deps (the codec is a plain path-dep Rust lib), so it needs no
//! §23 dep-resolution/compose — the right first end-to-end fixture. A REAL Cadenza reducer (rcdzc→component,
//! §21c/B3, whose `value-decode`/`value-encode` wraps this same byte fold) is the eventual bar.
//!
//! Behavior (deliberately tiny + observable): on an inbound message it (a) records a counter in KV via
//! the `kv` import (proving the guest reads/writes its own KV directly, §4b) and (b) emits ONE effect-
//! request (family `http`) with a correlation token; on any other event (a result echoing that token,
//! etc.) it emits nothing. Total: never traps (§17 can't-brick) — an undecodable/unknown event returns
//! the empty effect list `(list)`.

wit_bindgen::generate!({
    world: "reducer",
    path: "../../../wit/reducer.wit",
});

use cadenza::agent_kernel::kv;
use cadenza_ast::ast::{Arenas, Builder, Leaf, StructId};
use cadenza_ast::codec;
use exports::cadenza::agent_kernel::fold::Guest;

struct Guest0;

/// Build an `Option(Bytes)` value-form node: `(Some <bytes>)` present, `(None unit)` absent (matches the
/// kernel's `opt_bytes_form` — capital-`Some`/`None`, nullary payload `unit`).
fn opt_bytes(b: &mut Builder, v: Option<&[u8]>) -> StructId {
    match v {
        Some(bytes) => {
            let head = b.name("Some");
            let val = b.atom_leaf(Leaf::Bytes(bytes.to_vec().into()));
            b.list(vec![head, val])
        }
        None => {
            let head = b.name("None");
            let unit = b.name("unit");
            b.list(vec![head, unit])
        }
    }
}

/// Build a record field `=`-ascription triple `(= <name> <value>)` (child list `[=, name, value]`).
fn field(b: &mut Builder, name: &str, value: StructId) -> StructId {
    let eq = b.name("=");
    let n = b.name(name);
    b.list(vec![eq, n, value])
}

/// The empty effect list `(list)` — the totality answer for any event the guest ignores or can't decode.
fn no_effects() -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("list");
    let root = b.list(vec![head]);
    codec::encode(&b.finish(root))
}

/// Read a record's `=`-ascription field VALUE for the named field (the value-form reader dual). Accepts the
/// `(= name value)` triple; name-keyed (order-tolerant on read).
fn record_field(a: &Arenas, record: StructId, name: &str) -> Option<StructId> {
    let kids = a.as_form(record, "record")?;
    for &f in kids {
        if let Some([n, v]) = a
            .as_form(f, "=")
            .and_then(|p| <&[StructId; 2]>::try_from(p).ok())
        {
            if a.as_name(*n) == Some(name) {
                return Some(*v);
            }
        }
    }
    None
}

impl Guest for Guest0 {
    fn apply(event: Vec<u8>) -> Vec<u8> {
        // Decode the event document; an undecodable event is answered with the empty effect list (§17).
        let Some(a) = codec::decode(&event) else {
            return no_effects();
        };
        // Read the event record: content_type is a nested record with a `family` Str field; resumes is an opt.
        let family: Option<String> = record_field(&a, a.root, "content_type")
            .and_then(|ct| record_field(&a, ct, "family"))
            .and_then(|fam| a.as_str(fam).map(|s| s.to_string()));
        // `resumes = (Some …)` means this is a result/timer event the guest resumes on.
        let has_resumes = record_field(&a, a.root, "resumes")
            .map(|r| a.head_name(r) == Some("Some"))
            .unwrap_or(false);

        // A result/timer event (resumes = (Some token)) — the effect it correlates to completed. This tiny
        // fixture just stops (no cascade); a real reducer would look the token up + continue.
        if has_resumes {
            return no_effects();
        }
        // Inbound message → bump a KV counter (direct kv import, §4b) and request one effect (family http).
        if family.as_deref() == Some("message") {
            let prev = kv::get(b"count")
                .and_then(|v| v.first().copied())
                .unwrap_or(0);
            kv::put(b"count", &[prev.wrapping_add(1)]);
            // (list (record (= correlation (Some b"step-1"))(= kind "http")(= payload (None unit))(= target b"…")))
            // fields in canonical SORTED-KEY order: correlation, kind, payload, target.
            let mut b = Builder::new();
            let corr_v = opt_bytes(&mut b, Some(b"step-1"));
            let corr = field(&mut b, "correlation", corr_v);
            let kind_v = b.atom_leaf(Leaf::Str("http".into()));
            let kind = field(&mut b, "kind", kind_v);
            let pay_v = opt_bytes(&mut b, None);
            let payload = field(&mut b, "payload", pay_v);
            let tgt_v = b.atom_leaf(Leaf::Bytes(b"https://ok.host/x".to_vec().into()));
            let target = field(&mut b, "target", tgt_v);
            let rhead = b.name("record");
            let req = b.list(vec![rhead, corr, kind, payload, target]);
            let lhead = b.name("list");
            let root = b.list(vec![lhead, req]);
            return codec::encode(&b.finish(root));
        }
        no_effects()
    }
}

export!(Guest0);
