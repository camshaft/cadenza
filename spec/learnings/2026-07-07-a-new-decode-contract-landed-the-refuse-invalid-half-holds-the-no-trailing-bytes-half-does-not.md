# A decode over external bytes must be total (a Result), not trap — "refuse" is the error case, not a failure

*2026-07-07*

**What happened.** A sibling added a normative contract to `deterministic-value-form.md`: the canonical byte
form has an **inverting decode** (decode(encode(v)) ≡ v), decode **refuses** bytes that are not a canonical value
of the expected type, and **trailing bytes are a detected error** (valid bytes + extra bytes must not decode as
the prefix's value). The seed has `Ast.decode : Bytes → Ast`. Probing each clause against the running seed, then
a course-correction from the operator, produced the real shape of the requirement.

First probe (and my initial mis-reading): the seed's `Ast.decode` **traps** on invalid bytes (`(Ast.decode
(Bytes.of (list 255 255 255)))` → decline/trap) and **silently ignores** trailing bytes (`(Ast.decode
(encode(Ast.Int 7) ++ [99]))` → `Ast.Int 7`). I first recorded the trapping-on-garbage as *correct* ("refuse =
trap") and only the trailing-bytes leak as a gap. The operator corrected the design: **`Ast.decode` takes bytes
that can come from an EXTERNAL source, so it must be TOTAL — return a `Result`/`Option`, never trap.** "Refuse"
in the contract means *the error case of a fallible decode*, not a hard failure.

That correction re-frames both clauses as unmet:

- **Invalid bytes:** the seed TRAPS. Wrong — it should return the error case (`None`/`Err`), because a program
  handed untrusted bytes must be able to handle "not a valid AST" as an ordinary value, not die.
- **Trailing bytes:** the seed silently drops them and returns the prefix value. Wrong — it should return the
  error case (trailing bytes are malformed input, detected, not ignored).
- **Bijection on valid bytes:** holds (modulo the signature change — the success case wraps the value).

So the whole `Ast.decode` surface needs to become total, and that is a signature decision (Bytes → Option<Ast>,
mirroring `String.from-bytes`; or Bytes → Result<Ast, error> for diagnostics) that ripples to the 9 existing
round-trip corpus cases — an operator call, filed as ask-38.

**Why.** Two lessons, and the second is the one I got wrong first.

*A decoder over UNTRUSTED input must be total.* The dividing question for "trap vs error value" is *where the
bytes come from*. Arithmetic overflow traps because it is a program's own bug (a defined-outcome partial op). A
decode of external bytes is not a bug in the decoding program — malformed input is an expected, handleable
condition — so it must be a value the program branches on, never a trap that kills it. `String.from-bytes :
Bytes → Option<String>` already encodes exactly this for UTF-8; `Ast.decode` is the same shape and should have
the same fallible surface. I missed this because I pattern-matched "refuse = reject-don't-miscompile = trap"
from the compiler-internal setting, where a trap IS the honest decline; but decode is not compiler-internal, it
is a *library operation on data*, and data operations on untrusted input return Results. **Reject-don't-miscompile
(trap on a construct you can't compile) and total-decode (return an error value on data you can't parse) are
different disciplines for different layers — do not import the compiler's trap-is-honest reflex into a data
decoder.**

*"Refuse" in a spec is ambiguous between "trap" and "error value" — resolve it by the trust boundary, not the
word.* The contract said "refused," and I read it as trap; the operator read it as the error case of a Result.
The word alone doesn't decide; the fact that decode consumes external input does. When a spec says a decode
"refuses" bad input, the default should be the total (Result) reading unless the input is provably trusted.

**The requirement it drove.** No corpus case landed this cycle (the trap-asserting case I briefly added was
reverted — it would have enshrined the wrong behavior). The output is ask-38, rewritten to the total-decode
design: `Ast.decode` must return a Result/Option so invalid bytes AND trailing bytes yield an error VALUE (not a
trap, not a silent drop), with the signature choice (Option vs Result) flagged as an operator decision because it
ripples to the existing round-trip cases. The corpus cases that pin the error behavior are WITHHELD until the
signature is decided — adding them in the wrong signature would enshrine it — then they land as ordinary VALUE
cases (match the `None`/`Err` arm), no trap oracle involved. General lesson: **the trust boundary decides
trap-vs-Result — a partial operation on a program's own values may trap (a defined outcome), but a decode of
bytes that may arrive from outside must be total; "refuse" at that boundary means return the error case, and I
should not have read the compiler's honest-trap reflex onto a data decoder.**
