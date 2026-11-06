# Design — a config-schema framework: declare a config's shape + validation, compile it to a released wasm validator

**Author:** design pass (fleet `v-config-schema`).
**Audience:** the `vertical` agent(s) that build it — spans `rcdzc` (a thin schema-declaration surface
layered on the existing type system, refinement types, and contracts), the component/WIT boundary
(`cdz compile`, `component-abi`, `cdz-world-artifact`), and a new corpus file
`spec/semantics/NN-config-schema.sexp`. Depends on refinement types + contracts
(`[[spec/capabilities/verification-layers.md]]`) and value-interchange decode
(`[[spec/capabilities/value-interchange.md]]`), so it is sequenced after those are landable.
**Status:** DESIGN — PROPOSAL FOR OPERATOR REVIEW, not for build. This is a proposal-first doc: it sets
the surface, the model, requirements, and acceptance criteria, and slices the work so a later vertical
can build it. Several decisions carry a chosen default with open sub-questions flagged for the operator.
**Subsystem:** `rcdzc` (schema surface + validation lowering), the component/WIT boundary (validator
artifact + host interface), `cadenza-syntax` (a small schema-declaration marker, if adopted), and a new
`spec/semantics/NN-config-schema.sexp` corpus file.

## 0. The problem — READ FIRST

A configuration value produced by one component is consumed by another. Today the *producer's* notion of
what it may emit and the *consumer's* notion of what it will accept are maintained independently — often
in separate codebases — and they drift apart. The mismatch is discovered **late**: not when the config is
generated, but when the consumer starts up, acts on the config, and fails. The failure then surfaces far
from its cause (a distant crash or an unrelated-looking symptom), so diagnosis is expensive.

A concrete, generic instance of the class — the shape this framework must catch:

> A server's config carries a numeric field `max-connections`. One producer path defaults it to `0`. A
> newer consumer version rejects `0` (it now requires `max-connections >= 1`), because `0` used to mean
> "unlimited" but no longer does. The producer and consumer never shared a schema, and the tests
> exercised only the producer path that emitted a valid value — so the incompatibility shipped and the
> consumer crash-looped on startup.

Every element of that story is a validation category this framework must express **once, declaratively,
so both producer and consumer check against the same schema at config-generation time** rather than
re-deriving the rules independently in imperative startup code:

1. **Type** — a field is a number, a string, a bool; numbers are frequently encoded as strings, sometimes
   with unit suffixes (`"512mb"`), and must parse to a typed value.
2. **Required-field presence** — a mandatory field or whole section must not be missing/empty.
3. **Numeric bounds** — non-zero / positive / `>= floor` / `<= ceil` / within a range / a multiple of N.
4. **Enum / closed-set membership** — a string field constrained to a known set (and sometimes to a set
   defined *elsewhere* in the same config, e.g. a tier name that must exist in a `sizes` table).
5. **Sentinel-value semantics** — whether a magic value like `0` is legal and what it means, made
   explicit so its meaning cannot drift silently between versions.
6. **Cross-field consistency** — field A must relate to fields B/C (a replica count must fit within a
   node budget; `workers * memory-per-worker <= total-memory`).
7. **Producer/consumer contract agreement** — the set of values a producer *can emit* must be within the
   set a consumer *will accept*, checked as one **shared, versioned** schema.
8. **Template-resolution completeness** — no unresolved placeholders (`{{…}}`) remain when the config is
   validated.
9. **Fail-fast, close-to-source reporting** — validate *before* acting, and report the offending field,
   value, and rule directly.

**The thesis:** Cadenza already has every primitive this needs — structural records/sums for shape,
**refinement types** for field-level constraints, **contracts** for cross-field constraints,
**value-interchange `decode`** for parsing an external payload against a type-directed schema, and
**content-addressed wasm components** with a **contract-versioned ABI** for releasing a tagged validator
another service loads. A config-schema framework is therefore mostly a **thin, opinionated composition**
of existing capabilities plus one new released-artifact shape (a "validator component"), not a new
language. This proposal picks the surface and the seams so the composition is coherent and gated.

