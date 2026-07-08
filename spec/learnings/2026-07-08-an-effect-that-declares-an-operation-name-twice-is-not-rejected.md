# An effect that declares an operation name twice is not rejected

*2026-07-08*

**What happened.** Adversarial probing of effect declarations found that an effect declaring the same
operation name twice is silently accepted. `(effect E (op f (-> Int64 Int64)) (op f (-> Int64 Int64)))`
is accepted and the program runs. The different-signature form is accepted too — `(effect E (op f (->
Int64 Int64)) (op f (-> Bool Bool)))` — leaving `E.f` with two conflicting declared types. The
legitimate cross-effect case is correctly collision-free (`Unify.resolve` and `Scope.resolve` are
distinct operations of distinct effects), so the gap is specifically a duplicate name WITHIN one effect.

**Why it is a break.** capabilities-and-effects.md #An Effect Declaration Names The Effect And Types Its
Operations: an effect declaration "binds each of its operations to an operation type, so that the set of
operations an effect offers is a CLOSED, statically-known SET rather than an open collection of ad-hoc
names." Two `(op f …)` in one effect bind the name `f` twice, so the set is not well-defined — which
operation type governs a performance of `E.f`? This is the same ill-formedness a record with a duplicate
field (`(record (a 1) (a 2))` → CDZ0201, "names the field `a` more than once") and a module with a
duplicate definition (`(module … (def (f) 1) (def (f) 2))` → CDZ0201, the c41 fix) are rejected for: a
fixed/closed set cannot name the same member twice. The effect MUST be rejected, not resolved by keeping
one `f` and silently discarding the other.

**Root cause (likely) — the effect-declaration elaboration registers operations without a
duplicate-name check.** The pass that reads `(effect E (op … ) (op … ))` and builds the effect's
operation table inserts each `(op name type)` without checking whether `name` is already bound in that
effect, so a second `f` overwrites or is appended after the first and one is silently chosen. The record
path and (as of c41) the module path already reject a duplicate member; the effect-operation-set path
does not. The fix is to check the operation names of one effect for duplicates as the effect table is
built — reusing the same duplicate-member rejection, since an effect's operations are a closed set
exactly as a record's fields and a module's definitions are.

**The lesson (the recurring family).** The duplicate-member check is proven on a record literal's fields
and (c41) on a module's definitions, but not carried to the third member-set of the same kind — an
effect's operations, which the spec explicitly calls "a closed, statically-known set." This is the "a
check proven on one form is not carried to its sibling" family; here the siblings are the three closed
name-sets the language has (record fields, module definitions, effect operations), and the
duplicate-member rejection landed for two of the three. The tell: `(record (a 1) (a 2))` and `(module …
(def (f) 1) (def (f) 2))` are rejected, but `(effect E (op f …) (op f …))` — the same duplicate in the
same kind of set — is accepted.

**Corpus case added.** `spec/semantics/14-effects-and-handlers.sexp` §"an effect that declares an
operation name twice is rejected" — `(effect E (op f (-> Int64 Int64)) (op f (-> Int64 Int64)))` MUST
reject CDZ0201, the effect-declaration companion of the record-field and module-definition duplicate
cases. Gated `(needs effects)`, which the seed realizes, so the behavior gate runs and catches it
(expected reject CDZ0201, observed a running component). A generation that does not yet detect a
duplicate operation name declines rather than silently choosing one.
