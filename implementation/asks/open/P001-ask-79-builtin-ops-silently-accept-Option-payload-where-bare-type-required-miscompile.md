## 79. 🔴 (seed) SOUNDNESS HOLE — built-in operations SILENTLY ACCEPT an `Option<T>` argument where a bare `T` is required, and MISCOMPILE (no type error). User ctors reject the same mismatch; built-ins don't.

**This is a real type-soundness hole, minimally + standalone reproducible.** A built-in operation whose
parameter type is `Bytes` (or `String`, etc.) accepts an argument of type `Option<Bytes>` **without any
type error** and produces a WRONG value — the exact "decline, don't miscompile" violation. It bit cdzc:
`Bytes.slice` returns `Option<Bytes>` (fallible), and feeding that straight into `String.from-bytes` (which
wants `Bytes`) compiled fine but decoded to an EMPTY string, silently corrupting every decoded symbol name
(the module head `"module"` compared unequal to the literal `"module"`, so the front-end mis-resolved every
program). We only caught it because the *value* was wrong, not because the compiler flagged it.

### Minimal standalone repros (all on the current stable seed)

```
; (1) String.from-bytes given Option<Bytes> — COMPILES, returns Some "" (WRONG — should be a type error,
;     or if it somehow decoded, "ABC"). Bytes.slice(_,0,3) is the 3 bytes "ABC" wrapped in Some.
(module m (def (main)
  (match (String.from-bytes (Bytes.slice (Bytes.of (list 0x41 0x42 0x43)) 0 3))
    ((Some s) (if (= s "ABC") 100 (if (= s "") 200 300)))
    ((None _) 0))))
;  → 200   (i.e. Some "" — an empty string; the 3 real bytes are dropped)

; (2) Bytes.len given Option<Bytes> — COMPILES, returns 0 (treats the Option as an empty Bytes)
(module m (def (main) (Bytes.len (Bytes.slice (Bytes.of (list 0x41 0x42 0x43)) 0 3))))
;  → 0    (should be a type error; the slice's 3 bytes are invisible)
```

### The tell: user constructors DO reject this; built-in ops DON'T

```
; (3) a USER unary constructor with a declared Bytes payload, given the same Option<Bytes> — CORRECTLY REJECTED
(module m (type W (Mk Bytes))
  (def (main) (match (Mk (Bytes.slice (Bytes.of (list 0x41)) 0 1)) ((W.Mk b) (Bytes.len b)))))
;  → decline: "a unary variant applied to a payload of the wrong type" (CDZ0201)
```

So the seed's argument type-check (`arg_contradicts_declared_type` / `annotation_contradicts`, codegen.rs
~3464) fires for USER sum constructors but is **not applied to built-in operations** (`String.from-bytes`,
`Bytes.len`, and presumably every `Bytes.*`/`String.*` op with a declared arg type). A built-in op's
argument must be type-checked against its declared parameter type by the SAME rule — an `Option<Bytes>`
where `Bytes` is declared is CDZ0201, not a silent coercion.

### Why it miscompiles rather than trapping

The `Option<Bytes>` is a heap value (a sum with a `Bytes` payload); the built-in op appears to treat the
Option's handle AS a Bytes handle (reading the sum's header region as byte content → an empty/garbage
Bytes). No trap, no diagnostic — a wrong value. This is the most dangerous failure class (the project's
central discipline is "a construct with no sound lowering is a compile error, never wrong bytes").

### Impact / how it surfaced

cdzc's `decode` read every prelude symbol via `(String.from-bytes (Bytes.slice b off len))`. Because the
`Option<Bytes>` was silently accepted and decoded to `""`, `name-head-is xs "module"` was always false →
`find-main-body` never found `(def (main) …)` → the whole program resolved to `HError`. The fix in cdzc was
to unwrap the Option (`(match (Bytes.slice …) ((Some sub) (String.from-bytes sub)) …)`) — the CORRECT idiom
— which then exposed a SEPARATE context-dependent CDZ0201 gap (filed as ask-80). Had the seed rejected the
mismatch (this ask), the bug would have been a compile error at the first build, not a silent
mis-resolution discovered only by running the output.

### Ask

Apply the built-in operation argument type-check: an argument whose type is `Option<T>` (or any type) that
contradicts the op's declared parameter type is CDZ0201, uniformly with the user-constructor check that
already exists. At minimum `String.from-bytes : Bytes → Option<String>` and `Bytes.len : Bytes → Int64`
must reject a non-`Bytes` argument. Ideally audit every built-in `Bytes.*`/`String.*`/… op arg-type.

### ⚡ SAME FAMILY as your active c82/c83 work — likely the same fix site

This is the **built-in-operation-argument** sibling of the match-typing holes you're closing this cycle:
- **c82** (closed): a wrong-type payload constructor in match-scrutinee position skipped ordinary checking
  → fixed by descending into the scrutinee (`[[match-scrutinee-checked-as-ordinary-expression]]`).
- **c83** (open): a runtime-match bare-binder arm reinterprets the payload as the other arm's type.
- **this ask (c-?):** a built-in OP applied to an `Option<T>` where `T` is declared skips the arg
  type-check and reinterprets the Option handle as a `T` handle (→ `String.from-bytes` reads an empty
  string, `Bytes.len` reads 0). Every one is "a runtime-typing path that skips the ordinary type check";
  the built-in-op call path needs the SAME argument-vs-declared-type check `arg_contradicts_declared_type`
  already runs for user constructors (codegen.rs ~3449–3472). Worth checking whether the c82/c83 match-type
  fix and this share a single choke point (ordinary-expression type-checking of a call's arguments).

**Impact on cdzc RIGHT NOW:** on seed `f544412f` (your latest, which tightened match typing via c82) cdzc's
string-decode path regressed — `(String.from-bytes (Bytes.slice …))` name-resolution flipped from correct
to empty-string again, so the front-end mis-resolves `main`. cdzc's own code was FIXED to unwrap the Option
(the correct idiom), but until the seed rejects the mismatch at the built-in-op boundary the cdzc frontier
stays entangled with these match-typing fixes. The scalar arithmetic BACKEND is unaffected (15/15 oracle).

**Priority.** 🔴 soundness — a silent miscompile from a type mismatch the compiler should catch. Related:
c82/c83 (the active match-typing fixes — same "skips ordinary type check" family); the CDZ0201 user-ctor
check (codegen.rs ~3449–3472, the correct behavior to mirror onto built-in-op arguments);
decline-don't-miscompile discipline.
