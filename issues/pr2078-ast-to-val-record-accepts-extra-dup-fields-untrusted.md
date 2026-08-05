# PR #2078 review — cdz-kernel/src/ast_marshal.rs (v-agent-harness) — MERGED — input-validation (MED) + 1 LOW doc

https://github.com/camshaft/cadenza/pull/2078 (ast_marshal ast_to_val — the AST→Val dual). Copilot 2 inline:
a security-adjacent lax-record-decode + a test-harness doc mismatch.

## `ast_to_val` record decode checks declared fields EXIST but silently accepts EXTRA + DUPLICATE fields from untrusted AST (Copilot, ast_marshal.rs:264) — input-validation [VERIFIED, MED]
> `ast_to_val` record decoding currently only checks that each declared field exists, but it silently
> ignores *extra* fields in the AST and will also accept duplicate field entries (the first match wins).
> Because arg bytes are untrusted, this can hide malformed inputs and produce surprising/incorrect values;
> it's safer to reject unknown or duplicate fields and require an exact match to the WIT record shape.

VERIFIED on trunk: the `Type::Record` arm (ast_marshal.rs ~258) iterates the DECLARED (WIT) fields, looks
each up in the AST, and errors only on `missing field`. It never checks for AST fields BEYOND the declared
set, nor rejects DUPLICATE entries (a lookup takes the first match). Contrast the `Type::Tuple` arm right
below (:266) which STRICTLY checks arity (`elems.len() != types.len()` → error). Since `ast_to_val` decodes
UNTRUSTED arg bytes (guest/caller AST → Val for a host call), silently accepting extra/duplicate fields
hides malformed input + can yield a surprising Val (same untrusted-input-hardening spirit as the #2050
{val:?} DoS). MED. Fix per Copilot: require an EXACT match to the WIT record shape — reject unknown fields
(AST field not in the declared set) and duplicate field names, mirroring the tuple arm's strictness.

## test-harness `param_type` doc says it reads a probe fn PARAMETER via `Func::params`, but it reads the RESULT type (Copilot, ast_marshal.rs:555 & :569) — doc-accuracy [VERIFIED, LOW]
> The test-harness comment above `param_type` describes extracting a `Type` from a probe function
> *parameter* via `Func::params`, but the helper actually reads the probe function's *result* type (a
> `(list <wanted>)` wrapper). This mismatch makes the harness harder to understand/maintain.
LOW/doc — reword the comment to "reads the probe fn's RESULT type (a `(list <wanted>)` wrapper)" to match
the code. v-agent-harness owns cdz-kernel/src. The record-decode one is the finding that matters.
