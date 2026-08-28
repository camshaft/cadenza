# DESIGN — Native First-Class Compound-Data AST Tags (record / map / list / set / tuple)

> **Status:** Phase-1 design (draft for review). Owner: `v-ast-compound` (vertical
> `native-ast-compound-data`, subsystem `rcdzc`/`cadenza-ast`). Minted per operator directive
> 2026-08-27. Coordinated with `v-spec-oracle` (they own the spec pins; I lead the list/map/set
> compound-tag work).
>
> **Operator intent (verbatim):** "get native compound data structures in the cadenza-ast… a way to
> clearly say what's a record, map, list, etc. and then coordinate between all the front-end syntax
> crates to start emitting it; everything consuming the ast will need to use the new tags instead of
> the old string-matching approach. We don't have to START this now, but I want it BEFORE we start
> deploying the platform."

## 1. Problem

A compound value in the stored/resolved AST is recognized today by **comparing the head leaf's text**
to one of the reserved words `"record"` / `"tuple"` / `"list"` / `"map"`. That recognition is
open-coded at **~250 call sites** across the seed compiler, the surface crates, `cdz-platform`, and
the CLI tooling — every one an `as_ctor_form(id,"X").or_else(|| as_form(id,"X"))` or a raw
`head_ctor(id) == Some("X")` string compare. There are two problems with this:

1. **It is string-matching, not a tag.** Nothing in the node *declares* it is a record; a consumer
   must know the reserved vocabulary and re-derive the kind by string comparison. A misspelling, a
   new consumer that forgets a spelling, or a name/string leaf that happens to read `"record"` all
   fail silently or subtly. There is no single typed "what kind of compound is this" answer.
2. **It is about to spread.** The platform (`cdz-platform`) already re-reads compound heads from
   decoded arenas by string-matching (`contract_value.rs`, `testing/*.rs`). As the platform is built
   out and deployed, this string-matching multiplies across a new, harder-to-change surface. The
   operator's deadline framing — *before* platform deployment — is precisely to stop that spread.

The goal: **the AST node itself carries a first-class tag that says "I am a record / map / list / set
/ tuple," recognized once at the decode/parse boundary and dispatched on as a typed value
everywhere downstream — never re-derived by string comparison.**

## 2. Current state (verified on `origin/main` cfd967783)

### 2.1 Two head spellings, keyed to shadowability
A compound is a `List` node whose first child is an atom leaf:
- **`("record" …)` — a `Str` leaf head** = the *unshadowable primitive* (a string literal cannot be
  introduced by a name binding). Emitted by the **value reader path** and by all internal compiler
  synthesis that must not be captured by a rebound name.
- **`(record …)` — a `Name` leaf head** = the *shadowable prelude alias* `record`/`tuple`/`list`/`map`,
  an ordinary prelude name subject to lexical scoping. Emitted by the **pattern reader path**, the
  Cedar PST path, value-reification, and the corpus (which authors name-headed forms predominantly:
  `(tuple …)`≈4446, `(record …)`≈1436, `(list …)`≈4454, `(map …)`≈269 vs string-headed ≈329/≈215).

Children conventions: a **tuple**'s children are its elements in positional order; a **record**'s
children are `(= <key> <value>)` field pairs; a **list**'s children are its elements; a **map**'s
children are `(<key> <value>)` (or `(= k v)`) entry pairs.

