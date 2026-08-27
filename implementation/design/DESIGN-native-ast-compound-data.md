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

### 3.2 Layer 2 — the stored representation of the tag (OPERATOR DECISION — see §6 D1)
"A way to clearly say what's a record *in the cadenza-ast*" is a statement about the **stored node**.
Three candidate representations, additive-evolution-ordered:

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
- **D-SURFACE — s-expr paren PREFIX: RULED = full-word `#list(…)` (operator, 2026-08-27):** with the
  string head gone and a bare-name head ambiguous with application, the s-expr textual surface can no
  longer write a compound as a pure paren form; it takes an explicit constructor **`#`-word prefix on
  the paren** — `#list(1 2 3)`, `#tuple(a b)`, `#record(= x 1)`, `#map(k v)`, `#set(1 2 3)` — consistent
  with the existing `#"…"`/`#{…}`/`#(…)`/`#\` reader-directive family, unambiguously distinct from an
  application `(list …)`. This is a `cadenza-syntax` s-expr reader+printer change and gates the **corpus
  migration** (M2/M3) but NOT the Layer-1 dispatch work. The ML surface (`[…]`/`{…}`/`#{…}`/`#(…)`/`(a,b)`)
  already has its own literals. (Exact record/map field-pair spelling inside the prefix form —
  `#record((= x 1) …)` vs `#record(= x 1 …)` — pinned at the reader-design increment.)
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