## 1. Goals & non-goals

**Goals.**
- **G1.** Declare a config's *structure* (fields, types, nesting, collections, optionals, required-ness)
  in Cadenza, reusing the existing record/sum surface.
- **G2.** Declare *field-level* validation (ranges, non-zero/positive, enums, string formats, multiples)
  as refinement types on those fields, so the constraint travels with the type and erases to its base.
- **G3.** Declare *cross-field* validation (relationships between multiple fields) as contracts on the
  schema record.
- **G4.** Compile a schema to a **released, content-addressed, tagged wasm validator component** with a
  stable exported interface a host calls with a raw config payload.
- **G5.** Return **structured, human-readable errors** pinpointing field path, offending value, the rule
  that failed, and a message — and collect *all* failures, not just the first.
- **G6.** Let a producer and a consumer **pin the same validator artifact** (by content address + tag) so
  they validate against one contract; validate at generation time, not just at startup.
- **G7.** Accept common external config encodings (TOML/JSON at minimum) as input to the validator.

**Non-goals.**
- **N1.** Not a general-purpose data-migration/transformation engine. The validator *accepts or rejects*
  (and normalizes types, e.g. `"512mb"` → bytes); it does not rewrite or upgrade config across versions.
- **N2.** Not a new schema DSL divorced from the type system. Reject inventing a parallel type language
  (Decision 2).
- **N3.** Not template *substitution*. The framework can *reject* an unresolved placeholder (G-adjacent),
  but rendering templates is the caller's job.
- **N4.** Not runtime policy/authz. This is *shape + value* validation of static config.

## 2. Decision — the schema-definition surface

**(2a) REJECTED — a bespoke schema DSL** (a new `(schema …)` form with its own type vocabulary). It would
duplicate the type system, drift from it, and force a second inference/checking path. The motivating
lesson is precisely that a schema divorced from the consumer's real types drifts.

**(2b) CHOSEN — a schema *is* a Cadenza type declaration.** A config schema is an ordinary record type
whose fields carry refinement-typed values, wrapped in a nominal type so it has a stable identity:

```
;; structure + field-level constraints, reusing records + refinements
(type ServerConfig
  (: port           (Refine UInt16 (fn (p) (and (>= p 1024) (<= p 65535)))))   ; range
  (: max-connections (Refine Int64  (fn (n) (> n 0))))                          ; must-be-positive (the motivating case)
  (: mode            (Enum "parallel" "sequential"))                            ; closed-set membership
  (: timeout-ms      (Refine Int64  (fn (t) (>= t 0))))
  (: replicas        (Refine Int64  (fn (r) (>= r 1))))
  (: labels          (Optional (Map String String))))                          ; optional + collection
```

- **Structure / nesting / collections / optionals** → records, nested records, `List`/`Map`, and an
  `Optional`/option type. All already exist (`[[spec/capabilities/type-system.md]]`,
  `spec/semantics/05-compound-types.sexp`). A field with no `Optional` wrapper is **required**; presence
  is a type-directed decode obligation (Decision 5).
- **Field-level constraints** → refinement types (`[[spec/capabilities/verification-layers.md]]` §70–86):
  a refinement narrows a base type by a predicate, is checked, and **erases to its base at runtime**. This
  is exactly "must be non-zero / in range / a multiple of N" and it composes with the type. `Enum "a" "b"`
  is sugar for a refinement over `String` (or a closed sum) — see OQ-3.
- **Cross-field constraints** → a contract on the schema record (`verification-layers.md` §54–70:
  pre/post/invariants, discharged statically where possible, else as a checked predicate):

```
(contract ServerConfig
  (invariant (fn (c) (<= (* c.replicas c.memory-per-replica) c.total-memory))
             "replicas × memory-per-replica must fit within total-memory"))
```

- **Sentinel semantics made explicit** → because the legal set is a refinement/enum, a sentinel is either
  *in* the set (documented, e.g. `(Enum-int 0 unlimited)`) or *out* of it (rejected). The meaning can no
  longer drift silently: changing whether `0` is legal is a **visible change to the schema type**, which
  changes the validator's content address (Decision 8) — the drift becomes a reviewable diff, not a
  buried `if`.

