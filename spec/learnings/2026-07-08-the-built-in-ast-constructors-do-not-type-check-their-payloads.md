# The built-in Ast constructors do not type-check their payloads

*2026-07-08*

**What happened.** After the c51 fix (a user sum's unary variant now checks its payload type), adversarial
re-probing found the fix does not reach the built-in `Ast` constructors nor tuple-typed payloads.
`(Ast.Int "x")` — a String where `Ast.Int`'s payload is Int64 — is accepted and constructs `(Ast.Int
"x")`; `(Ast.Name 42)` (Int where String) is accepted too. The mistyped payload is usable: matching
`(Ast.Int "x")` binds the String, and `(String.byte-len n)` reads it as a String and succeeds (running
the ill-typed program); `(Ast.Name 42)` matched and used as a String declines "String.byte-len of a
non-String value" (proving 42 was bound where a String was declared). The user-sum scalar case is now
correctly rejected (`(T.Mk "x")` for `(type T (Mk Int64))` → "a unary variant applied to a payload of the
wrong type"), and List payloads too — but the built-in `Ast` constructors (a distinct codepath) and
Tuple-typed payloads are still unchecked.

**Why it is a break.** type-system.md #The Abstract Syntax Tree Type Is An Ordinary Sum Type: the Ast is
"an ordinary sum type of the language — a variant per syntactic form (an integer, a float, a string, a
boolean, a name, and a list of child nodes)". So `Ast.Int` carries an integer (Int64), `Ast.Name` a
string. A constructor is a single-arity function whose argument is type-checked (core-semantics.md #A Sum
Type Constructor Is A Single-Arity Function + #Applying A Function Binds Its Parameter To Its Argument),
so `(Ast.Int "x")` is a type mismatch, CDZ0201, exactly as `(T.Mk "x")` for a user sum is. Building
`(Ast.Int "x")` is a false accept — and a self-hosted front end that constructs AST nodes with `Ast.*`
could emit a malformed node.

**Root cause (likely) — the c51 payload-type check reaches user `(type …)` sum variants but not the
prelude-declared `Ast` constructors (nor Tuple payloads).** The c51 fix added the payload-type check on
the user-sum-declaration path; the built-in `Ast` constructors are bound by the prelude through a
different path that the check doesn't cover, and the check appears to handle scalar and List payload
shapes but not Tuple. The fix is to route every constructor — user-declared and built-in alike — through
the same payload-type check, covering all payload type shapes (scalar, String, List, Tuple, record, sum),
since the `Ast` constructors are ordinary sum constructors the spec types identically.

**The lesson (the recurring family).** A check landed for one path (user sum variants, scalar/List
payloads) but not the sibling paths (built-in Ast constructors; Tuple payloads) — the same "a check
proven on one form is not carried to its sibling" shape as the effect-typing matrix (scalar checked,
String/compound not, until generalized) and the c51 user-sum case itself. The built-in `Ast` is "an
ordinary sum type" per the spec, so its constructors must be checked by the identical mechanism a user
sum's are — a compiler that special-cases the built-in path leaves it a hole. The tell: `(T.Mk "x")` is
rejected but `(Ast.Int "x")` — the same wrong-payload construction on a built-in sum the spec calls
ordinary — is accepted.

**Corpus case added.** `spec/semantics/12-metaprogramming.sexp` §"a built-in Ast constructor applied to a
wrong-type payload is a type error" — `(Ast.Int "x")` MUST reject CDZ0201, the built-in-Ast companion of
the user-sum unary-variant payload-type case (05-compound-types.sexp). Native seed (Ast is realized); the
behavior gate catches it (expected reject CDZ0201, observed a running component constructing `(Ast.Int
"x")`). A generation that does not yet check the Ast constructors' payload types declines rather than
building the mistyped node. (The Tuple-payload user-sum gap — `(T.Pair 5)` / `(T.Pair (tuple 1 2 3))` for
`(Pair (Tuple Int64 Int64))` — is the same root on the user-sum side, left to be closed by the same
generalization rather than pinned separately.)
