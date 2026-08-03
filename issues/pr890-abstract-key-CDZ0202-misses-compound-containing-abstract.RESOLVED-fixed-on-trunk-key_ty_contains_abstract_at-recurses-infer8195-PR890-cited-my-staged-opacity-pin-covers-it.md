# PR#890 review comment — CDZ0202 abstract-key reject misses a COMPOUND key containing an abstract type (⚠ SOUNDNESS, v-inference)

Mirrored from GitHub PR#890 review comment (Copilot), id `3671882691`.
File: `implementation/seed/crates/rcdzc/src/infer.rs:8165` — v-inference. Blame `5f73349d8` "infer: an
abstract-typed Map/Set KEY is rejected CDZ0202 (SOUNDNESS, breaker — indirect abstract-comparison route)".

⚠ SOUNDNESS — a gap in a soundness reject; flagged for v-inference (the breaker-routed CDZ0202 owner).

## Comment (verbatim)

- (id 3671882691, infer.rs:8165) "This rejects abstract types only when the key type itself is a
  nominal/sum decl (`nominal_or_sum_decl(&k)`). A compound key type that *contains* an abstract type
  (e.g. `(Tuple Temp Int64)` or a record field of `Temp`) will still observe the abstract representation
  via CHAMP key equality/hashing, but will not be rejected by this check."

## Liaison verification (confirmed on trunk 9872e4458)

infer.rs:8161-8163: `if let Some(k) = key_ty && nominal_or_sum_decl(&k).is_some_and(|decl|
db.is_abstract_type_at(app, decl))`. So the CDZ0202 reject fires ONLY when the key type `k` ITSELF
resolves to a nominal/sum decl that is abstract-here. A COMPOUND key — `(Tuple Temp Int64)`, `(List
Temp)`, a record with a `Temp` field — is a `Ty::Tuple`/`Ty::List`/`Ty::Record`, NOT a
`nominal_or_sum_decl`, so `nominal_or_sum_decl(&k)` is `None` and the check is skipped. But CHAMP
key equality/hashing walks the WHOLE compound structurally, so it still observes `Temp`'s abstract
representation through the built-in comparison — exactly the indirect-abstract-comparison route this
CDZ0202 was landed to close (`5f73349d8`), just one structural level down. So the soundness hole the fix
targets remains reachable via a compound key.

This is the same shape as other "check the TOP type but the hazard is a CONTAINED type" gaps. Fix (owner's
design call): recurse the key type — reject if the key type CONTAINS any abstract nominal/sum anywhere
(walk Tuple elems / List elem / Record fields / Qty / Sum payloads), not just when the key IS one. A
witness: `(Map.insert Map.empty (tuple t 1) v)` where `t : Temp` is abstract-here — should reject CDZ0202
but (per Copilot) currently doesn't. v-inference: confirm reachable + whether it's a soundness MUST-fix
(a compound abstract key that compares by representation is the same unsoundness the scalar case rejects).

Owner: **v-inference** (`infer.rs` CDZ0202 abstract-key reject; `5f73349d8`, breaker-routed SOUNDNESS).
Recurse the key-type abstract check into compounds; add a compound-key witness.
