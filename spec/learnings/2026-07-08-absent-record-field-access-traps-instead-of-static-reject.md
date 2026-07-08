# Accessing a field a record does not have traps at runtime instead of a static type error

*2026-07-08*

**What happened.** Member access of a field the record's type does not carry — `p.z` where `p` is
`(record (x 1))` — is lowered to code that **traps at run time**, instead of being **rejected at
compile time**. `(let ((p (record (x 1)))) p.z)`, `(. (record (x 1)) z)`, `(. (record (a 1) (b 2)) c)`,
and `(+ (. (record (x 1)) z) 10)` all reach a wasm trap. A valid field access on the same record
(`(. (record (x 1)) x)` = 1) works, so the defect is specific to the absent-field case.

**Why it is a break.** A record's TYPE is its field names with their types (type-system.md #The
Structural Types Are Record, Tuple, And Sum: "a record's field names with their types"). Member access
projects "the field named by its key FROM the record" (core-semantics.md #Member Access Projects A
Record Field). A field the record's type does not carry cannot be projected — the projection has no
defined result — exactly as projecting a field of a NON-record has no defined result. The corpus
already fixes the non-record case as a compile-time CDZ0201 rather than a trap: `(. 5 x)`, `(. true
x)`, `(. (tuple 1 2) f)`, `(. "hi" x)` all reject ("member access on a non-record") "rather than emit
a component that traps" (05-compound-types.sexp). And the row operations fix the absent-field outcome
UNCONDITIONALLY: type-system.md #A Record Is Restricted To A Named Set Of Its Fields and #A Record Is
Reduced By Dropping A Named Set Of Its Fields both state that naming "a field the operand record does
not contain MUST be rejected at compile time with the machine-readable code for a required field that
is absent." Bare member access is the same projection and must reject the same way. Emitting trapping
code is the exact anti-pattern the non-record cases were written to forbid, applied to the sibling
operand shape.

**Root cause (the master pattern — a check proven on one operand shape not carried to its sibling).**
The type checker rejects `.field` when the OPERAND is not a record (it has a "member access requires a
record operand" gate that fires for Int/Bool/Tuple/String → CDZ0201). But when the operand IS a
record, it does not additionally check that the record's field set CONTAINS the named field — it lowers
the access to a field-slot read that traps at run time when the field is absent. The "projection with
no defined result ⇒ static reject, never a trapping component" rule was proven for the non-record
operand and never carried to the record-operand-missing-the-field case. The tell: `(. 5 x)` (non-record
operand) correctly declines CDZ0201, but `(. (record (x 1)) z)` (record operand, absent field) traps —
same "no field to project" situation, opposite outcome.

**The lesson (a projection's well-formedness is BOTH operand-is-a-record AND the-field-exists).** A
field access is well-typed only when the operand is a record *and* that record's type has the named
field; either failure has no defined result and must be a static rejection, not a runtime trap. The
seed checks the first half and lowers the second half to trapping code. Same shape as the row-op rules
(project/drop of an absent field → static reject) and the non-record member-access cases — all three
are "you cannot project a field that isn't there," and the bare-access record-operand corner is the one
that still traps. Ties to the fixed-field-set discipline: a record's fields are a fixed compile-time
set (the map-key vs record-field-label distinction), so field presence is a static property the type
checker already knows and must enforce at the access site.

**Fix direction (gitignored seed).** At the member-access type-check, after establishing the operand
is a record, look up the accessed field in the operand's record type (its field-name-to-type set) and
reject CDZ0201 (the required-field-absent code) when the field is not present — before lowering. This
is the same field-set membership check the row project/drop operations already perform; member access
must run it too. Regression guards: a valid field access still succeeds (`(. (record (x 1)) x)` = 1,
nested `(. (. p inner) v)` = 42); the non-record member-access cases still reject CDZ0201; the row
project/drop absent-field cases still reject.

**Corpus case added.** `spec/semantics/05-compound-types.sexp` §"member access of a field the record
does not have is a type error" — `(. (record (x 1)) z)` MUST be CDZ0201. Placed right after the
non-record member-access cases it is the sibling of. Realized (ungated — member access is realized),
the behavior gate catches it (expected CDZ0201, observed a runtime trap).

Related: the non-record member-access cases (05-compound-types.sexp, already correct); type-system.md
#A Record Is Restricted To A Named Set Of Its Fields (the row-projection absent-field rule this mirrors);
core-semantics.md #Member Access Projects A Record Field; the fixed-field-set / record-field-label
discipline. Master-pattern family: a well-formedness check proven on one variation (non-record operand)
must carry to every sibling (record operand missing the field).