A **`schema`-tagged module** (Decision 4) marks a module as a released config schema so tooling knows to
emit a validator component from it. If a lightweight marker beyond "a nominal type + a contract" is
needed, it is at most a one-token annotation; the surface stays the type system.

## 3. The validation model

Validation = **type-directed decode of an external payload against the schema type**, then discharge the
schema's refinements and contract. Concretely, three layers, all from existing machinery:

1. **Decode (structure + type + presence).** `decode` is directed by a known type
   (`[[spec/capabilities/value-interchange.md]]`; `DESIGN-open-sums-and-schema-typed-payloads-rcdzc.md`):
   it parses the payload against `ServerConfig`, yielding a typed `Result`, never a trap. A missing
   required field, a wrong type, or a number-as-string that won't parse is a **decode error** with the
   field path. Unit-suffix parsing (`"512mb"` → bytes) is a typed coercion attached to the field type
   (OQ-4).
2. **Refine (field-level rules).** Each refinement predicate on a field is checked against the decoded
   value. A failure names the field and the predicate's message.
3. **Contract (cross-field rules).** The schema record's contract invariants are checked against the whole
   decoded record. A failure names the invariant's message and the fields it read.

**Collect-all, not fail-fast-internally.** The validator MUST gather every failure across all three layers
into one error list before returning, so a caller sees *all* problems in one pass (the motivating incident
was worsened by discovering one problem at a time). Fail-fast is the *caller's* posture (reject up front);
the validator itself is exhaustive.

**Template completeness (G-adjacent).** An unresolved `{{…}}` in a string field is a decode/refine failure
("field `x` contains an unresolved placeholder"). The framework detects it; it does not substitute.

## 4. Error-reporting design

The validator returns `Result<Config, ValidationErrors>` where a failure is a list of structured records:

```
(type ValidationError
  (: path    (List PathSegment))   ; e.g. [server, pool, max-connections]  (field/index segments)
  (: rule    RuleTag)              ; Type | Required | Range | Enum | Multiple | Sentinel | CrossField | Template
  (: value   (Optional String))    ; the offending value, rendered
  (: message String))              ; human-readable: what & why & (where possible) the fix
```

- **Rich + actionable** (rustc is the bar per the diagnostics standing directive): the message states the
  field path, the value seen, the rule, and — where the rule affords it — the acceptable set/range so the
  fix is obvious ("`server.max-connections` = 0 is invalid: must be >= 1 (0 is not a valid connection
  count; use a positive integer)").
- **Structured first, string second.** Callers get machine-readable `path`/`rule` for programmatic
  handling and a rendered `message` for humans. A rendering helper produces the human string from the
  structured error via the canonical value-render path (do not hand-roll — per the render standing rule).
- **Stable rule tags** so a consumer can branch on `Range` vs `CrossField` without string-matching.

## 5. Compile-to-wasm — the validator artifact

A schema module compiles, via the ordinary compile flow (`cdz compile`), to a **content-addressed wasm
component** — the "validator component" — exactly like any other released Cadenza component
(`[[spec/contracts/build-tool-interface.md]]`, `[[spec/contracts/component-abi.md]]` contract v5). No new
backend: the schema is Cadenza code (types + refinements + a `validate` entry fn), so it lowers through the
existing component emit.

The validator component exports one function (see Decision 6 for the WIT shape). Its dependency closure is
just the runtime value-heap; it imports no host function, so any host that speaks the component ABI can
load and run it. It records its required runtime content address (`component-abi.md:182`), so a pinned
validator resolves the exact runtime it was emitted against.

## 6. Decision — the host / validator interface (WIT)

**(6a) CHOSEN — a single generic entry, payload-in / structured-result-out.** The validator exports one
function over the canonical component boundary (`[[spec/contracts/host-interface-binding.md]]`,
`spec/semantics/28-wit-abi-boundary.sexp`):