### 2.2 What the spec already pins
- **`spec/contracts/ast-encoding.md` (FROZEN):** every node is *a symbol applied to an ordered
  sequence of children*; a node **names its kind by referencing a namespaced (optionally versioned)
  symbol in the file's own prelude, by index** (§"The Symbol Prelude"); **adding a new node kind is a
  new symbol, with no container-encoding version bump** (§"New Constructs Do Not Bump The Encoding
  Version"). The encoding is a **bijection** — one canonical byte form per tree — which
  content-addressing and hashing depend on. **⚠ This means the ideal end-state (recognize a node kind
  by a namespaced prelude symbol referenced by index, not by leaf text) is *already normative and
  frozen*.** The concrete codec does not yet implement a namespaced prelude — the leaf pool plays that
  role and the head is a `Name`/`Str` leaf recognized by *text* — so today's string-matching is an
  **impl divergence from a frozen requirement, not a spec gap.** This design conforms the impl toward
  the contract; it **must not edit `ast-encoding.md`** (a governance-floor act — escalate to the
  operator if it were ever genuinely needed). *(Confirmed with v-spec-oracle 2026-08-27.)*
- **`spec/capabilities/core-semantics.md` §"A Compound Value Has A Symbol Constructor And A Shadowable
  Alias" (§263-269):** §265 pins the surface spellings (string-headed primitive + shadowable alias);
  §269 (v-spec-oracle **#4370**) pins that the **stored head MUST be the constructor *symbol*, never a
  string-literal leaf** — a decode observes a **name-leaf head** `tuple`/`record`. **Only tuple and
  record are enumerated; list, map, and set are not covered here.**

### 2.3 The reference architecture already exists — `compiler-ml`
The self-hosted compiler (`implementation/compiler-ml/`) is exactly the target shape:
- It string-matches the head **only in its reader** (`src/sread.cdz:182-183`:
  `sym == "tuple" → read-tuple-form → Node.NTuple`, `sym == "record" → … → Node.NRecord`).
- Everywhere downstream (`parse-db.cdz`, `resolve-db.cdz`, `infer-db.cdz`, `lower-db.cdz`, `ty.cdz`)
  it dispatches on the **first-class node variants** `Node.NTuple` / `Node.NRecord` — **never on
  strings.** (It has no list/map/set compound node yet.)

The migration is, in one line: **make the seed compiler (`rcdzc`) and the `cadenza-ast` consumers look
like `compiler-ml` — recognize the compound kind once at the boundary, dispatch on a typed tag
everywhere else — and extend the treatment to list, map, and set.**

## 3. The tag scheme

The design separates cleanly into two layers. **Layer 1 is the heart of the mission and is
low-risk; Layer 2 is a stored-form decision that needs an operator ruling.**

### 3.1 Layer 1 — a typed `CompoundCtor` tag, recognized once, dispatched everywhere (RECOMMEND: do)
Introduce a single typed tag and one boundary recognizer, in **both** `Arenas` implementations
(`rcdzc::ast` and `cadenza_ast`):

```
enum CompoundCtor { Record, Tuple, List, Map, Set }
fn compound_ctor(&self, node: Id) -> Option<CompoundCtor>   // the ONE place head text is read
```

`compound_ctor` is the *only* function that inspects head-leaf text against the reserved vocabulary.
It subsumes today's scattered `head_ctor` / `as_ctor_form(…).or_else(as_form(…))` idiom (accepting
both the `Str` primitive and the `Name` alias, exactly as the dual-accept idiom does today, so all
name-headed corpus input keeps resolving). Downstream:

- `rcdzc/src/resolve.rs:411-416` — the central value dispatch — routes `compound_ctor(id)` →
  `resolve_{record,tuple,list,map,set}` → `Resolved::{Record,Tuple,List,Map,Set}`. **`Resolved`
  already carries `Record`/`Tuple`/`List`/`Map` as first-class variants; this design adds `Set`
  (§3.3) so all five are peers.**
- Every consumer that today re-string-matches (the ~250 sites: `lower.rs`, `infer.rs`, `effects.rs`,
  `compile.rs`, the sidecars/reifiers, and the `cdz-platform` re-readers) dispatches on the typed
  `CompoundCtor` / the resolved variant instead of a raw string. Most of `lower.rs`/`infer.rs`
  already consume `Resolved::*`; the residual literal-shape predicates (`is_tuple_literal`,
  `is_list_literal`, `is_map_literal`, the `head_ctor(id) == Some("…")` diagnostics) route through
  `compound_ctor`.

This is a **consolidation**: it changes *how many times and where* the reserved words are matched
(once, at the boundary, into a typed tag), **not the wire format**. It is the change that structurally
prevents string-matching from spreading into the platform. It is safe (behavior-preserving) and can
land incrementally site-by-site behind the new helper.

### 3.2 Layer 2 — the stored representation of the tag (RESOLVED — see §3.6 / §6 D1)

> **⚠ SUPERSEDED / historical.** The operator RULED this (2026-08-27): the stored representation is a
> **distinct payloadless LEAF KIND per collection** — see **§3.6** and **§6 D1 (2d)**. The (2a)/(2b)/(2c)
> menu below is the *rationale that led there* (why not name-leaf, why not a bare `Sym`, why the
> prelude-by-index endgame is deferred), retained for context. **Do NOT implement the (2b) namespaced-`Sym`
> "recommendation" — it is superseded by the leaf-kind ruling.** Read §3.6/§6 for the decision.

"A way to clearly say what's a record *in the cadenza-ast*" is a statement about the **stored node**.
Three candidate representations were weighed (additive-evolution-ordered); the operator chose a fourth,
(2d), a distinct leaf kind (§3.6):

- **(2a) Keep the `Name`-leaf head** (status quo, as #4370 pins). The "tag" is a `Name` leaf whose
  text is a reserved word. Layer 1 already delivers typed dispatch; the wire is unchanged and #4370
  stands as-is. **Lowest risk; but the stored node is still only *distinguishable from a variable
  reference by knowing the vocabulary*** — a `Name("record")` head and a `Name("record")` reference
  are the same leaf kind.
- **(2b) Promote the head to a NAMESPACED `SYM` leaf (`KIND_SYM = 15`).** A compound head becomes a
  reserved-namespace symbol, *structurally distinct* from any user `Name` (always a reference/binder)
  or `Str` (always data). Recognition becomes "head is a reserved-namespace compound `Sym`" — an
  unspoofable **namespace** in the sense of ast-encoding §"A Prelude Symbol Is Namespaced". Three
  requirements make this correct (v-spec-oracle review, 2026-08-27):
  - **It must be NAMESPACED, not bare text on a `Sym`.** The `SYM` leaf is *today* the content-valued
    `#"…"` symbol literal (e.g. `#"meter"`), so a bare `Sym("record")` compound head could collide with
    a user's `#"record"` value symbol. A reserved namespace (i) prevents that spoof, (ii) keeps
    `node_eq`/`canon` (`cadenza-ast/ast.rs:805`/`:1401`) from collapsing a compound-tag `Sym` with a
    plain content `Sym` of the same text — protecting the bijection guard, and (iii) makes (2b) a
    **forward-compatible additive step toward (2c)** — the prelude-symbol-by-index endgame *subsumes*
    a namespaced symbol rather than re-flipping it.
  - **It touches §265 *and* §269, not just §269.** §265 says the non-shadowable primitive is "named by
    a string literal" and "the string spelling IS the reserved symbol"; under (2b) the primitive is a
    `Sym`, so the string-headed surface form (`("tuple" …)`) is redefined as **surface sugar the reader
    maps to the reserved `Sym` tag**, and §269's "decode observes a name-leaf head" becomes
    "symbol-leaf head." Both are **co-written with v-spec-oracle** (they own the tuple/record pins).
  - **Oracle impact:** the resolved-AST recognition rule v-lean-oracle relies on ("unshadowed name-leaf
    `tuple`/`record` = constructor") becomes "reserved-namespace `Sym`-leaf head"; v-spec-oracle
    re-answers the oracle the moment D1 is ruled.
  **Additive wire** (the `SYM` kind already exists in both codecs). This most directly delivers "the
  node *clearly says* what it is." **RECOMMENDED** as the native tag, framed as the forward-compatible
  first step toward (2c), if a wire touch is acceptable pre-platform.
- **(2c) Full namespaced-prelude-symbol-by-index** — implement ast-encoding's "The File Carries Its
  Own Symbol Prelude" / "A node names its kind by referencing a symbol in the prelude by index." This
  is **not an optional endgame — it is what the frozen contract already requires** (§2.2); the concrete
  codec's leaf-pool-by-text approach is the divergence. It would make *every* node kind (not just
  compounds) a first-class indexed namespaced symbol. It is a **large codec change spanning every node
  kind, well beyond this mission's scope**, and is the correct home for closing the full
  contract-vs-impl gap. **DEFER to a separately-scoped codec effort; this mission moves toward it, not
  through it.** (No `ast-encoding.md` edit — the requirement is already there.)

**Recommendation:** land Layer 1 regardless (it is the mission's core and is safe). For Layer 2,
recommend **(2b)** — the `SYM`-leaf tag — as the concrete "native compound tag in the cadenza-ast,"
coordinated with v-spec-oracle to revise §269, and explicitly **defer (2c)**. Present (2a)/(2b)/(2c)
to the operator (§6 D1).

### 3.3 `set` becomes a first-class compound (RECOMMEND; see §6 D2)
Today **`set` has no primitive head**: `#(…)` and `Set.of(...)` desugar to `((. Set of) ("list" …))`,
so there is no compound `set` node — only a `Set.of` application over a list. The operator explicitly
lists "set" among the kinds that should "clearly say what they are." **Recommend giving `set` a
first-class compound tag** (`CompoundCtor::Set`, `Resolved::Set`) symmetric with the other four, so
the reader emits a `set` head (Layer-2 representation per D1) directly instead of the `Set.of`-over-list
desugar, and the AST plainly declares a set. This is net-new (a `Resolved::Set` variant + its lowering,
mirroring `Map`). The alternative (keep `set` as `Set.of` sugar) leaves set the odd one out.

**⚠ Construction tag vs value-render form are separable (v-spec-oracle, 2026-08-27).** Giving `set` a
*construction* tag (a `set` head in the AST) is distinct from its *value output/render* form — today a
set **value** renders `(Set.of (list …sorted))` with type `(Set T)` (pinned by `19-sets.sexp` outputs
and catalogued for v-lean-oracle). D2 must **decide and pin** whether the value render *also* changes to
a `(set …)` form:
  - **Construction-tag-only (RECOMMENDED as the cheaper first step):** the reader/AST gains a `set`
    construction tag, but a set *value* still renders via `(Set.of (list …))`. The oracle value form and
    the `19-sets.sexp` outputs are **unaffected**.
  - **Also change the value render:** every `19-sets.sexp` `(output (: (Set.of (list …)) (Set T)))` case
    migrates in lockstep (a corpus flip) and v-spec-oracle re-answers the oracle.
Either way, coordinate the renderer owner **and** v-spec-oracle before flipping any set output form.

### 3.4 Nesting
Compounds nest **by construction** and need no special handling: a record field value, a list/set
element, or a map key or value is itself any AST node, recognized by *its own* head tag. A
`(record (= xs (list 1 2)) (= m (map (k v))))` is a record whose `xs` field's value node carries the
`list` tag and whose `m` field's value carries the `map` tag. Layer 1's `compound_ctor` is applied
per-node during the recursive walk exactly as the resolver already recurses; Layer 2's tag
representation is uniform at every depth. No arity or depth limits.

### 3.5 Invariants the design must preserve
- **Bijection / canonical bytes** (ast-encoding): each layer choice must keep one canonical byte form
  per tree. (2a) is trivially unchanged; (2b) must ensure the `Str`↔`Name`↔`Sym` head collapse in
  `node_eq`/`canon` (`cadenza-ast/ast.rs:805`, `:1401`) is updated so equal trees still canonicalize
  identically. The `cdzast\x00\x01` byte-stability test is the guard.
- **Shadowability semantics** (core-semantics §267): the *alias* name (`record`/`tuple`/…) stays an
  ordinary, shadowable prelude name; a program binding named `record` still shadows it and an
  application `(record a b)` in that scope applies the binding (resolves to a reference, not the
  compound tag). Only the *primitive* (the `Str` today, or `Sym` under 2b) is beyond shadowing.
  `compound_ctor` must therefore be applied to the **resolved** head, after scope resolution has
  decided whether a `Name` head is the alias or a shadowing reference — exactly where `resolve.rs`
  does it today.
- **Tuple and list are distinct tags even when their payloads coincide.** A list and a tuple can share
  an identical heap array; they are told apart by the **static type** the renderer walks (a list
  renders `(list …)`, a tuple `(tuple …)`) — see `05-compound-types.sexp`. So `CompoundCtor::Tuple` and
  `CompoundCtor::List` are always **separate constructor symbols**; the tag must never be inferred from
  payload shape. (`v-spec-oracle`, 2026-08-27.)
- **Map and set add key/element canonical ordering** (deterministic-value-form's unordered-aggregate
  rule) — their canonical byte form orders entries/elements by a member-derived key, as records order
  by field key. The tag change is orthogonal to this ordering, but the byte-stability gate covers both.

### 3.6 The tag family extends to `=` (field-pair) and `.` (member access) — RULED BY OPERATOR (2026-08-27)

The operator extended the scheme beyond the five collection constructors to the two other *pervasive
value-structural heads*, each of which is recognized by head text today and would otherwise be
string-matched everywhere:
- **`=` — the field-pair marker** `(= key value)`, used for BOTH record fields AND map entries (the
  latter unified from the legacy plain `(k v)` map entry per the operator's flat-pairs ruling, see
  D-SURFACE §6). It is a distinct node so the ML printer can attach a comment to a field; making it a
  **dedicated payloadless leaf kind** keeps that while removing
  the text dispatch. **Disambiguation win (CONFIRMED, v-spec-oracle 2026-08-27):** the field-pair `=` and
  the equality operator `(= a b)` ARE the same bare `=` leaf today, disambiguated ONLY by position (a `=`
  directly under a resolved record head = field pair; a `=` in expression/application position = equality
  — `03-equality-and-observation.sexp` vs `05-compound-types.sexp`). A dedicated `FieldPair` leaf kind
  *separates the two structurally*, removing that position-context ambiguity.
- **`.` — member access / projection** `(. obj key)`. Ubiquitous; same treatment — a dedicated leaf
  kind (`Member`/`Dot` tag), dispatched by kind, not by matching the `.` head text.

These are **not** compound-VALUE constructors (they don't join `CompoundCtor`); they are sibling reserved
structural tags. They ride the same migration mechanism (M0 recognizer → M1 dual-read → M2 leaf kind +
surface → M3 delete text dispatch), the same no-backward-compat end state, and the same leaf-kind codec
addition. The s-expr surface for `=`/`.` is TBD at the reader-design increment (they are not `#word(…)`
literals — they appear *inside* forms; e.g. a field pair inside `#record(…)`).

**D-SCOPE (open, low-stakes):** this wave = **7 tags** — `list`/`tuple`/`record`/`map`/`set` + `=` +
`.`. Whether to also promote the remaining reserved grammar heads (`if`/`let`/`match`/`:`/`fn`/`quote`/…)
to leaf tags — converging on the frozen contract's "every node names its kind by a prelude symbol"
endgame — is deferred to a follow-on pass. **Recommendation: land the seven pervasive value-structural
tags now; sweep the rest later.**

## 4. Emitters that must produce the tag

Grouped by the two current spellings. Under Layer 1 these are unchanged; under Layer 2 (2b) each is
redirected to emit the `SYM` head via a single builder (`compound_head(ctor)`), replacing today's
`push_str`/`push_atom(Leaf::Str)` / `push_name`/`Leaf::Name` at these sites.

**Str-head (unshadowable primitive) emitters:**
- `cadenza-syntax/src/parser.rs` value reader — `ctor_head(…)`: tuple :1982, list :3555, record :3616,
  map :3701, set→`((. Set of)("list"…))` :3782.
- `cadenza-ast/src/ast.rs` WIT/kernel **type descriptors** — `atom_leaf(Leaf::Str)`: list :588,
  tuple :614, record ~:628. (Mirrored in `rcdzc/src/ast.rs`.)
- `rcdzc/src/`: `prelude.rs` `ctor_record` + built-in module records (~30 `push_atom(Leaf::Str("record"))`
  sites, :454+); `eval.rs` :2949/:3002/:4039-4162 (+ the `Prim`→spelling map :2923-2925);
  `effects.rs` :268/:294/:5173/:6806/:6834; `lower.rs:15530`; `sums.rs` :281/:386/:461/:485;
  `modules.rs` :63/:129; `wit_world.rs` :455/:475/:495/:595.

**Name-head (shadowable alias) emitters:**
- `cadenza-syntax/src/parser.rs` pattern reader — `name(…)`: tuple :3342, list :3364, map :3417,
  record :3452; `cedar.rs` PST — `mk_name`: set :370, record :379.
- `rcdzc/src/`: `lower.rs` value-reification :15739-16169 + the many `push_name` list/map
  pattern-rebuilds; runtime-literal wrappers `bytes_of_runtime.rs`, `set_of_runtime.rs`, `quote.rs`,
  `tagged_template.rs`; `proptest_gen.rs` :1595/:1609/:1772/:1852.
- `cdz-runtime/src/lib.rs` value→Doc-AST reification (`name_leaf`, a *separate* Doc AST, `Name`-only):
  :2566-2773, :10262-10401 (tuple/list/record/map).

**Peripheral / out of scope:** `cdz-smith` emits *source text* not AST nodes; the Markdown doc-list AST
(`markdown.rs`) is an unrelated tree; the `SYM` leaf's existing `#"…"` symbol-literal use is untouched.

## 5. Consumers that must switch to the tag

**Central choke points (migrate first):**
1. `rcdzc/src/resolve.rs:411-416` — the `head_ctor` value-construction dispatch → `Resolved::*`.
2. The four head-reader helpers, **defined twice**: `head_name`/`head_ctor`/`as_form`/`as_ctor_form`
   at `rcdzc/ast.rs:1254/1268/1276/1306` **and** `cadenza-ast/ast.rs:687/698/706/717`; plus
   `ctor_head_key` (`cadenza-ast/ast.rs:805`) and the head-flip collapse (`:1401`) used by `node_eq`.
3. The pervasive `as_ctor_form(id,"X").or_else(|| as_form(id,"X"))` idiom — pattern matchers in
   `resolve.rs`/`lower.rs`/`compile.rs`/`infer.rs`; sidecars `accum.rs`/`param_sidecar.rs`; reifiers
   `bytes_of_runtime.rs`/`set_of_runtime.rs`.

**Per-crate:**
- **rcdzc** — the bulk. `resolve.rs` (central dispatch + all pattern matchers), `infer.rs`
  (:4278-4296, :6805-6976, :8654 literal-shape/diagnostic checks), `lower.rs` (`is_*_literal`
  helpers :10227-10263 + pattern-compilation `as_*_form` blocks), `effects.rs` (:6740-6834 constructor
  rebuild), `db.rs` (:5835/:5869/:6094-6099 type-param harvest of lowercase compound-*type* aliases),
  `eval.rs` (:825-869/:2465 record-context/binder checks), `compile.rs` (:4876-4998 pattern
  compilation), `proptest_gen.rs`, `eval_ast.rs`.
- **cadenza-syntax** — `match_to_let.rs:76-79` (tuple/record match-binder→let desugar), `query.rs:981`
  (reserved-word classification), `cedar.rs:849/856` (Cedar set/record dispatch), `parser.rs`
  type-position recognizers :2773/:2781. (`json.rs`/`toml_surface.rs` use `head_name` for their *own*
  surface heads — not the core compound ctors.)
- **cdz-platform** — re-reads decoded arenas (does **not** re-parse text): `contract_value.rs:183/367`,
  `testing/log_value.rs:945`, `testing/checker_protocol.rs:296`, `testing/spec.rs:852/860/1459`. **This
  is the surface the deadline is about — migrate it before it grows.**
- **cdz-run** — text-based (parses value literals from raw strings, not the arena):
  `lib.rs:3542/3628/3654/3790`. These match a *textual* head token, so they need the reserved
  vocabulary as data even after the arena side is tag-based — call out as a distinct sub-case.
- **cdz** `main.rs:9367`, **cdz-rust-render** `lib.rs:165/180-204` (type/value renderer),
  **cdz-cad** `lib.rs:400/420/460`, **cdz-runtime** consumers `lib.rs:3289-3429`.
- **compiler-ml** — `sread.cdz:182-183` reader; downstream already tag-based (the reference). Extend
  its node set with list/map/set variants to match (later, as its own increment).

## 6. Decisions — RULED BY OPERATOR (2026-08-27)

The operator ruled D1 and D2 directly. Verbatim: *"i think the least intrusive change is probably to
introduce a bunch of new leaf types that are for each one of the collections"*; *"yes we should have a
set symbol as well"*; *"i am not a huge fan of the string-encoded head. it's just janky."*

- **D1 — stored-tag representation: RULED = (2d) a distinct payloadless LEAF KIND per collection
  constructor** (`list`/`tuple`/`record`/`map`/`set`), used in head position of the existing
  `(head child…)` list node; recognition is a match on the **leaf kind** (a byte), never head text.
  This supersedes the earlier (2a)/(2b)/(2c) menu: it is *more* first-class than (2b)'s namespaced
  `Sym` (a distinct kind can't collide with a user `#"record"` value-symbol at all) yet stays a
  **leaf-pool addition**, not a codec-container change. The janky string-encoded head leaves the
  **stored** form entirely — replaced by the ctor leaf. It is a forward-compatible step toward (2c) the
  frozen contract's prelude-symbol-by-index (which would subsume it). **Spec cost:** co-edit §265
  *and* §269 with v-spec-oracle (the string/name-headed spellings are RETIRED, not re-homed as sugar)
  + v-spec-oracle re-answers v-lean-oracle (recognition rule = "reserved ctor-leaf-kind head").
- **D1-END-STATE — NO BACKWARD-COMPAT (operator, 2026-08-27):** *"i don't want sugar with the strings.
  i want to deprecate it and remove it."* / *"i don't want to carry any old stuff through. we should
  remove all of the old behavior by the time we're done. no backward-compat."* So the END state carries
  **zero** legacy: the string-headed `("list" …)` spelling is **removed** (not kept as sugar), name-head
  compound *recognition* is removed (a `(name …)` is always an application — the prelude constructor
  names build a ctor-leaf node like any function, they are not a recognized head form), the
  `head_ctor`/`ctor_head_key`/`Name`↔`Str` head-collapse helpers are **deleted**, and the **corpus is
  fully migrated** to the new surface. The dual-read phase (M1/M2) is *transient scaffolding only* so
  `trunk` never breaks mid-flight; **M3 deletes every trace of the old path** (§7). No permanent compat
  layer.
- **D-SURFACE — s-expr `#word(…)` prefix, FLAT PAIRS + reader-inserted `=`: RULED (operator, 2026-08-27,
  via v-syntax):** with the string head gone and a bare-name head ambiguous with application, the s-expr
  textual surface takes an explicit constructor **`#`-word prefix on the paren**, consistent with the
  existing `#"…"`/`#{…}`/`#(…)`/`#\` reader-directive family. Body grammar (operator refinement,
  superseding the earlier "grouped" proposal — *"for the record and map syntax we know what the data
  types are and should just do pairs, and then the sexpr would implicitly insert the `=` nodes"*):
  - `#list(a b c)` → `(list a b c)`, `#tuple(a b)` → `(tuple a b)`, `#set(a b c)` → `(set a b c)` — FLAT,
    elements pass through, no pairing.
  - `#record(f1 v1 f2 v2 …)` → `(record (= f1 v1) (= f2 v2) …)` — FLAT pairs; **the reader inserts an `=`
    (FieldPair) node per pair** (no explicit `(= …)` in the surface). Odd element count = reader error.
  - `#map(k1 v1 k2 v2 …)` → `(map (= k1 v1) (= k2 v2) …)` — likewise; **map entries UNIFY with record
    fields as `(= key value)`** (NOT today's plain 2-elem `(k v)` entry). Odd count = reader error.
  🔑 **Consequence (v-ast-compound lane):** the map AST-entry shape changes from `(k v)` to `(= k v)`,
  unified with record fields and consistent with the `=` FieldPair tag (§3.6). The rcdzc map
  consumer/lowering (`resolve_map` + lower) must DUAL-READ both `(= k v)` and legacy `(k v)` through the
  migration; M3 migrates the corpus `(map (k v))` → `(map (= k v))` and drops the plain form. **v-syntax
  owns the reader/printer (`cadenza-syntax/src/sexpr.rs`); this design owns the map-consumer change + the
  ctor leaf kinds; v-spec-oracle's §269 co-edit shape's "map = (key value)" becomes "(= key value)".**
  The ML surface (`[…]`/`{…}`/`#{…}`/`#(…)`/`(a,b)`) already has its own literals.
- **D2 — `set`: RULED = first-class** — `set` gets its own ctor leaf/symbol like the other four
  (`CompoundCtor::Set`, `Resolved::Set`). The set **value render** stays `(Set.of (list …sorted))` :
  `(Set T)` (operator did not ask to change it) so `19-sets.sexp` outputs + the oracle value-form are
  untouched — see §3.3; reversible if the operator later wants the render to change too.
- **D3 — scope/timing vs platform:** Layer 1 (typed dispatch consolidation) is the deadline-critical
  piece and should land before platform deployment spreads the string-matching; Layer 2's wire touch
  can follow. Confirm this staging is acceptable.

## 7. Safe migration sequence

Additive-first, dual-read, then flip, then delete — so `trunk`/`main` is never broken mid-flight. The
dual-read phase is **transient scaffolding only**; per the operator's no-backward-compat ruling
(§6 D1-END-STATE) the END state carries **zero** legacy — M3 deletes every trace and the corpus is
fully migrated.

1. **M0 — Layer 1 recognizer (additive).** Add `CompoundCtor` + `compound_ctor(id)` to both `Arenas`
   (`rcdzc`, `cadenza-ast`) and route the central resolve dispatch (`resolve.rs:444`) through it.
   Behavior-preserving. Add `Resolved::Set` + its lowering (mirrors `Map`) for D2 = first-class set.
   *(rcdzc slice landed 2026-08-27 — the enum + `compound_ctor` + central dispatch; see the log.)*
2. **M1 — route consumers through the tag (dual-read scaffold).** Migrate the choke points (§5.1-5.3)
   then the per-crate consumers to dispatch on `CompoundCtor`/`Resolved::*` instead of open-coded string
   compares. Sites keep accepting both spellings *temporarily* (via `compound_ctor`), behavior-preserving,
   landing crate-by-crate, each gated. Prioritize `cdz-platform` (the deadline surface).
3. **M2 — introduce the ctor LEAF KINDS + the `#word(…)` surface (additive), then flip producers.** Append
   one payloadless leaf kind per collection to both codecs' leaf-kind enums; add the s-expr reader/printer
   `#list(…)`/`#tuple(…)`/`#record(…)`/`#map(…)`/`#set(…)` forms; redirect every emitter (§4) to produce
   the ctor leaf; update `node_eq`/`canon` so a ctor leaf is its own identity (removing the `Name`/`Str`
   collapse) while canonical bytes stay a bijection. Reader still accepts the *old* spellings **only as a
   transient bridge** so already-stored ASTs/corpus decode during the flip. Co-write the §265/§269 edit
   with v-spec-oracle. Gate on the `cdzast\x00\x01` byte-stability test + full corpus.
4. **M3 — DELETE all legacy + migrate the corpus (no compat left).** Migrate every `spec/semantics/*.sexp`
   compound literal to the `#word(…)` form; remove the string-headed spelling entirely; delete name-head
   compound *recognition* (a `(name …)` is henceforth always an application — the prelude constructor names
   build a ctor-leaf node like any function); delete `head_ctor`/`ctor_head_key`/`as_ctor_form`/the
   `Name`↔`Str` collapse and the dual-accept idiom for compounds. End state: compounds exist **only** as
   ctor leaf kinds; string-matching is *impossible* to reintroduce.

Each step is independently green and independently landable (one coherent PR per crate/step). Dual-read
holds only through M2→M3; nothing legacy survives M3.

## 8. Gate / coverage

- Corpus (`spec/semantics/*.sexp`) cases exercising each compound kind (incl. nested) — must stay
  green through every step; add a case pinning a *nested* record-in-map-in-list if not already
  witnessed, and (if D2) `set` first-class cases.
- The `cdzast\x00\x01` byte-stability / round-trip test — the structural proof the bijection holds
  (guards M2).
- `rcdzc` + `cadenza-ast` unit tests over `compound_ctor` / `Resolved::*` dispatch.
- A guard test (or lint) that the reserved compound vocabulary is matched **only** inside
  `compound_ctor` after M3 — so a future consumer cannot re-introduce a raw string compare.

## 9. Coordination

- **v-spec-oracle** — territory split (agreed 2026-08-27): **I lead** the compound-tag *scheme* spec
  edits + impl (extending the symbol-constructor + stored-head treatment to list/map/set in
  `core-semantics.md`, and tag-identity recognition); **they own** the oracle-facing authoritative
  answers + the tuple/record clarifications already landed (#4370) and will **review this doc and any
  `core-semantics.md` edit before I land**. Neither of us edits `core-semantics.md` without looping the
  other (no double-write). §265 today names only tuple/record, but list/map/set already have their own
  capability sections and the ordering section already treats them as compounds — so extending the
  symbol-constructor treatment there is a genuine *additive* clarification. Under D1=2b it would also
  revise §269's "name-leaf head" to "symbol-leaf head" (co-written with them). **`ast-encoding.md` is a
  frozen contract — not edited here** (§2.2).
- **v-platform / v-platform-itest** are the prime beneficiaries and the deadline driver — their
  compound re-readers (§5) migrate under M1.
- **v-rust-backend / v-compiler-ml** own their emit/consumer arms (a new `Core`/`Resolved` variant —
  `Resolved::Set` — needs a Rust-backend arm per the standing rcdzc rule).

## 10. Landed migration state (2026-08-27)

Layer-1 (typed-tag recognition, representation-agnostic) is substantially LANDED on `main`:

- **The typed `CompoundCtor` tag** exists in BOTH `Arenas` (`rcdzc` + `cadenza-ast`), with str-primitive
  (`compound_ctor`), accept-both (`compound_ctor_either`), and tag+children (`compound_form_of`)
  recognizers. The central resolver dispatch, `node_eq` head-normalization, and **all 54 dual-accept
  `as_form/as_ctor_form(…).or_else(…)` sites across rcdzc** route through the tag — 0 dual-accept idioms
  remain in the crate.
- **The `(= key value)` FieldPair is consolidated** behind `Arenas::field_pair` (read) +
  `Builder::field_pair` / `Db::push_field_pair` (emit); `read_record_fields` and the record-field emit
  sites route through them.
- **Maps and records are UNIFIED on `(= key value)`** (operator ruling: prefer `=`, author it
  explicitly, verbatim reader with no phantom insertion, hard-fail on a missing `=` post-migration).
  `resolve_map` + infer's `MapNew` branch accept the canonical `(= k v)` entry (via `field_pair`) and
  dual-read legacy raw `(k v)`; corpus migrates `(map (k v))` → `(map (= k v))` at M3, then raw `(k v)`
  becomes an error. Records already emit `(= name value)`. (v-syntax owns the `#word(…)` surface —
  `#record((= x 1))` / `#map((= k v))` explicit-`=`; v-spec-oracle owns the §265/§269 pin, extended to
  map entries.)

**Still open:**
- **`#set` / first-class set (D2):** `#set(e…)` → `("set" e…)` isn't recognized yet. Design fork
  (desugar to `(Set.of (list …))` vs a compiler-level `Resolved::Set` → `Core::SetOf`, ~26-site ripple)
  surfaced to the operator. Set value render stays `(Set.of (list …sorted))`.
- **M2:** promote the reserved heads to distinct per-collection LEAF KINDS (`list`/`tuple`/`record`/
  `map`/`set` + `=` FieldPair + `.` Member) in both codecs + wire the `#word(…)` reader/printer; gated
  by the `cdzast\x00\x01` byte-stability invariant.
- **M3:** migrate the corpus to the new forms; delete the string/name-head recognition, the dual-accept
  idiom, and the head-collapse helpers; hard-fail on legacy forms. No back-compat carried through.

## 11. M2 Option-B state + value-wire reconciliation (2026-08-28) — SUPERSEDES §10 "Still open"

Updates that supersede §10's open items:

- **`#set` is LANDED first-class** (`#4652`: `("set" …)` → `Resolved::Set` → `Core::SetOf`). And the SET
  VALUE RENDER is now **`#set(…)`**, NOT `(Set.of (list …sorted))` (operator ruled 2026-08-28: "render as
  #set and not Set.of"). §10's "set render stays (Set.of …)" is SUPERSEDED. `19-sets.sexp` + any oracle
  set-render form migrate to `#set(…)` in the M3 corpus migration.
- **Reflected-Ast surface = OPTION B** (operator ruled 2026-08-28: "end-to-end native collections in the
  binary AST, no string heads for collections anywhere"). The reflected `Ast` sum gains DISTINCT first-class
  ctors — `Ast.ListCtor / TupleCtor / RecordCtor / MapCtor / SetCtor ((List Ast))` + `Ast.FieldPair /
  Ast.Member ((Tuple Ast Ast))` — so `quote`/`Ast.encode` of a compound produces the native ctor variant,
  not a string/name-headed node. The generic `Ast.List` variant is KEPT for a non-collection name-headed
  node (`(if …)`/`(fn …)`/an application). Landed in `sums.rs` (`ast_decl`) + v-spec-oracle's spec arm.

### 11.1 M2 wire (landed additively, both codecs)
Seven PAYLOADLESS ctor-head leaf kinds appended after `FLOAT_NEG_INF=19`, byte-identical in `cadenza-ast`
+ `rcdzc` codecs: `20 LIST_CTOR, 21 TUPLE_CTOR, 22 RECORD_CTOR, 23 MAP_CTOR, 24 SET_CTOR, 25 FIELD_PAIR,
26 MEMBER`. A compound literal's HEAD is `Atom(<ctor-leaf>)`, recognized by leaf-KIND identity (not head
text). Emit/read API: `Builder::compound(ctor,&children)/field_pair(k,v)/member(obj,key)` +
`Arenas::compound_ctor_leaf/field_pair_parts/member_parts`.

### 11.2 value-wire == AST-wire reconciliation (the content-address invariant)
The platform content-addresses ENCODED VALUES (`Hash-of(Blob)`), so the runtime value codec (op62/90) and
the compile-time value form (`const_value_ast`, `lower.rs`) MUST encode a value to BYTE-IDENTICAL output.
Key facts:
- **`rcdzc::codec::encode` runs NO canon — it serializes BUILD ORDER** (leaf pool = insertion order). The
  runtime `#[path]`-includes this codec, so op62/90 AND `const_value_ast` both serialize build-order. So the
  authoritative content-address value form = build-order = **HEAD-FIRST** (ctor/field-pair head leaf pushed
  BEFORE its children). (`cadenza-ast::codec::encode` DOES canon under std — but that is the front-end AST
  codec, a separate path from the runtime value form.)
- **Both encoders build head-first ctor-leaf forms** → byte-identical by construction. `const_value_ast`
  (me) flips its 5 compound sites + the set (from the old inner-first `((. Set of)(list …))` member-path to
  `SET_CTOR` head-first); op62 (v-runtime) flips `encode_value` name-heads → ctor-leaf kinds + set → SET_CTOR.
  The payloadless `FieldPair` kind (one shared deduped leaf, no `=` name leaf) DISSOLVES the record/field-pair
  build-order divergence v-static-data found.
- **Gate:** v-static-data's `op62 == const_value_ast` byte-EQ test across all shapes (list/tuple/option/map/
  record/set/nested) is the authoritative value-wire==AST-wire gate; `cadenza-ast` golden byte-vectors
  (record/set/map/tuple/list/nested, head-first) are the compiler-side cross-reference.

### 11.3 corpus migration (M3) — no dual-read window; use `cdz rewrite`; migrate everything
Operator ruled MIGRATE EVERYTHING via the AST refactoring tooling (`cdz rewrite`/`cdz corpus`). No
back-compat. Mechanics: v-syntax's flipped reader has NO bootstrap gap — old corpus forms (string/name-headed
lists, `(Set.of (list …))`) still parse; the reader ADDS native heads only for the `#word(` literal surface
(`Ctor`), a literal `(= k v)` direct body item of `#record(`/`#map(` (`FieldPair`, at read time), and all
member access (`Member`). `cdz rewrite` on same-surface `.sexp` is a TEXT splice validated by re-parse; the
corpus is TEXT the gate RE-READS, so `FieldPair`/ctor heads emerge on the output text (e.g. `#record((= a 1))`
re-reads with `FieldPair`). Migration must pass `cdz corpus check` + the ML round-trip.

### 11.4 flag-day assembly + coordination
M2 is a content-address FLAG-DAY assembled as ONE atomic squash (concierge lands). Lanes: ME (codec
leaf-kinds + APIs + `const_value_ast`/`field_pair` head-first flip + recognition flip + corpus migration),
v-syntax (`#word` reader/printer + `ast-binary-format.md` 20-26), v-spec-oracle (`§265/§269` + reflected-Ast
spec), v-runtime (op62/90 + op93/94 flip; both REQUIRED+DEBUG hashes bump → full guest recompile; v-nix
confirms both), v-static-data (byte-EQ gate). Downstream (sequenced AFTER): v-cadenza-backend (codec
consumer — build ctor-leaf head-first), v-ast-consolidate (unify rcdzc AST onto cadenza-ast + dep-lighten
`Leaf::Int` num_bigint→dep-free IntValue, post-M3 — wire-neutral), v-corpus-declines (re-baseline
`12-metaprogramming` after the corpus migration).

## 12. M2 flag-day ASSEMBLY RUNBOOK (2026-08-28) — authoritative; CORRECTS §11.3 migration mechanism

The push-button assembly procedure, validated piece-by-piece. Executes as ONE atomic squash the moment the
last peer arm (op62) lands.

### 12.1 Preconditions (arm readiness)
- **rcdzc side (me): DONE + green.** codec kinds 20-26, native ctor/FieldPair/Member API (both twins),
  recognition flip (resolve dual-accepts native + legacy via `compound_ctor_prim`), reflected-Ast Option-B
  variants, `const_value_ast` + `member_access` head-first EMIT flip. Pinned `rcdzc --lib` green except the
  deferred `regenerate_verify_kernel_bin` golden.
- **node_eq collapse: DONE** (`cadenza-ast 80a9934b0`). `ctor_head_key` maps native `Leaf::Ctor(c)` == the
  Name-alias == Str-primitive spelling → structurally_eq treats all three ctor-head spellings as one head.
  Byte content-addressing UNCHANGED (codec still byte-distinguishes heads). FieldPair/Member stay distinct.
- **v-syntax reader/printer: DONE + registered.** `--ref 74f3f6e1f` (delta `7beac9988..74f3f6e1f`,
  cadenza-syntax + `ast-binary-format.md` only; based on old origin/main → SQUASH the delta onto the
  integration base, do NOT merge the range). All 5 ctors have a `#word(…)` reader→Ctor + printer resugar;
  `#(…)` set literal → native `Ctor(Set)` (was Set.of); `(= k v)` field-pairifies on READ. Keep their 3
  fixes (alias_field_pairify, PATTERN .member→Member, match_to_let FieldPair head).
- **op62 (v-runtime): THE LONG POLE — pending.** runtime value-encode op62/90 head-first ctor-leaf +
  negatives as the neg-literal leaf (kinds 3-5, NOT `(- x)`) + REQUIRED/DEBUG runtime-hash bump. Byte target =
  my const_value_ast (golden vectors `676eea6e0`+`2fe9ccc58`; cite CONTENT not sha).
- **v-static-data byte-EQ arm** + **v-spec-oracle §265/§269 + reflected-Ast spec**: fold at assembly.

### 12.2 CORPUS MIGRATION MECHANISM — ml-convert route (SUPERSEDES §11.3 "cdz rewrite")
§11.3's blind `cdz rewrite` head-rename is UNSAFE: the corpus `(def (map …))` defines a `map` FUNCTION, so a
bare `(map it f)` is a HOF CALL, not a literal — a syntactic `(map ,@e)→#map(,@e)` would corrupt it (prototype
tick13). CORRECT mechanism = **`cdz convert sexpr → ml → sexpr`** per file: the ML surface DISAMBIGUATES a
literal (`[…]`/`#{…}`/`#(…)`/`(a,b)`) from a call `f(x)` from a pattern, so the round-trip nativizes exactly
the LITERALS/patterns and leaves HOF calls as calls — correct by construction; handles nesting, empty
`(list)`→`#list()`, record/map field-pairify, and (post-74f3f6e1f) all 5 ctors incl. set (`(set …)` and
`((. Set of)(list …))` both → `#(…)` → native `Ctor(Set)`). Prototype-VALIDATED (tick13) for
list/tuple/record/map + HOF-safety; set via 74f3f6e1f. Unambiguous string-head `("list" …)` forms, if any,
are directly-safe. Migration MUST pass `cdz corpus check` + the ML round-trip harness (`xtask roundtrip`),
not just the gate.

### 12.3 Ordered assembly steps
1. Rebase the integration branch onto current origin/main. Drop the transiently-folded STALE peer arms
   (early v-syntax cadenza-syntax fold; any v-runtime provisional decode/hash arm).
2. Fold FINAL peer `--ref`s onto the integration base (squash-not-merge): v-syntax `74f3f6e1f`, v-runtime
   op62 (final tip), v-spec-oracle spec arm, v-static-data byte-EQ test.
3. Run the corpus migration: `cdz convert sexpr→ml→sexpr` over `spec/semantics/*.sexp` (all 34). Diff-review.
4. Verify: `cdz corpus check` + `xtask roundtrip` (ML round-trip) green across all 34.
5. Regenerate `verify_kernel.bin` (`REGEN_VERIFY_KERNEL_BIN=1`) — const_value_ast + op62 + reader all shifted
   kernel bytes.
6. Belt-and-suspenders: run a `#(…)` SET behavior corpus case through rcdzc (value + a set op) to confirm
   native `Ctor(Set)` consumed identically (node-identity says yes; verify anyway).
7. v-nix confirms BOTH runtime hashes (REQUIRED + DEBUG) — full guest recompile.
8. Gate ALL-OR-NOTHING: pinned `rcdzc --lib` (incl. the re-baselined kernel-bin) + `cargo test -p
   cadenza-syntax` (corpus_roundtrip now GREEN) + cadenza-ast + `--target platform` + cdzast byte-stability
   re-baseline + clippy `--all-targets -D warnings` + pinned `cargo fmt --all --check`.
9. Hand the concierge the single atomic `--ref` for the flag-day land.

### 12.4 Post-flag-day (sequenced AFTER M2 lands)
Ping v-ast-consolidate (num_bigint→IntValue swap), v-corpus-declines (re-baseline 12-metaprogramming),
v-cadenza-backend (ctor-leaf head-first consumer). M3 (delete legacy string/name-head recognition) is a
separate follow-on: flip the deferred boundary emitters (effect-request record in `lower.rs:16651` +
`db.push_field_pair`; v-effects' tuple-projection `.` sites in effects.rs) to native then.
