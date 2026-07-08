# A module with two definitions of the same name is not rejected

*2026-07-08*

**What happened.** Adversarial probing of module composition found that a module with two definitions
of the same name is silently accepted, keeping the first and discarding the second. `(module m (def (f)
1) (def (f) 2) (def (main) (f)))` runs to `1` — the second `(def (f) 2)` is dropped, and the collision
is resolved by an implicit first-wins precedence. Duplicate `main` is likewise accepted (first wins).
The record-literal analogue IS caught: `(record (a 1) (a 2))` is rejected "record names the field `a`
more than once" (CDZ0201).

**Why it is a break.** core-semantics.md #A Module Evaluates To A Record Of Its Exports: "Each
definition MUST register its name and value as a field of the module's record." #A Record Has A Fixed
Set Of Named Fields: a record associates "a fixed SET of statically-known field names each with a
value." So two definitions of `f` register the field `f` twice — the exact ill-formedness the record
literal `(record (a 1) (a 2))` is rejected for (CDZ0201). A module with two `(def (f) …)` is therefore
ill-typed and MUST be rejected, not resolved by an implicit precedence — the same principle
modules-and-namespaces.md #Importing states for imports ("Importing two definitions under the same name
into one scope MUST be a compile-time error rather than resolved by an implicit precedence"), here for
two definitions written in one module. Silently keeping the first (so `(f)` = 1) is the implicit
first-wins precedence the fixed field set forbids.

**Root cause (likely) — the duplicate-field check is applied to record literals but not to a module's
definition list.** The record-construction path checks that field names are distinct (rejecting `(record
(a 1) (a 2))`), but the module-elaboration path that registers each `(def name …)` as a field of the
module's export record inserts the definitions into a map/list without checking for a name already
registered, so the second `f` overwrites (or is appended after) the first and one is silently chosen.
The fix is to check the module's definition names for duplicates as it builds the export record —
reusing the same duplicate-field rejection the record literal already applies — since a module IS a
record of its exports.

**The lesson (the recurring family).** A well-formedness check proven on one construct (a record
literal's field set) is not carried to its sibling (a module's definition set), even though the spec
says a module IS a record of its definitions-as-fields. This is the same "a check proven on one form is
not carried to its sibling" shape as the annotation-descent (tuple/list/sum vs record), the
if-branch-vs-connective scope check, and the nominal record-vs-sum boundary — here the siblings are a
record literal and a module's export record, which the spec explicitly identifies as the same kind of
value. The tell: `(record (a 1) (a 2))` is rejected but `(module … (def (f) 1) (def (f) 2) …)` — the
same duplicate field, built by definitions instead of a literal — silently keeps one.

**Corpus case added.** `spec/semantics/11-modules.sexp` §"a module with two definitions of the same name
is rejected" — `(module m (def (f) 1) (def (f) 2) (def (main) (f)))` MUST reject CDZ0201, the module-
definition companion of the record-literal duplicate-field case. Native seed; the behavior gate catches
it (expected reject CDZ0201, observed a running component returning `1`). A generation that does not yet
detect a duplicate definition declines rather than silently choosing one.