```wit
// synthesized from the schema's validate fn signature; shape only
interface config-validator {
  record validation-error { path: list<path-segment>, rule: rule-tag, value: option<string>, message: string }
  variant validate-result { ok, err(list<validation-error>) }
  // input is the raw config bytes + a tag for its encoding (toml|json|...)
  validate: func(payload: list<u8>, encoding: config-encoding) -> validate-result;
}
```

- The host calls `validate` with the raw config bytes and an encoding tag; it gets back `ok` or a list of
  structured errors. The typed `Config` value stays *inside* the guest (it does not generally cross the
  boundary as a rich type — ABI v5 monomorphizes and generics do not cross); the boundary carries the raw
  payload in and the accept/errors verdict out. A variant that also returns the *normalized* config (with
  units parsed, defaults applied) is OQ-5.
- The WIT world can be **synthesized** from the validate fn's annotations, or a shared world can be
  **imposed** via a `wit-world` artifact (`implementation/seed/crates/cdz-world-artifact/`,
  `cdz compile guest.cdz wit-world:<world>=<world>.bin`) so many validators share one host-facing world.
  Decision: default to a **single shared `config-validator` world** so every validator any service loads
  presents the identical interface (a host writes one loader). (Exact `cdz compile` flag surface to be
  confirmed against `implementation/seed/crates/cdz/src/compile_args.rs` at build time — OQ-6.)

**(6b) REJECTED — one bespoke exported function per field/rule.** It would leak the schema into the ABI
and defeat "one loader for all validators."

## 7. Input formats

The boundary takes raw bytes + an `encoding` tag (`toml` | `json` | …). Inside the guest, the bytes are
parsed to a canonical Cadenza value and then `decode`d against the schema type. Cadenza already carries
front-end readers for multiple syntaxes (`cadenza-syntax-json`, `cadenza-syntax-toml`); the validator
reuses a reader to turn bytes into the canonical value form before the type-directed decode. Adding a new
encoding is adding a reader arm, not touching the schema. **TOML and JSON are the required v1 encodings;**
YAML is OQ-7.

## 8. Versioning & tagging

Two axes, both already defined by the platform:

- **Identity = content address.** The validator is a content-addressed component
  (`[[spec/contracts/reproducible-derivation.md]]`). *Any* change to the schema type, a refinement, or a
  contract changes the address — so "did the acceptance contract change?" is answered by "did the address
  change?", turning silent drift into a visible artifact diff.
- **Release tag.** A released validator is published under a human tag (e.g. `server-config@3`) resolving
  to a content address, so a **producer and consumer pin the same tag** and are guaranteed to validate
  against the identical contract. Bumping the tag is the explicit, reviewable act of changing the contract.
- **ABI/contract version** rides `component-abi.md`'s existing contract-version + additive-evolution rule
  (`build-tool-interface.md:130-134`): the validator *interface* (Decision 6) evolves additively or with an
  explicit version bump + migration note.

**Producer-side use (the prevention story).** The producer runs the *same* pinned validator on the config
it is about to emit, at generation time. A `max-connections = 0` default is rejected *where it is
produced*, before release — not discovered as a crash-loop in the consumer. This is the whole point of a
*shared* schema: both sides check one contract.

## 9. The increments — how a vertical lands it

Top-to-bottom, each a landable + gated slice (mirrors the sibling-vertical increment style):

- **CS1 — schema surface & structure.** A nominal record type as a schema; required vs `Optional`; nesting;
  `List`/`Map`. Gate: corpus cases decoding a well-formed payload to the typed record; a missing required
  field → structured `Required` error. (Depends only on records + decode.)
- **CS2 — field-level refinements.** Range / non-zero / positive / multiple-of / string-format refinements
  on fields, checked during validation; each failure → a `Range`/`Multiple`/… `ValidationError`. Gate: the
  motivating `max-connections >= 1` case (0 rejected with an actionable message); a valid value accepted.
- **CS3 — enums & sentinels.** `Enum "a" "b"` membership; explicit legal sentinels; off-set values → `Enum`
  error naming the allowed set. Gate: corpus cases for accept/reject + the message pins the allowed set.
