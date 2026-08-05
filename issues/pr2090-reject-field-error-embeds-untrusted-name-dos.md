# PR #2090 review — cdz-kernel/src/ast_marshal.rs (v-agent-harness) — OPEN — security/DoS [VERIFIED, MED] (consequence of my #2078)

https://github.com/camshaft/cadenza/pull/2090 (ast_to_val record EXACT-shape — reject unknown/duplicate
fields; the fix-forward for MY #2078). Copilot (id 3715444234) flags that the NEW reject-field errors
embed the untrusted field name — a bounded-hint DoS, reintroduced by the very reject path I recommended.

## the reject-unknown / reject-duplicate errors format `{name:?}` / `{extra:?}` of the UNTRUSTED field name into `TypeMismatch.found` (documented as a BOUNDED hint) → a huge attacker field name blows up the error allocation (Copilot, ast_marshal.rs:275 & :285) — security/DoS [VERIFIED, MED]
> The TypeMismatch message includes the full untrusted field name via `{name:?}`. Since `ast_to_val`
> parses attacker-controlled bytes and `MarshalError::TypeMismatch.found` is documented as a bounded hint,
> a very large field name could force large allocations in the error path (DoS risk) and violates the
> stated invariant. Consider truncating/sanitizing the name before formatting it into the error string.

VERIFIED in the #2090 diff: the new arms do `type_mismatch("record", format!("duplicate field {name:?}"))`
(diff:24) and `format!("unknown field {extra:?}")` (diff:49) — `name`/`extra` are field names from the
UNTRUSTED AST (attacker-supplied bytes). `TypeMismatch.found` is doc'd as a bounded hint, so embedding an
unbounded untrusted name violates that + is a DoS on the error path (a multi-MB field name → multi-MB error
string). This is the SAME untrusted-input-in-error class as the #2050 `{val:?}` finding — and note the
provenance: MY #2078 recommended "reject unknown/duplicate fields", which #2090 implemented by putting the
untrusted name in the message. So closing the silent-extra-field gap opened a bounded-hint DoS. (Owning the
chain: the reject is correct; the error just needs to not dump the raw name.) MED. Fix per Copilot:
truncate/sanitize the name before formatting — e.g. a length-capped repr (`&name[..name.len().min(64)]` +
an ellipsis) or report just "unknown field (len N)" / the count — bounded, matching the `TypeMismatch.found`
contract. (Same fix shape as the #2050 val_shape bounding.) PR OPEN → foldable pre-merge. v-agent-harness
owns cdz-kernel/src.
