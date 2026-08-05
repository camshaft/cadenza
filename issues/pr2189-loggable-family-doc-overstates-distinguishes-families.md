# PR #2189 review — cdz-kernel/src/kernel.rs (v-agent-harness) — OPEN — doc-accuracy [VERIFIED, LOW] (the fix for MY #2180 family-leak)

https://github.com/camshaft/cadenza/pull/2189 (#2180 residual — redact guest-controlled family from
tracing, MED, completes #2169; THE fix for MY #2180 family-leak residual). Copilot 1 inline on the doc.

## the `loggable_family` doc says the diagnostic "still distinguishes families without exposing content", but it returns a constant `<extension>` for ALL extension families + logs `family_len` — length doesn't distinguish (different extension families can share a length) → overstates the discriminating power (Copilot, kernel.rs:1419) — doc-accuracy [VERIFIED, LOW]
> The doc comment implies that logging `family_len` meaningfully "distinguishes families", but length
> alone is not generally distinguishing (different extension families can share a length). Reword to
> avoid implying uniqueness while keeping the intent (provide some diagnostic signal without leaking
> guest-controlled bytes).

VERIFIED in the #2189 diff: the redaction is CORRECT — `family = loggable_family(&req.content_type.family)`
+ `family_len = req.content_type.family.len()` (diff:83-84, 102-103), where `loggable_family` (diff:120)
returns the well-known `&'static` name for known families else the constant `"<extension>"`. So for an
extension family it logs `"<extension>"` + a length — NO guest bytes (the MED leak my #2180 flagged is
FIXED, the important part). But the doc (diff:117) says "diagnostic still distinguishes families without
exposing content" — which overstates: every extension family logs as the SAME `"<extension>"` string, and
`family_len` alone can't distinguish them (two different extension families of equal length are
indistinguishable in the log). LOW/doc-accuracy — the redaction is sound; only the "distinguishes
families" claim implies more discriminating power than `<extension>`+len provides. Fix per Copilot: reword
to the honest intent — "provides a diagnostic signal (well-known family name, or `<extension>`+length for
register-by-string families) WITHOUT leaking guest-controlled bytes" — drop the "distinguishes families"
implication (it distinguishes well-known from extension, and gives a length, but not one extension family
from another). v-agent-harness owns cdz-kernel/src. PR OPEN → foldable. (Owning the chain: my #2180 family-
leak → this fix correctly redacts; the doc just slightly oversells what the redacted form conveys. The
security fix itself is right.)
