# An unbound name in a map key is coerced to a string instead of a scope error

*2026-07-08*

**What happened.** A map's key is a VALUE resolved in scope, but the seed silently coerces an UNBOUND
name in a `(map (k v) …)` key position to a String literal of its spelling instead of raising the
unbound-name error. `(map (undefined-key 1))` yields `(map ("undefined-key" 1))` — a wrong value that
fabricates a String key the program never wrote. A BOUND name in the same position correctly resolves to
its value: `(let ((k 42)) (map (k 1)))` is `(map (42 1))`, equal to `(Map.insert Map.empty 42 1)`. So the
key reader DOES scope-resolve a bound name (right), but falls back to stringifying an unbound one (wrong).
The identical unbound name in the map VALUE position, or in any ordinary expression, correctly declines
"unbound name" (CDZ0101).

**Why it is a break.** collections-and-text.md #A Map's Canonical Form: "a map's keys are VALUES of one
key type; a record's field names are fixed compile-time labels." A map key is an ordinary expression
evaluated and resolved in scope — that is how a map has a dynamic key at all. core-semantics.md #Binding
Is Lexical makes a reference to a name with no enclosing binding an unconditional compile-time error
(CDZ0101). So an unbound name in a key position MUST be that scope error, exactly as in the value
position. Coercing it to a String is a wrong value (a fabricated key) AND it makes a dynamic key
impossible to reason about — there is no way to intend the String `"undefined-key"` by writing a bare
unbound name, and a name the author expected to be bound silently becomes a string typo rather than being
caught.

**Root cause (the unquote-fallback family).** The map-key reader tries to resolve the key name and, on
failure, falls back to treating it as a String literal — the same shape as the c29 unquote bug ("a
fallback keyed on 'eval_const returned no value' collapses 'fine-but-not-const' and 'broken' into one
path and picks the wrong behavior for the broken one"). A position whose semantics is "evaluate this as a
value" must not reinterpret an unresolvable name under different semantics (a String label); a name that
fails to resolve is the name's scope error. The fix is to resolve a map key as an ordinary scoped value
expression — a bound name to its value, an unbound name to CDZ0101 — never a String fallback.

**Entangled defect — no readable literal for a non-name key.** The reader also rejects a String-literal
key and an integer-literal key: `(map ("a" 1))` and `(map (1 10))` both decline "a map entry is not a
(key value) pair." So the unbound-name→String coercion is currently the ONLY way the `(map …)` literal
expresses a String key — which is why the corpus's own map cases (`(map (a 1))`, `(map (a 1) (b 2))`,
used with `a`/`b` unbound throughout 05-compound-types.sexp) lean on it and pass. Both are one defect: the
key position is not read as an ordinary value expression. Fixing it (unbound → CDZ0101) will require those
corpus cases to use bound names or a real String-key literal `(map ("a" 1))` — so the key-literal reader
must ALSO accept literal keys (string, integer) for the map literal to remain expressible. (This is why
my earlier cycle-67 note mis-framed the issue as an unpinned "display form isn't a readable literal"
spec-gap: the real break is the wrong-value coercion of an unbound key, which the value-position contrast
makes unambiguous — a map key is a scoped value, so an unbound one is CDZ0101.)

**Corpus case added.** `spec/semantics/05-compound-types.sexp` §"an unbound name in a map key is a scope
error, not a coerced string" — `(map (undefined-key 1))` MUST reject CDZ0101, with the doc pinning that a
bound key resolves to its value. Native seed (`(needs collections)`, realized); the behavior gate catches
it (expected reject CDZ0101, observed a running component coercing the key to `"undefined-key"`). A
generation that does not yet evaluate a map key as a scoped value declines rather than coercing.
