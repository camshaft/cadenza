# Decision — Symbol Interning

**The decision.** The surface and semantics of Cadenza's interned-name value — a `Symbol`: a value
that wraps a name and promises constant-time equality — over the `String` value form the language
already realizes. The constitution and the collections spec fix that a `String` is a sequence of
Unicode scalar values with normalized-contents equality (spec/semantics/13-strings.sexp;
collections-and-text.md); they do not fix a value whose equality is a constant-time identity
comparison rather than an O(N) scan over contents. That value — how a program obtains one, how it
compares, and how it crosses back to text — is the choice this decision pins.

**Why the language wants it.** A self-hosting compiler keys its symbol table on names: identifier
resolution, node-kind dispatch, and scope lookup all compare names, and a name comparison over raw
strings is O(N) in the name's length and reallocates or rescans on every probe. Interning maps equal
names to one shared identity, so a comparison becomes a handle compare — the single highest-leverage
representation win on the self-hosting path, because name comparison is the compiler's hot inner loop.
"Cloning a name around" is already O(1) under the shared reference-counted heap
(memory-and-resource-model.md #Sharing Is Not Observable); interning closes the remaining gap by making
*comparison* O(1) too.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- A Symbol's identity — hence its equality and its canonical byte form — MUST be a deterministic
  function of its content, never of the order in which symbols were interned. An allocation-order id
  (the classic `0,1,2…` interning trick) is FORBIDDEN: it depends on evaluation order and would leak
  into observable behavior, violating deterministic-value-form.md #A Value Has One Canonical Byte Form
  and core-semantics.md #Equality Is Structural (equality agrees with the canonical byte form).
- Interning is an unobservable representation optimization, not a new distinction between values: a
  Symbol built from a computed string and one built from a literal of the same content MUST be one
  value, indistinguishable by every operation (memory-and-resource-model.md #Sharing Is Not
  Observable).
- Two Symbols MUST be equal exactly when their underlying strings are equal, so Symbol equality
  inherits String's normalized-contents equality (collections-and-text.md #String Equality Follows
  Normalized Contents) lifted through the Symbol tag.
- A Symbol is a value with a canonical byte form and a boundary representation. Adding it is an
  ADDITIVE change: it defines a canonical/boundary form for a value that previously had none, permitted
  by deterministic-value-form.md #Additive Evolution and component-abi.md #Additive Evolution without a
  contract version increment.
- The intern table is retained storage accounted for what its representation actually holds live
  (memory-and-resource-model.md #Retained Storage Is What A Value's Representation Holds Live); bounding
  how much a program may allocate is a concern of the environment that runs it, not a language
  requirement (constitution Principle V, retired by Amendment 0.7.0).

**Why this is an isolated decision.** A `Symbol` is a nominal value over the existing `String` form
(type-system.md #User Types Are Declarable As Nominal Or Structural): a structural `String` carrying an
orthogonal tag. Its equality reuses String equality; its nominal boundary reuses the existing `CDZ0202`
rejection (a Symbol is not comparable to the untagged String of its shape, exactly as a nominal record
is not comparable to the plain record of its shape); it needs no new diagnostic code and no new trap.
The interning that makes equality constant-time is a runtime representation concern behind the opaque
value-heap-runtime handle (component-abi.md #A Runtime Value Crosses As An Opaque Handle, #The Runtime
Owns The Value Heap And Its Representation), already licensed as an invisible optimization. So the form
touches no frozen contract and no capability requirement: it is a new value form realized by a later
generation, not the seed (`options/realized-capability-set/`). Until then its corpus cases carry
`(needs symbols)` and the seed's behavior gate skips them.

## Choices

- [`interned-string-symbol`](./interned-string-symbol.md) — a `Symbol` value wrapping an interned
  `String`, obtained with `Symbol.of`, recovered with `Symbol.to-string`, compared by content in
  constant time, equality-only, with a `#"<text>"` reader literal. **The default.**

DEFAULT: interned-string-symbol