- **CS4 — cross-field contracts.** Schema-record invariants relating multiple fields; failure → `CrossField`
  error naming the invariant + fields. Gate: `replicas × mem <= total` accept/reject cases.
- **CS5 — collect-all errors.** A payload violating several rules returns *all* of them in one list, ordered
  deterministically by field path. Gate: a multi-violation payload → the exact expected error set.
- **CS6 — encodings.** TOML + JSON input → canonical value → decode; a number-as-string / unit-suffix
  (`"512mb"`) coercion. Gate: same schema, both encodings, equal verdicts; an unparseable value → `Type`.
- **CS7 — the validator component + WIT interface.** `cdz compile` a schema module to a validator component
  exporting `validate(payload, encoding) -> validate-result`; a host round-trip (call with bytes, get the
  verdict). Gate: a `28-wit-abi-boundary`-style boundary case exercising the exported `validate`.
- **CS8 — release tag + producer/consumer pin.** Publish under a tag; a second component pins the tag and
  validates. Gate: a cross-component-interop-style case where a "producer" and "consumer" resolve the same
  tagged validator and agree.
- **CS9 — template-completeness rule.** Reject an unresolved `{{…}}` in a string field with a `Template`
  error. Gate: accept resolved / reject unresolved.

CS1–CS5 need no boundary work (pure in-language validation over a decoded value) and can land first; CS6
adds encodings; CS7–CS8 add the released-artifact story; CS9 is a small standalone rule. Keep each slice a
coherent gated unit (per the fleet FLOOR/CEILING rule).

## 10. Seams / file anchors

| What | Where |
| --- | --- |
| Refinement types (field-level rules) | `spec/capabilities/verification-layers.md` §70–86 |
| Contracts (cross-field rules) | `spec/capabilities/verification-layers.md` §54–70 |
| Records / sums / optionals / collections | `spec/capabilities/type-system.md`; `spec/semantics/05-compound-types.sexp` |
| Type-directed `decode` (structure + type + presence) | `spec/capabilities/value-interchange.md`; `implementation/design/DESIGN-open-sums-and-schema-typed-payloads-rcdzc.md` §2 |
| Component emit + content-address | `spec/contracts/build-tool-interface.md`; `spec/contracts/component-abi.md` (v5) |
| Host-facing WIT interface | `spec/contracts/host-interface-binding.md`; `spec/semantics/28-wit-abi-boundary.sexp` |
| Shared WIT world artifact | `implementation/seed/crates/cdz-world-artifact/`; `cdz compile … wit-world:<w>=<w>.bin` |
| Runtime pinning / versioning | `spec/contracts/reproducible-derivation.md`; `component-abi.md:182` |
| Cross-component pin (producer/consumer) | `spec/capabilities/cross-component-interop.md`; `implementation/design/DESIGN-package-linking.md` |
| Compiler arg surface (validate `cdz compile` flags) | `implementation/seed/crates/cdz/src/compile_args.rs` |
| New corpus file | `spec/semantics/NN-config-schema.sexp` (to be numbered) |

## 11. The gate that protects it

- A new `spec/semantics/NN-config-schema.sexp` graded by `cargo xtask gate --files
  spec/semantics/NN-config-schema.sexp --target wasm` (add `--target rust` if a slice touches
  backend-specific emit), covering each CS increment's accept/reject cases with the **exact structured
  error** pinned (rule tag + field path + message text), per the diagnostic-quality corpus convention.
- A boundary case in the `28-wit-abi-boundary` family exercising the exported `validate` (CS7), and a
  cross-component case (CS8) where two components resolve one tagged validator.
- `rcdzc` unit tests for the refinement/contract lowering the corpus can't isolate.
- Per the corpus policy: assert the **idealistic** behavior; if a needed primitive (refinements, decode)
  isn't landable yet, mark the case `todo` asserting the expected value and route the gap to its owner —
  never work around it.

## 12. Requirements & acceptance criteria

- **R1 (structure).** A schema declares fields, types, nesting, collections, optionals, required-ness.
  *Accept:* a well-formed payload decodes to the typed record; a missing required field → a `Required`
  error with the field path.
