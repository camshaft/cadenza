# PR#883 review comment — do_local_binds ix==0 early-return can short-circuit the re-parent fallback (⚠ correctness, v-inference)

Mirrored from GitHub PR#883 review comment (Copilot), id `3669316742`.
File: `implementation/seed/crates/rcdzc/src/resolve.rs:2088` — rcdzc `resolve` = v-inference's owned
surface (infer/unify/resolve). Blame `60d2fce20` "resolve: recover the do-scope window when `from` was
re-parented — F2 bin-build-operand-under-handler false-unbound fix".

⚠ CORRECTNESS class (could reintroduce a false-unbound CDZ0101) — flagged for v-inference's judgment.

## Comment (verbatim)

- (id 3669316742, resolve.rs:2088) "`do_local_binds` still returns `None` early when `child_ix_of(from)
  == 0`, but `Db::child_ix_of` returns 0 both for the real `do` head *and* when a node has no recorded
  position (or is re-parented). In the re-parenting scenario this fix is meant to address, that `ix == 0`
  early return can prevent the identity-based fallback scan from running, reintroducing the false-unbound
  behavior."

## Liaison verification (confirmed on trunk f85b2c320)

- `Db::child_ix_of` (db.rs:3225-3227): `self.child_ix.get(id.0).copied().unwrap_or(0) as usize` — returns
  **0** for the real `do` head AND for any node with no recorded child-index (unrecorded / re-parented).
  The two cases are indistinguishable at this call.
- `do_local_binds` (resolve.rs:2085-2088): `let ix = db.child_ix_of(from); if ix == 0 { return None; }`
  with the comment "`from` is the `do` head itself, not a do-form (defensive)".
- The whole point of the `60d2fce20` fix (the `else` branch at ~2094, `forms.iter().position(|f| *f ==
  from)?`) is to recover the TRUE window by IDENTITY when `from` was RE-PARENTED (its live `child_ix`
  reads against a different parent). But a re-parented `from` whose recorded child-index is absent →
  `child_ix_of` returns 0 → the `if ix == 0` early-return fires FIRST and returns `None`, so the
  identity fallback never runs → the F2 false-unbound (bin-build-operand-under-handler) can reappear for
  that sub-case.

So the guard meant to catch only "`from` IS the do head" also catches "re-parented `from` with no
recorded index" and defeats the fix in that scenario. The fix (owner's judgment): before the early
return, or instead of it, fall through to the identity scan when `from != form`'s head — e.g. only
early-return `None` if `from` is genuinely the do's child-0 head (`forms.get(0)`/`as_form` head identity
check), otherwise let `forms.iter().position(|f| *f == from)` decide (it already returns `None` if `from`
isn't a real form). Whether this sub-case is currently REACHABLE (does any re-parent produce a `from`
with an unrecorded child_ix?) is v-inference's call — a witness would be a re-parented do-item reference
whose `child_ix` slot is 0/unset.

Owner: **v-inference** (rcdzc `resolve.rs` do-scope resolution; their `60d2fce20` F2 fix). Correctness —
please confirm reachable-vs-latent with a witness, don't just reword.
