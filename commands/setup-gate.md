# Command — setup-gate

**Purpose.** Configure the conformance gate for a target language: materialize or
refresh the regenerated `[[source]]` half of the duvet configuration, then verify
the gate runs. This is the "instructions an agent executes" for gate setup —
duvet initialization is itself a spec-driven procedure, not a manual step.

**Agent-agnostic.** Neutral prompt body; assumes a shell and the `duvet` CLI.
The gate's meaning is fixed in `spec/capabilities/conformance-gate.md`; this
command performs the setup that spec describes and is invoked by `build`'s
synthesize phase before the requirement gate is run.

## Inputs

- The target language of the code being generated (e.g. the seed host language a
  generation chose, or the language a later generation targets).
- `.duvet/config.toml`, whose `[[specification]]` half is stable and MUST NOT be
  modified by this command (conformance-gate.md §"The Requirement And Code Sides
  Are Separated").

## Procedure

1. Determine the target language and its citation comment style.
2. Rewrite ONLY the `[[source]]` block(s) of `.duvet/config.toml` for that
   language:
   - Set `pattern` to the code and test globs for the language, rooted at
     `implementation/` (where the generated compiler lives; it is gitignored). The
     default Rust pattern is `implementation/**/*.rs`.
   - Set `comment-style = { meta = "...", content = "..." }` if the language's
     comment markers differ from the default (see "How a citation is written"
     below for the defaults and how matching works).
   - Add a separate `[[source]]` block marking the test globs where citations of
     `type=test` are expected, if the language separates tests by path.
3. Leave every `[[specification]]` block untouched (the requirement side is
   stable and hand-owned — conformance-gate.md §"The Requirement And Code Sides
   Are Separated"; a regeneration of the code side MUST NOT alter the requirement
   side).
4. Verify: run `duvet report` and confirm it parses the config, extracts
   requirements from all specifications, and emits the JSON report without error.

## How a citation is written (READ THIS — the marker is load-bearing)

A citation in source code is a two-line comment immediately above the code that
satisfies a requirement, then (optionally) the same above a test:

```rust
//= spec/contracts/deterministic-value-form.md#a-value-has-one-canonical-byte-form
//# Each serializable value MUST have exactly one canonical byte encoding.
fn canonical_bytes(/* … */) { /* … the implementation … */ }
```

- **`//=` is the META marker** — it names the requirement by
  `<spec-path>#<section-slug>`. The slug is the GitHub-style anchor of the section
  heading (lowercased, spaces→hyphens). The spec path is project-root-relative,
  matching a `[[specification]]` `source`.
- **`//#` is the CONTENT marker** — it carries the exact requirement sentence.
  **duvet validates this text against the spec:** the quoted text MUST be found
  within that section, or the gate HARD-ERRORS with
  `could not find text in section "<slug>" of <file>` (naming the source file and
  line). This is precisely how the `(file, section, exact sentence)` identity model
  in [conformance-gate.md](../spec/capabilities/conformance-gate.md) §"Identity Is
  The Quoted Sentence" and [constitution.md](../constitution.md) §XII/§XIII is
  enforced: reword a requirement so the cited words are no longer present, and every
  citation that quoted the old wording fails the gate.

`//=` and `//#` are duvet's **defaults**; the Rust `[[source]]` needs **no**
`comment-style` override. Only set `comment-style = { meta = "...", content = "..." }`
for a language whose line-comment marker is not `//`.

### Citation type — how the impl + test coverage rule is satisfied

[conformance-gate.md](../spec/capabilities/conformance-gate.md) §"Coverage Requires
Implementation And Test" counts a requirement covered only when it has **both** an
implementation citation and a test citation. duvet expresses the role on an extra
`//= type=<value>` meta line (default when omitted is `implementation`):

```rust
//= spec/capabilities/core-semantics.md#conditionals-evaluate-one-branch
//= type=test
//# A conditional MUST evaluate only the branch its condition selects.
#[test]
fn conditional_evaluates_only_selected_branch() { /* … exercises the behavior … */ }
```

Types (from <https://awslabs.github.io/duvet/annotations.html>):

- **`implementation`** — default; the code implements the cited text. (Satisfies the
  implementation half of coverage.)
- **`test`** — the code tests the cited behavior. (Satisfies the test half.) A cited
  test MUST actually exercise the behavior, not merely restate the text
  (conformance-gate.md §"Coverage Requires Implementation And Test", §"A Citation
  Discharges Its Own Requirement").
- **`implication`** — counts as **both** implementation and test at once; use only
  for a requirement that is correct by construction (e.g. enforced by the type
  system), so a single citation legitimately covers both halves.
- **`exception`** — deliberately not implementing the cited text; add
  `//= reason=<why>`.
- **`todo`** — not yet implemented, on the roadmap; add `//= tracking-issue=<id>`.
- **`spec`** — mark additional spec text as a requirement even without an RFC-2119
  keyword; add `//= level=MUST` (rarely needed here — our requirements already carry
  keywords).

So a load-bearing MUST/SHALL needs an `implementation` citation **and** a `test`
citation (or one `implication`) to pass the gate; `exception`/`todo` leave it
uncovered and MUST NOT appear on a promoted generation's load-bearing requirements
(conformance-gate.md §"The Gate Is The Promotion Bar").

**Behavioral requirements need an executing test, not a shape-matching one.**
conformance-gate.md §"A Behavior Requirement Is Covered Only By Execution" and
§"A Cited Behavioral Test Is Sensitive To Its Requirement" mean a citation whose
test does not run the behavior and fail when it breaks does not discharge the
requirement. For a behavioral requirement, the discharging path is a
`spec/semantics/*.sexp` case that the reference interpreter executes to its recorded
output (the behavior gate — see `commands/gate.md`); the duvet `type=test` citation
points at the code path that runs it.

**GOTCHA (this cost a full spike a false "duvet ignores sentence text" finding):**
if you write the content line with the WRONG marker (e.g. `//%` instead of `//#`),
duvet treats it as an ordinary comment, parses only the `//=` section reference, and
matches by section alone — making it look like the quoted sentence is not checked.
It is checked, but only when the marker is exactly `//#`. Always verify a new gate
setup with a deliberately-wrong quote and confirm duvet errors.

Run a report against a non-default config location with
`duvet report --config-path <path>` (bare `duvet report` discovers
`.duvet/config.toml`; source globs and `[[specification]]` sources resolve relative
to the project root, the parent of `.duvet/`).

## Guardrails

- This command MUST NOT add, remove, or edit any `[[specification]]` block.
- This command MUST keep `format = "markdown"` on every specification (removing
  it silently drops all requirements — the default format is IETF and extracts
  nothing from our prose).
- If the target language changes between generations, re-run this command; the
  same requirement set then gates the new language purely by swapping
  `[[source]]` (conformance-gate.md §"The Requirement And Code Sides Are Separated":
  the code side MUST be regenerated per target language).

## Output

`SETUP-GATE: OK <language>` on success, listing the `[[source]]` globs written;
otherwise the parse or extraction error to fix.