- **R2 (field rules).** Range / non-zero / positive / multiple-of / string-format / enum. *Accept:* the
  `max-connections >= 1` case rejects `0` with an actionable message naming the field, value, and rule;
  accepts a valid value.
- **R3 (cross-field).** *Accept:* a `replicas × mem <= total` invariant rejects an over-budget config
  naming the invariant + fields; accepts a within-budget one.
- **R4 (errors).** Structured (`path`/`rule`/`value`/`message`) + human-rendered; **all** failures
  returned in one deterministic list. *Accept:* a multi-violation payload returns the exact expected error
  set in field-path order.
- **R5 (compile-to-wasm).** A schema compiles to a content-addressed validator component exporting
  `validate(payload, encoding) -> validate-result`. *Accept:* a host round-trip returns `ok` for a good
  payload and the structured errors for a bad one.
- **R6 (versioning + shared pin).** The validator is content-addressed and tag-released; a producer and a
  consumer pin the same tag. *Accept:* two components resolving one tag agree on the verdict; changing any
  rule changes the content address.
- **R7 (encodings).** TOML + JSON input, with number-as-string / unit-suffix coercion. *Accept:* the same
  schema over both encodings yields equal verdicts; an unparseable value → a `Type` error.
- **R8 (fail-fast, close-to-source).** The validator is callable at config *generation* time (producer
  side), not only at consume time. *Accept:* a producer running the pinned validator rejects a bad default
  before release.

## 13. Resolved vs. open

**Resolved (do not re-litigate):**
- A schema is a Cadenza type declaration + refinements + a contract, not a bespoke DSL (Decision 2).
- The released validator is a content-addressed wasm component with a single generic `validate` entry over
  the component ABI (Decisions 5, 6).
- Errors are structured + collect-all (Decisions 3, 4).
- Identity + drift-detection = content address; contract change = tag bump (Decision 8).

**Open questions (each with a chosen default; operator to confirm):**
- **OQ-1.** Does this need refinement types + contracts to be *landable first*, or can CS1–CS3 ship against
  a decode-only subset while refinements mature? *Default:* sequence CS1 (decode) independently; CS2+ gate
  behind refinements — coordinate with the verification-layers owner.
- **OQ-2.** Is a `schema`-tagged **module** (with a self-describing descriptor,
  `spec/capabilities/modules-and-namespaces.md:84`) the right release unit, vs. a plain nominal type + a
  known `validate` export? *Default:* start with a nominal type + `validate` export; adopt the module
  descriptor if release ergonomics need it.
- **OQ-3.** `Enum "a" "b"` as sugar over a `String` refinement vs. a closed sum type. *Default:* closed sum
  where the members are used as tags; string-refinement where they're free-form strings.
- **OQ-4.** Unit-suffix coercion (`"512mb"`): a per-field typed coercion attached to the field type vs. a
  caller-side pre-parse. *Default:* a field-type coercion so the schema owns the unit semantics.
- **OQ-5.** Should `validate` also return the *normalized* config (units parsed, defaults applied), not just
  accept/errors? *Default:* v1 returns accept/errors only (N1); a `validate-and-normalize` variant is a
  later increment.
- **OQ-6.** Exact `cdz compile` flag surface for schema→validator + shared-world imposition (confirm against
  `compile_args.rs`). *Default:* reuse the documented `wit-world:<w>=<w>.bin` artifact path.
- **OQ-7.** YAML as a v1 encoding? *Default:* TOML + JSON in v1; YAML follows if there's demand.

## Ownership / hand-off

Owned by `v-config-schema`. Build order: CS1–CS5 (in-language validation, no boundary) → CS6 (encodings)
→ CS7–CS8 (validator component + release/pin) → CS9 (template rule). Coordinate with the verification-layers
owner (refinements/contracts must be landable for CS2–CS4) and with the component/WIT owners for CS7–CS8;
`note` those verticals before touching a shared seam. This doc is a **proposal for operator review** — it
is not scheduled for build until the operator approves the surface and resolves the open questions.
