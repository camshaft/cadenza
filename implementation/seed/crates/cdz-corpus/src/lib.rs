//! Corpus reader: turn a `spec/semantics/*.sexp` file into a flat, easily-consumed record stream.
//!
//! A corpus file is a sequence of `(case "desc" (input <program>) <primary> <annotations>…)` forms
//! (the test-DSL vocabulary, `spec/semantics/README.md`). This module parses each case, NORMALIZES
//! its `input` to the runnable export shape the compiler expects, and emits one text record per case
//! so a thin driver (xtask) can run each program through the pipeline and compare — without a parser
//! or any dependency of its own.
//!
//! Normalization (the on-the-fly bridge from today's mixed corpus to the export interface):
//!   - a bare expression `E`              → `(do (def (main) E) (export main))`
//!   - an old `(module name def…)`        → `(do def… (export main))`
//!   - an already-shaped `(do … (export …))` → passed through unchanged
//!
//! Record format — line-oriented, one record per case, records separated by a `---` line. Each field
//! is `<key>\t<value>` on one line (the normalized program prints on a single line, so this is safe):
//!   case\t<description>
//!   program\t<normalized program, s-expression on one line>
//!   call\t<export>                 (present only when the case has a `(call …)` clause)
//!   arg\t<value-form>              (zero or more, in order; the arguments to the call)
//!   expect\t(output <value-form>) | (error <CODE>) | (trap <reason>)
//!   ---

/// The command surface (`CorpusArgs` + `run`), embeddable so the unified `cdz` binary can mount
/// `cdz corpus`. The standalone `cdz-corpus` bin is a thin shim over it.
pub mod cli;

use cadenza_syntax::ast::{Arenas, Builder, StructId};
use cadenza_syntax::{codec, sexpr};

/// A single parsed + normalized corpus case, ready to run.
pub struct Record {
    pub description: String,
    /// The `input` rewritten to the runnable export shape, as one-line s-expression text. Consumed by
    /// the stdout record stream + the xtask gate driver.
    pub program: String,
    /// The SAME normalized program as `program`, encoded as BINARY AST (`codec::encode`) — the form the
    /// nix corpus pipeline's shred emits (`program.ast`), fed straight to the compiler with no reparse.
    /// Built from the one normalized arena alongside `program` (the text is `sexpr::print` of it, the
    /// bytes are `codec::encode` of it — neither round-trips through the other).
    pub program_ast: Vec<u8>,
    /// The RAW `(input …)` payload form `E` as BINARY AST — the input subtree VERBATIM, BEFORE the
    /// `build_normalized_program` rewrite that wraps a bare expression as `(do (def (main) …) (export
    /// main))`. This is the exact form the `--quote-wrap` corpus pass reifies: it synthesizes a program
    /// that `(quote E)`s this raw input and round-trips it through the binary codec across the caller
    /// boundary (`design/DESIGN-quote-corpus-roundtrip-pass.md`). Encoded from the one live arena where
    /// the input node sits (no reparse), exactly like `wit_world_ast`. Always present (every case has an
    /// `(input …)`); unused by the ordinary compile/run path (which reads `program_ast`).
    pub input_ast: Vec<u8>,
    /// Sibling LIBRARY modules of a multi-file PACKAGE case (`DESIGN-package-linking.md`), each a
    /// `(name, program-text)` from a `(module "name" <prog>)` clause — the files the ENTRY (`program`,
    /// named `main`) may `(import …)` from. Empty for the common single-file case (then `program` is
    /// compiled alone, exactly as before). When non-empty, the gate driver writes every module + the
    /// entry to a temp dir and runs `cdz compile <files> --entry main`.
    pub modules: Vec<Module>,
    /// PEER components of a CROSS-COMPONENT case, each a `(interface, provider-program)` from a `(peer
    /// "<iface>" <prog>)` clause — separately-compiled providers the entry (a consumer) binds across the
    /// live boundary via `(extern <iface> …)`. Empty for the common single-component case. When non-empty,
    /// the gate compiles each peer to its OWN component and runs the entry with `cdz-run --peer
    /// <iface>=<path>` (`run_with_peers`), rather than linking them into one component like `modules`.
    pub peers: Vec<Peer>,
    /// One or more TRIALS: each pairs an optional `(call …)` with the result it must produce. A case
    /// with a single `(output)`/`(error)`/`(trap)` and no `(call …)` is ONE trial with `call: None`
    /// (the common nullary case — invoke the sole export with no arguments). A case that INTERLEAVES
    /// several `(call …) (output …)` pairs runs the SAME compiled program once per trial, comparing
    /// each against its own result — so one case documents a shape exercised at several runtime
    /// arguments (`(def (main (: x UInt8)) (+ x 1))` called with 100→101, 200→(traps), …). The case
    /// passes iff EVERY trial passes. Always non-empty.
    pub trials: Vec<Trial>,
    /// The recorded HOST-CALL RESPONSES (E2h) — `(op, value-form)` pairs from a `(host-responses (respond
    /// E.op (: v T)) …)` clause, in call order. A case whose program delegates an effect to the host
    /// consumes these when it performs an operation; the gate driver passes each to `cdz-run
    /// --host-response`. Empty for a case that makes no host call.
    pub host_responses: Vec<(String, String)>,
    /// The recorded HOST-CALL sequence (E2h) — the dotted `E.op` names from a `(host-calls (call E.op
    /// arg…) …)` clause, in call order. The gate verifies the run's OBSERVED host calls against this, so
    /// a dropped/extra/reordered call is a Fail (not a false Pass on a matching return value). Empty for a
    /// case with no `(host-calls …)`.
    pub host_calls: Vec<String>,
    /// The WARNING diagnostics a case pins on a compiles-clean program (`(warns <CODE> (message "…")?)`
    /// clauses, zero or more) — each a `(code, optional message-substring)`. ORTHOGONAL to the primary
    /// outcome: a case asserts its `(output …)`/`(trap …)` AND that the compile emitted these warnings
    /// (a PRESENCE check, not exclusive). The portable-diagnostic-test capability (operator seq353 inc2);
    /// empty for a case with no `(warns …)`.
    pub warns: Vec<(String, Option<String>)>,
    /// An explicit WIT WORLD the case imposes on the guest (`(wit-world <world-sexpr>)`) — the general
    /// WIT-ABI shape where the export boundary is DECLARED, not synthesized from the guest. Stored as
    /// one-line s-expression text (like `program`); the gate driver converts it to a `wit-world` binary-AST
    /// artifact fed to `cdz compile`. `None` for the common synthesized-world case (byte-identical to before).
    pub wit_world: Option<String>,
    /// The interface a `(wit-world …)` case's guest exports under (`(component-name "cadenza:pkg/iface")`) —
    /// the shred's `component-name` text file, passed to the compiler as `--component-name` and used to
    /// qualify the run export as `<iface>#<export>`. `None` when no world is imposed.
    pub component_name: Option<String>,
    /// The imposed WIT world as BINARY AST (`codec::encode` of the `(world …)` subtree) — the shred's
    /// `wit-world.ast`, fed to the compiler EXACTLY as its native `wit-world:<name>=<path>` input with NO
    /// transform (the corpus `(world …)` form already IS the `world_schema_tree` shape the compiler reads;
    /// the `<name>` label is ignored — the world name is read from the artifact root). Built in the reader
    /// where the world's arena node is live (`clone_into`'d into a fresh arena, then encoded — no reparse).
    /// `None` for the common synthesized-world case (no `wit-world.ast` emitted).
    pub wit_world_ast: Option<Vec<u8>>,
    /// The recorded live-heap-cell count a `(live-objects N)` clause asserts AFTER the run — the
    /// heap-balance invariant (`N = 0` is no leak / no double-free) the memory-liveness cases pin. When
    /// set, the gate drives the run on the DEBUG-COUNTERS runtime with `cdz-run --report-live-objects` and
    /// compares the reported count to N (the shipped runtime reports 0 unconditionally, so the assertion
    /// has teeth only on the debug-counters runtime). ORTHOGONAL to the value/trap outcome: a case asserts
    /// its `(output …)`/`(trap …)` AND this balance. `None` for a case with no `(live-objects …)`.
    /// Under the OPT-OUT default a `None` on a HEAP-importing case enforces == 0 (no leak); on a no-heap
    /// case it is skipped.
    pub live_objects: Option<u32>,
    /// `true` iff the count came from a `(live-objects known-leak N)` OPT-OUT MARKER — N is a TOLERATED
    /// current leak grandfathered when the opt-out default landed (graded identically to a plain
    /// `(live-objects N)`; the flag records the intent so the marker set can be shrunk over time).
    pub live_objects_known_leak: bool,
    /// PER-CALL positional counts from a `(live-objects [known-leak] N1 N2 …)` clause with 2+ counts (one
    /// per call, in order) — `None` for the uniform/absent form. Expresses an arm-dependent balance a single
    /// count cannot (a leak that scales with input size). `live_objects` holds the FIRST count (uniform /
    /// direct-gate path); this carries the whole list for the per-call check (`design/DESIGN-corpus…`).
    pub live_objects_per_call: Option<Vec<u32>>,
    /// `true` iff the case authored a bare `(no-other-errors)` clause — a CASE-LEVEL no-cascade assertion:
    /// the compiler must emit NO error-severity diagnostic whose code is not one of the case's own
    /// `(error CODE …)` codes. Composes with the per-code `(count …)`: `(error CDZ0201) (no-other-errors)`
    /// pins "exactly this one code, nothing else". ERRORS ONLY (warnings are orthogonal — a separate
    /// `(no-other-warnings)` would split them if ever needed). `false` for a case without the clause.
    pub no_other_errors: bool,
    /// `(no-diagnostic "phrase")` clauses — a CASE-LEVEL, PROGRAM-SCOPED, CROSS-KIND message-ABSENCE
    /// assertion: the phrase must appear in NO diagnostic the compiler emits for this program — ANY kind
    /// (coded/uncoded error, decline, warning). Distinct from a trial's `(not "phrase")`, which is
    /// KIND-scoped to its own matched diagnostic's message (first-error / matched-warning) and so cannot
    /// assert the absence of a SIBLING diagnostic of another kind. Repeatable (each is an independent
    /// required-absence, AND). Empty for a case without the clause. (concierge-greenlit 2026-08-31; closes
    /// the cross-kind false-green gap `(not …)`/`(no-other-errors)` leave — e.g. "a mismatched compare
    /// must NOT also leak an uncoded heap-walk decline", "a malformed pattern must NOT also warn its dead
    /// binders unused".)
    pub no_diagnostic: Vec<String>,
}

/// One sibling LIBRARY module of a multi-file package case — its file name (the string an `(import
/// "name" …)` names it by) and its program text, normalized to the runnable `(do … )` shape like the
/// entry. A `(module "name" <prog>)` clause produces one of these.
pub struct Module {
    /// The file name (the `(import "name" …)` target).
    pub name: String,
    /// The module's program, as one-line s-expression text (same normalization as the entry program).
    pub program: String,
    /// The module's program as BINARY AST (`codec::encode`) — the shred's `module-<name>.ast`, built
    /// from the one normalized arena alongside `program` (no reparse).
    pub program_ast: Vec<u8>,
}

/// One PEER component of a CROSS-COMPONENT case (`DESIGN-cross-component-interop-rcdzc.md`): a separately-
/// compiled provider the case's entry (a consumer) binds across the live boundary. Unlike a [`Module`] (a
/// library file LINKED into the entry's one component), a peer is its OWN finished component — the entry
/// imports its interface via `(extern <iface> …)` and the gate composes them with `cdz-run --peer
/// <iface>=<path>` (`run_with_peers`). A `(peer "<iface>" <prog>)` clause produces one of these.
pub struct Peer {
    /// The interface the peer EXPORTS and the consumer imports under (`cadenza:<pkg>/<iface>`), e.g.
    /// `cadenza:math/api` — the `--peer <interface>=<path>` key.
    pub interface: String,
    /// The peer's program, one-line s-expression text (normalized like the entry — it is a full program
    /// exporting `interface`, compiled to its own component).
    pub program: String,
    /// The peer's program as BINARY AST (`codec::encode`) — the shred's `peer-<iface>.ast`, compiled to a
    /// standalone component by the gate/nix build (NOT linked as a module), built from the one normalized
    /// arena alongside `program` (no reparse).
    pub program_ast: Vec<u8>,
}

/// One (call, expected-result) pair of a case — a single run of the compiled program.
pub struct Trial {
    /// The `(call <export> <arg>…)` for this trial, or `None` to invoke the sole export with no args.
    pub call: Option<Call>,
    /// The recorded oracle result for this trial: `Output(value-form)`, `Error(code)`, or `Trap(reason)`.
    pub expect: Expect,
    /// The DIAGNOSTIC-QUALITY assertions a `(error …)` / `(warning …)` case pins BEYOND the code + message
    /// — a structural `(fix …)` / `(no-fix)` and an exact fault `(count N)` / `(once)`, authored NESTED
    /// inside the diagnostic clause (per-diagnostic attribution) and lifted to trial-level clauses in the
    /// shredded `test-run.ast`. `None` for the common code+message-only case (the vast majority). The
    /// grade side decodes the same clauses into `cdz_corpus_grade::DiagExpect` and grades them.
    pub diag: Option<DiagQuality>,
}

/// The DIAGNOSTIC-QUALITY facets a corpus `(error …)` / `(warning …)` clause pins — the authoring-side
/// mirror of `cdz_corpus_grade::DiagExpect` (the two crates share the sexp WIRE, not the type). All optional
/// so a case asserts only what it checks; an all-absent `DiagQuality` is never constructed (`None` instead).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiagQuality {
    /// A required structural fix on the diagnostic (`(fix …)`), or `None` to not constrain the fix.
    pub fix: Option<FixQuality>,
    /// The diagnostic must carry NO fix (`(no-fix)`); mutually exclusive with `fix`.
    pub no_fix: bool,
    /// The exact number of faults with this `(severity, code)` (`(count N)`, or `(once)` == `1`).
    pub count: Option<u32>,
}

impl DiagQuality {
    /// Whether this pins anything at all (else it is not emitted — the trial stays clause-free).
    pub fn is_empty(&self) -> bool {
        self.fix.is_none() && !self.no_fix && self.count.is_none()
    }
}

/// The asserted structural FIX a `(fix …)` clause pins — mirror of `cdz_corpus_grade::FixExpect`. Each
/// field optional (constrains only what the case cares about).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FixQuality {
    /// The structural edit kind (`(kind replace|insert|wrap|delete)`), or `None` to not constrain it.
    pub kind: Option<String>,
    /// How the fix's replacement text must match, or `None` to not constrain it.
    pub replacement: Option<ReplMatch>,
    /// The verified flag the fix must have (`(verified)` / `(unverified)`), or `None` to not constrain it.
    pub verified: Option<bool>,
}

/// How a `(fix …)` clause matches the fix's replacement text: `(replacement "r")` = exact, and
/// `(replacement-contains "s")` = substring. Mirror of `cdz_corpus_grade::ReplMatch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplMatch {
    Exact(String),
    Contains(String),
}

/// A `(call <export> <arg>…)` clause: run the named export with the given runtime arguments. This is
/// how a case exercises a program's runtime machinery rather than a constant-folded nullary entry —
/// the argument crosses the component boundary as a lifted value, so `(def (main (: x Int64)) (+ x 1))`
/// runs a real `local.get` + add instead of folding to a constant (`component-abi.md` §The Entry Is A
/// Plain Function — an entry is `input -> output`, its parameter type carrying a boundary representation).
pub struct Call {
    /// The export to invoke (e.g. `main`).
    pub export: String,
    /// The argument value-forms, in order — each the value's canonical text (e.g. `41`), stripped from
    /// its `(: <value> <Type>)` annotation. The runner coerces each to the export's declared parameter type.
    pub args: Vec<String>,
    /// A `(then <arg>…)` continuation: a SECOND call on the SAME closure handle this call minted, for a
    /// `borrow<t>` closure (which does NOT consume its handle, so it is repeatable — an `own<t>` closure
    /// would trap on the second call). `None` for the ordinary one-make/one-call form; `Some(args)` (which
    /// may be empty for a nullary second call) drives make ONCE then `call` TWICE on that handle, and the
    /// run renders both results as a tuple value-form `(tuple <r1> <r2>)`. This is how a case pins that a
    /// borrowed closure handle stays live across calls.
    pub second_call: Option<Vec<String>>,
    /// A `(drop)` clause: after the closure call(s), the host RESOURCE-DROPS the minted handle before the
    /// run reads its result / the heap balance. `call` BORROWS the handle (it does not consume it), so
    /// without an explicit drop the cell stays live until store teardown (a `(live-objects 1)` known leak);
    /// the drop fires the resource's `t-dtor`, reclaiming the cell, so a `(drop)` case can assert
    /// `(live-objects 0)`. Default `false` (hold the handle — the historical leaks-1 behavior). Wasm-only.
    pub drop_handle: bool,
    /// A `(call-method <member> …)` clause: the NAMED member to invoke on the value-resource the program
    /// produces (a runtime value crossing as a resource in the `cadenza:run/run` instance exposes
    /// compiler-emitted members — e.g. a `Bytes` value's `len : borrow<t> -> u32`, `is-empty`, `to-bytes`,
    /// besides `encode`). `None` for the ordinary make/encode escape or a closure call; `Some("len")` makes
    /// the value once then reaches that member and calls it with the handle (+ args). `(then …)` repeats
    /// the member on the SAME handle (a borrow method is repeatable); `(drop)` reclaims after. Wasm-only.
    pub method: Option<String>,
}

/// The recorded primary result of a case — exactly one per the corpus vocabulary.
pub enum Expect {
    /// `(output (: <value> <Type>))` — the value the run produces, as its canonical value-form text.
    Output(String),
    /// `(error <CODE>)` (or a `(compiler (error <CODE>))` for a provable-at-compile-time trap) — the
    /// diagnostic code the compiler must reject with.
    /// The second field is a list of load-bearing SUBSTRINGS of the diagnostic MESSAGE the corpus pins —
    /// one per `(message "phrase")` clause, REPEATABLE (`(error <CODE> (message "a") (message "b"))`): the
    /// gate requires the emitted diagnostic to contain EVERY one (AND). Empty = code-only (historical). This
    /// captures multi-part messages that name the rule AND each operand without shed (operator seq353 +
    /// capture-max-coverage).
    /// The third field pins message ABSENCE substrings (repeatable `(not "phrase")`, seq-29): the emitted
    /// diagnostic must NOT contain ANY of them (the complement of the required-substrings — required-absence,
    /// AND-d). Lets a case assert a message does not mention a phrase (e.g. no `"internal error"`) so a
    /// message-absence rust test can move into the corpus. Empty = no absence assertion.
    Error(String, Vec<String>, Vec<String>),
    /// `(warning <CODE>)` (or `(warning <CODE> (message "phrase"))`) — a NON-DENYING diagnostic: the
    /// compiler COMPILES the program (produces an artifact) AND emits a WARNING with this code. The
    /// severity companion of `Error` (which is a REJECTION — no artifact); a warning accompanies a produced
    /// component (e.g. a dead-trap or unused-binding lint). Pairs with a `(count N)` for the exact-warning-
    /// count tests. The second field pins message substrings (repeatable `(message …)`, ALL required), as with `Error`;
    /// the third pins message ABSENCE substrings (repeatable `(not "phrase")`, seq-29 — NONE may appear).
    Warning(String, Vec<String>, Vec<String>),
    /// `(trap "<reason>")` — the run halts with this reason.
    Trap(String),
    // NOTE: the bare `(declines)` expectation was REMOVED (operator directive, corpus (declines)=0):
    // a corpus rejection must now be coded `(error CDZxxxx)` and a should-work must be a TODO `(output V)`.
    // Parsing a `(declines)` clause is now a hard error (see `parse_case`), so this enum carries no
    // `Declines` variant — the acceptance path is gone and cannot be reintroduced without re-adding it here.
}

/// A platform-conformance case (`(platform-case "title" …)`) — the runtime/platform analog of a
/// compiler `(case …)`, in the separate `spec/platform/` tree (DESIGN-platform-conformance-suite.md,
/// operator seq358/seq359). Distinct GENRE from `Record`: a constellation of interacting reducer
/// SESSIONS is driven from ONE kick-off event to a fixpoint (no scripted response tape), and the case
/// asserts the emitted effects/messages + each session's end-state. The reader parses + normalizes it;
/// v-platform-conformance's xtask `run_platform_case` grade path drives the fixpoint and compares.
#[derive(Debug)]
pub struct PlatformRecord {
    /// The case title (the first string child of `(platform-case "…" …)`).
    pub title: String,
    /// The `(doc "…")` prose, if present — documentation only.
    pub doc: Option<String>,
    /// The `(session <alias> (reducer <prog>) (serves <family>…)?)` blocks, in declaration order. Each
    /// carries its alias, its reducer program (normalized one-line like a `(case (input …))` program —
    /// the reader does NOT compile it), and the effect families it serves as a handler.
    pub sessions: Vec<PlatformSession>,
    /// `(child <alias> (reducer <prog>))` blocks — registered-but-not-kicked SPAWNABLE child reducers, in
    /// declaration order, as `(alias, normalized-program)`. Structurally like a `session` (the reducer
    /// program is normalized one-line, NOT compiled here) but a child declares a spawn target rather than
    /// a live seeded session — the grade side compiles it and hands the runner `--child-reducer alias=path`.
    pub children: Vec<(String, String)>,
    /// The `(kickoff <alias> (inbound <family> <value>))` seed events, in DECLARATION ORDER (1+). A
    /// single-kickoff case (the common one) carries exactly one; the multi-kickoff fan-in shape carries
    /// several, all enqueued in this order before the FIFO drive loop (single-fixpoint-deterministic,
    /// just N seeds instead of 1). At least one is required — the fixpoint needs a seed.
    pub kickoffs: Vec<Kickoff>,
    /// Ordered `(expect-effects (effect (from <a>) (family <f>) <value>? (schema-hash <tok>)?)…)` — each
    /// emitted effect the run must produce, in stream order (order-verified, like `host_calls`). Value-form
    /// optional (an effect with no payload omits it). Optional `(schema-hash present)` sub-clause pins the
    /// phase-3 reify→descriptor→hash round-trip (present-vs-absent, not a literal hash).
    pub expect_effects: Vec<ExpectEffect>,
    /// Ordered `(expect-messages (message (from <a>) (to <b>) (family <f>) <value>)…)` — the inter-session
    /// messages the run must deliver, in stream order.
    pub expect_messages: Vec<ExpectMessage>,
    /// `(expect-delivery-failure (from <a>) (to <b>)…)` — messages whose delivery must FAIL (e.g. to a
    /// closed session), as `(from, to)` alias pairs.
    pub expect_delivery_failures: Vec<(String, String)>,
    /// Per-alias end-state key/value assertions: `(end-state <alias> (kv <key> <value>)…)` → one
    /// `(alias, key, value-form)` each.
    pub end_kv: Vec<(String, String, String)>,
    /// Per-alias end-state status: `(end-state <alias> … (status <state>))` → one `(alias, status)` each
    /// (status in active/quiescent/stalled/closed).
    pub end_status: Vec<(String, String)>,
    /// Per-alias end-state close-outcome: `(end-state <alias> … (close-outcome <kind>))` → one
    /// `(alias, kind)` each, `<kind>` a bare token `Success`|`Failure`. Distinguishes HOW a self-closed
    /// session closed (both report `status closed` otherwise). The grade side compares this against the
    /// runner's observed `end-close-outcome` line (string eq, like `status`).
    pub close_outcome: Vec<(String, String)>,
    /// Per-alias `(events-processed <alias> <n>)` — the total processed-log length the session must reach
    /// (grades `Session::event_count()`).
    pub events_processed: Vec<(String, String)>,
    /// Optional `(expect-fault <kind>)` — the run must FAULT with a stderr marker containing `<kind>`
    /// (a bare token like `SettleUnbounded`), rather than exit cleanly. Absent = today's behavior (a
    /// clean run graded normally, any nonzero exit a plain Fail). The grade-side arm lives in xtask
    /// (v-platform-conformance's domain); the reader only carries the token.
    pub expect_fault: Option<String>,
    /// Top-level `(recover-check <alias>)` clauses (0-or-more) — each names a session to
    /// replay-equals-recover check (I4: a recovered session must reach the same state as a
    /// full replay). Absent = no line rendered = byte-identical. The grade-side arm (assert
    /// recover-equal) lives in xtask (v-platform-conformance's domain); the reader only carries
    /// the aliases.
    pub recover_checks: Vec<String>,
}

/// One `(session <alias> (reducer <prog>) (serves <family>…))` block of a platform case.
#[derive(Debug)]
pub struct PlatformSession {
    /// The session's alias (how the kickoff/effects/messages/end-state address it).
    pub alias: String,
    /// The reducer program, normalized one-line (same normalization as a `(case (input …))` program).
    pub program: String,
    /// The effect families this session serves as a handler (zero or more), in order.
    pub serves: Vec<String>,
}

/// The single kick-off event of a platform case: an inbound `family` carrying `value` delivered to
/// session `alias` to seed the fixpoint.
#[derive(Debug)]
pub struct Kickoff {
    pub alias: String,
    pub inbound: String,
    pub value: String,
}

/// One expected emitted effect: session `from` performed effect `family`, optionally carrying `value`.
///
/// `schema_hash` is the optional phase-3 schema-hash observation: `Some("present")` when the case asserts
/// `(schema-hash present)` — i.e. the reify emitted a schema_descriptor and the kernel hashed it to a
/// non-None `EffectRequest.schema_hash`. This is a PRESENT-vs-absent assertion, not a literal hash (a literal
/// is brittle + is the descriptor-encoding owner's to pin); `None` = the case does not observe it (absent →
/// byte-identical stream). Rendered as a separate `expect-effect-schema-hash` line, keyed on `(from, family)`.
#[derive(Debug)]
pub struct ExpectEffect {
    pub from: String,
    pub family: String,
    pub value: Option<String>,
    pub schema_hash: Option<String>,
}

/// One expected inter-session message: `from` sent `to` an effect `family` carrying `value`.
#[derive(Debug)]
pub struct ExpectMessage {
    pub from: String,
    pub to: String,
    pub family: String,
    pub value: String,
}

/// Extract a `(message "phrase")` sibling clause's string from a clause's tail, if present — the
/// EVERY diagnostic-message substring pin (operator seq353) shared by `(error …)`/`(warning …)` —
/// one per `(message STR)` child, in order, REPEATABLE (all AND-required at grade). Empty
/// when no well-formed `(message STR)` child is present (code-only).
fn message_clauses(a: &Arenas, tail: &[StructId]) -> Vec<String> {
    tail.iter()
        .filter_map(|&child| {
            a.as_form(child, "message")
                .and_then(|t| t.first().copied())
                .and_then(|id| string_leaf(a, id))
        })
        .collect()
}

/// Scan a diagnostic clause's `tail` for `(not "phrase")` sub-forms (seq-29) — the message-ABSENCE
/// complement of [`message_clauses`]. Each yields a substring the emitted diagnostic must NOT contain
/// (required-absence, AND-d with the positive `(message …)` substrings). Repeatable; empty when none.
fn not_message_clauses(a: &Arenas, tail: &[StructId]) -> Vec<String> {
    tail.iter()
        .filter_map(|&child| {
            a.as_form(child, "not")
                .and_then(|t| t.first().copied())
                .and_then(|id| string_leaf(a, id))
        })
        .collect()
}

/// Parse the DIAGNOSTIC-QUALITY facets NESTED inside a `(error …)` / `(warning …)` clause's `tail` —
/// `(fix …)`, `(no-fix)`, `(count N)`, `(once)` (== `count 1`) — into a [`DiagQuality`], or `None` when the
/// clause pins none (the common code+message-only form). Nesting (not bare case-level siblings) is required
/// so each facet attributes to ITS diagnostic in a multi-diagnostic case.
fn diag_clause(a: &Arenas, tail: &[StructId]) -> Option<DiagQuality> {
    let mut d = DiagQuality::default();
    for &child in tail {
        match a.head_name(child) {
            Some("fix") => d.fix = Some(fix_clause(a, child)),
            Some("no-fix") => d.no_fix = true,
            Some("count") => {
                // `2` parses as a NUMBER leaf (not a name/string), so render it to text before parsing.
                d.count = a
                    .as_form(child, "count")
                    .and_then(|t| t.first().copied())
                    .and_then(|id| sexpr::print_from(a, id).trim().parse::<u32>().ok());
            }
            Some("once") => d.count = Some(1),
            _ => {}
        }
    }
    (!d.is_empty()).then_some(d)
}

/// Parse a `(fix (kind K)? (replacement "r")|(replacement-contains "s")? (verified|unverified)?)` clause
/// into a [`FixQuality`]. Each sub-clause optional; `(replacement …)` is EXACT, `(replacement-contains …)`
/// SUBSTRING (the later wins if both appear — an authoring slip).
fn fix_clause(a: &Arenas, id: StructId) -> FixQuality {
    let mut fx = FixQuality::default();
    for &child in a.as_form(id, "fix").unwrap_or(&[]) {
        match a.head_name(child) {
            Some("kind") => {
                fx.kind = a
                    .as_form(child, "kind")
                    .and_then(|t| t.first().copied())
                    .and_then(|cid| {
                        a.as_name(cid)
                            .map(str::to_string)
                            .or_else(|| string_leaf(a, cid))
                    });
            }
            Some("replacement") => {
                fx.replacement = a
                    .as_form(child, "replacement")
                    .and_then(|t| t.first().copied())
                    .and_then(|cid| string_leaf(a, cid))
                    .map(ReplMatch::Exact);
            }
            Some("replacement-contains") => {
                fx.replacement = a
                    .as_form(child, "replacement-contains")
                    .and_then(|t| t.first().copied())
                    .and_then(|cid| string_leaf(a, cid))
                    .map(ReplMatch::Contains);
            }
            Some("verified") => fx.verified = Some(true),
            Some("unverified") => fx.verified = Some(false),
            _ => {}
        }
    }
    fx
}

/// Parse a corpus file's `text` into records. Returns an error only if the file itself does not
/// parse as s-expressions; a malformed individual case is reported as an error record inline.
pub fn read(text: &str) -> Result<Vec<Record>, String> {
    let arenas = sexpr::read_all(text).map_err(|e| format!("corpus parse error: {}", e.0))?;
    // `read_all` wraps every top-level form under a synthetic `(do …)`; the cases are its children.
    let top = match arenas.get(arenas.root) {
        cadenza_syntax::ast::Struct::List(items) => &items[1..], // skip the synthetic `do` head
        _ => return Ok(Vec::new()),
    };
    let mut records = Vec::new();
    for &top_id in top {
        // A leading `;`/`//` comment above a top-level `(case …)` reifies to a `(comment "…" (case …))`
        // wrapper (comment-preservation, seq-285); peel it so the case head is found. Read-only — the
        // comment stays in the tree for the fmt/round-trip path, it just does not hide the case here.
        let case_id = arenas.peel_comments(top_id);
        if arenas.head_name(case_id) == Some("case") {
            match parse_case(&arenas, case_id) {
                Ok(r) => records.push(r),
                Err(e) => return Err(e),
            }
        }
    }
    Ok(records)
}

/// Parse a PLATFORM-conformance file's `text` into [`PlatformRecord`]s — the separate `spec/platform/`
/// genre (operator seq358/seq359). Dispatches on the `(platform-case …)` head, exactly as [`read`]
/// dispatches on `(case …)`; a non-`platform-case` top-level form is skipped, so a file may mix (though
/// in practice platform files are homogeneous). Errors only if the file does not parse as s-expressions;
/// a malformed individual case is a hard error (fail loud, like `read`).
pub fn read_platform(text: &str) -> Result<Vec<PlatformRecord>, String> {
    let arenas = sexpr::read_all(text).map_err(|e| format!("corpus parse error: {}", e.0))?;
    let top = match arenas.get(arenas.root) {
        cadenza_syntax::ast::Struct::List(items) => &items[1..], // skip the synthetic `do` head
        _ => return Ok(Vec::new()),
    };
    let mut records = Vec::new();
    for &top_id in top {
        // Peel a leading comment wrapper (see [`read`]) so a `;`/`//`-commented platform case is found.
        let case_id = arenas.peel_comments(top_id);
        if arenas.head_name(case_id) == Some("platform-case") {
            records.push(parse_platform_case(&arenas, case_id)?);
        }
    }
    Ok(records)
}

/// Render a coded diagnostic expectation (`error`/`warning`) to the flat manifest: the severity
/// `kind` token, the `code`, then each pinned `message` as ` (message "…")` and each absence pin as
/// ` (not "…")` — the exact surface xtask's `split_message_clause` parses. Shared by the `Expect::Error`
/// and `Expect::Warning` arms, which differ only in the `kind` token.
fn push_diag(out: &mut String, kind: &str, code: &str, message: &[String], not_message: &[String]) {
    out.push_str(kind);
    out.push(' ');
    out.push_str(code);
    for m in message {
        out.push_str(" (message \"");
        out.push_str(m);
        out.push_str("\")");
    }
    for n in not_message {
        out.push_str(" (not \"");
        out.push_str(n);
        out.push_str("\")");
    }
}

/// Render `records` to the flat record stream (see the module docs for the format).
pub fn render(records: &[Record]) -> String {
    let mut out = String::new();
    for r in records {
        out.push_str("case\t");
        out.push_str(&r.description);
        out.push('\n');
        out.push_str("program\t");
        out.push_str(&r.program);
        out.push('\n');
        // Sibling LIBRARY modules (multi-file package case): one `module\t<name>\t<program>` line each,
        // after the entry program and before the trials. Absent for a single-file case (the common
        // shape stays byte-identical). Ordered as written, so the record stream is deterministic.
        for m in &r.modules {
            out.push_str("module\t");
            out.push_str(&m.name);
            out.push('\t');
            out.push_str(&m.program);
            out.push('\n');
        }
        // PEER components (cross-component case): one `peer\t<interface>\t<program>` line each, after any
        // modules. The gate compiles each to its own component and composes via `--peer <iface>=<path>`.
        // Absent for a single-component case (byte-identical to before).
        for p in &r.peers {
            out.push_str("peer\t");
            out.push_str(&p.interface);
            out.push('\t');
            out.push_str(&p.program);
            out.push('\n');
        }
        // One group of lines per TRIAL: its `call`/`arg` lines (if any) then its `expect`, which ends
        // the trial. A single-trial case emits exactly the historical `call?`/`arg*`/`expect` shape.
        for trial in &r.trials {
            if let Some(call) = &trial.call {
                // A `(call-method <member>)` case has no export (the program's sole producer makes the
                // value-resource); it emits a `call-method\t<member>` line the driver reaches after make,
                // instead of the `call\t<export>` line. The `arg` lines are the member's arguments either way.
                if let Some(member) = &call.method {
                    out.push_str("call-method\t");
                    out.push_str(member);
                    out.push('\n');
                } else {
                    out.push_str("call\t");
                    out.push_str(&call.export);
                    out.push('\n');
                }
                for arg in &call.args {
                    out.push_str("arg\t");
                    out.push_str(arg);
                    out.push('\n');
                }
                // A `(then …)` continuation (two-call-on-one-handle): a `then-call\t<n>` marker line (n =
                // the second call's arg count, so an empty `(then)` still records its presence) followed by
                // one `then-arg\t<value>` line per argument. Absent for the ordinary one-call form (byte-
                // identical to before).
                if let Some(second) = &call.second_call {
                    out.push_str("then-call\t");
                    out.push_str(&second.len().to_string());
                    out.push('\n');
                    for arg in second {
                        out.push_str("then-arg\t");
                        out.push_str(arg);
                        out.push('\n');
                    }
                }
                // A `(drop)` clause: a bare `drop-handle` marker line so the gate driver resource-drops the
                // minted closure handle before reading the result / heap balance. Absent by default.
                if call.drop_handle {
                    out.push_str("drop-handle\t1\n");
                }
            }
            out.push_str("expect\t");
            match &trial.expect {
                Expect::Output(v) => {
                    out.push_str("output ");
                    out.push_str(v);
                }
                // `error CODE`, plus ` (message "phrase")` VERBATIM when the case pins a message — the
                // exact surface xtask's split_message_clause parses (operator seq353). Absent → byte-
                // identical to the historical `error CODE` line (back-compat).
                Expect::Error(code, message, not_message) => {
                    push_diag(&mut out, "error", code, message, not_message);
                }
                // `warning CODE`, plus ` (message "phrase")` — mirrors `error` (the non-denying severity
                // companion). The diagnostic-quality facets ride the sexp `test-run.ast` grade path, not this
                // flat direct-gate manifest (which grades only code + message today). A `(not "phrase")`
                // absence pin (seq-29) is rendered too but graded only on the sexp path (xtask ignores it).
                Expect::Warning(code, message, not_message) => {
                    push_diag(&mut out, "warning", code, message, not_message);
                }
                Expect::Trap(reason) => {
                    out.push_str("trap ");
                    out.push_str(reason);
                }
            }
            out.push('\n');
        }
        // HOST-CALL RESPONSES (E2h): one `host-response\t<op>\t<value>` line each, in call order. The
        // gate driver forwards each to `cdz-run --host-response op=value`. Absent for a non-host case.
        for (op, value) in &r.host_responses {
            out.push_str("host-response\t");
            out.push_str(op);
            out.push('\t');
            out.push_str(value);
            out.push('\n');
        }
        // HOST-CALL sequence (E2h): one `host-call\t<op>` line each, in call order — the ordered host
        // operations the run must make. The gate verifies the run's observed calls against these.
        for op in &r.host_calls {
            out.push_str("host-call\t");
            out.push_str(op);
            out.push('\n');
        }
        // WARNING pins (operator seq353 inc2): one `warns\t<CODE>` or `warns\t<CODE> (message "phrase")`
        // line each — the compile warnings the case asserts (a presence check, orthogonal to the outcome).
        for (code, message) in &r.warns {
            out.push_str("warns\t");
            out.push_str(code);
            if let Some(m) = message {
                out.push_str(" (message \"");
                out.push_str(m);
                out.push_str("\")");
            }
            out.push('\n');
        }
        // An explicit WIT world the case imposes (general WIT-ABI shape): `wit-world\t<world-sexpr>` (one
        // line, like `program`) + `component-name\t<iface>`. The gate driver converts the world to a
        // `wit-world` binary-AST artifact for `cdz compile --component-name` + qualifies the run `--call`.
        // Absent for a synthesized-world case (byte-identical to before).
        if let Some(w) = &r.wit_world {
            out.push_str("wit-world\t");
            out.push_str(w);
            out.push('\n');
        }
        if let Some(cn) = &r.component_name {
            out.push_str("component-name\t");
            out.push_str(cn);
            out.push('\n');
        }
        // `live-objects\t<N>` — the post-run CLEAN residual the case asserts on the debug-counters runtime
        // (the reachable-return cell count; N=0 = fully reclaimed). Orthogonal to the value/trap outcome.
        // A KNOWN-LEAK case (seq-15 pure-binary marker) renders as `live-objects\tknown-leak` with NO count —
        // it is accepted-as-leaking and NOT count-checked (magnitude does not matter). Absent for a case with
        // no `(live-objects …)`.
        if r.live_objects_known_leak {
            out.push_str("live-objects\tknown-leak\n");
        } else if let Some(n) = r.live_objects {
            out.push_str("live-objects\t");
            // Per-call positional CLEAN residuals render tab-separated (`live-objects\t0\t0\t0`); a uniform
            // residual renders as the single `live-objects\t<N>`.
            match &r.live_objects_per_call {
                Some(counts) => {
                    let joined: Vec<String> = counts.iter().map(u32::to_string).collect();
                    out.push_str(&joined.join("\t"));
                }
                None => out.push_str(&n.to_string()),
            }
            out.push('\n');
        }
        out.push_str("---\n");
    }
    out
}

/// Convenience: read + render in one step (what the `corpus` command emits).
pub fn to_records(text: &str) -> Result<String, String> {
    Ok(render(&read(text)?))
}

/// Render `PlatformRecord`s to the flat record stream — the `spec/platform/` genre analog of [`render`].
/// FIXED-ARITY tab lines + one line per element of an ordered list (mirrors `host-call`/`module`), so
/// v-platform-conformance's `run_platform_case` parses with the same split-on-tab loop, no s-expr parser:
///   `platform-case\t<title>` · `doc\t<text>`? · `session\t<alias>\t<program>` (1+) ·
///   `serves\t<alias>\t<family>` (0+) · `child\t<alias>\t<program>` (0+) ·
///   `kickoff\t<alias>\t<inbound>\t<value>` (1+, declaration order) ·
///   `expect-effect\t<from>\t<family>[\t<value>]` (0+, order) ·
///   `expect-message\t<from>\t<to>\t<family>\t<value>` (0+, order) ·
///   `expect-delivery-failure\t<from>\t<to>` (0+) · `end-kv\t<alias>\t<key>\t<value>` (0+) ·
///   `end-status\t<alias>\t<status>` (0+) · `end-close-outcome\t<alias>\t<kind>` (0+) ·
///   `events-processed\t<alias>\t<n>` (0+) ·
///   `expect-fault\t<kind>` (0-or-1) · `recover-check\t<alias>` (0+, declaration order) ·
///   `---` terminator.
pub fn render_platform(records: &[PlatformRecord]) -> String {
    let mut out = String::new();
    for r in records {
        out.push_str("platform-case\t");
        out.push_str(&r.title);
        out.push('\n');
        // NOTE: `doc` is documentation-only and is DELIBERATELY NOT part of the record stream — the
        // grader ignores it (xtask `"doc" => {}`), matching the compiler genre where `(doc …)` is prose
        // dropped from `render`. The parsed `PlatformRecord.doc` field is kept (harmless, available to
        // any doc-aware tool) but is not rendered into the graded stream.
        for s in &r.sessions {
            out.push_str("session\t");
            out.push_str(&s.alias);
            out.push('\t');
            out.push_str(&s.program);
            out.push('\n');
            for family in &s.serves {
                out.push_str("serves\t");
                out.push_str(&s.alias);
                out.push('\t');
                out.push_str(family);
                out.push('\n');
            }
        }
        for (alias, program) in &r.children {
            out.push_str("child\t");
            out.push_str(alias);
            out.push('\t');
            out.push_str(program);
            out.push('\n');
        }
        for k in &r.kickoffs {
            out.push_str("kickoff\t");
            out.push_str(&k.alias);
            out.push('\t');
            out.push_str(&k.inbound);
            out.push('\t');
            out.push_str(&k.value);
            out.push('\n');
        }
        for e in &r.expect_effects {
            out.push_str("expect-effect\t");
            out.push_str(&e.from);
            out.push('\t');
            out.push_str(&e.family);
            if let Some(v) = &e.value {
                out.push('\t');
                out.push_str(v);
            }
            out.push('\n');
        }
        // Phase-3 schema-hash present-vs-absent pin: a separate line per effect that asserts it, keyed on
        // `(from, family)`. Absent on every effect ⇒ no lines ⇒ stream byte-identical to pre-clause cases.
        for e in &r.expect_effects {
            if let Some(sh) = &e.schema_hash {
                out.push_str("expect-effect-schema-hash\t");
                out.push_str(&e.from);
                out.push('\t');
                out.push_str(&e.family);
                out.push('\t');
                out.push_str(sh);
                out.push('\n');
            }
        }
        for m in &r.expect_messages {
            out.push_str("expect-message\t");
            out.push_str(&m.from);
            out.push('\t');
            out.push_str(&m.to);
            out.push('\t');
            out.push_str(&m.family);
            out.push('\t');
            out.push_str(&m.value);
            out.push('\n');
        }
        for (from, to) in &r.expect_delivery_failures {
            out.push_str("expect-delivery-failure\t");
            out.push_str(from);
            out.push('\t');
            out.push_str(to);
            out.push('\n');
        }
        for (alias, key, value) in &r.end_kv {
            out.push_str("end-kv\t");
            out.push_str(alias);
            out.push('\t');
            out.push_str(key);
            out.push('\t');
            out.push_str(value);
            out.push('\n');
        }
        for (alias, status) in &r.end_status {
            out.push_str("end-status\t");
            out.push_str(alias);
            out.push('\t');
            out.push_str(status);
            out.push('\n');
        }
        for (alias, kind) in &r.close_outcome {
            out.push_str("end-close-outcome\t");
            out.push_str(alias);
            out.push('\t');
            out.push_str(kind);
            out.push('\n');
        }
        for (alias, n) in &r.events_processed {
            out.push_str("events-processed\t");
            out.push_str(alias);
            out.push('\t');
            out.push_str(n);
            out.push('\n');
        }
        if let Some(kind) = &r.expect_fault {
            out.push_str("expect-fault\t");
            out.push_str(kind);
            out.push('\n');
        }
        for alias in &r.recover_checks {
            out.push_str("recover-check\t");
            out.push_str(alias);
            out.push('\n');
        }
        out.push_str("---\n");
    }
    out
}

/// Convenience: read + render platform cases in one step (the `spec/platform/` genre analog of
/// [`to_records`]).
pub fn to_platform_records(text: &str) -> Result<String, String> {
    Ok(render_platform(&read_platform(text)?))
}

/// Whether `text` is a PLATFORM-genre corpus file — i.e. its first top-level form is a `(platform-case
/// …)` rather than a compiler `(case …)`. The two genres are disjoint (a file is homogeneous), so the
/// leading form's head decides. Used by the `records` CLI to route to [`read_platform`]. A file that
/// does not parse, or has no forms, is NOT platform (falls through to the normal reader, which reports
/// the parse error). Cheap: reads the s-exprs but inspects only the first child.
pub fn is_platform_genre(text: &str) -> bool {
    let Ok(arenas) = sexpr::read_all(text) else {
        return false;
    };
    match arenas.get(arenas.root) {
        cadenza_syntax::ast::Struct::List(items) => items
            .get(1) // [0] is the synthetic `do` head
            .is_some_and(|&first| arenas.head_name(first) == Some("platform-case")),
        _ => false,
    }
}

/// Parse one `(case …)` occurrence into a [`Record`].
fn parse_case(a: &Arenas, case_id: StructId) -> Result<Record, String> {
    let items = match a.get(case_id) {
        cadenza_syntax::ast::Struct::List(items) => items,
        _ => return Err("case is not a list".into()),
    };
    // `(case "<desc>" <clause>…)` — the description is the first string child (peel a comment wrapper
    // that a `;`/`//` above it would introduce under comment-preservation).
    let description = items
        .get(1)
        .and_then(|&id| string_leaf(a, a.peel_comments(id)))
        .ok_or("case has no description string")?;

    let mut input: Option<StructId> = None;
    let mut modules: Vec<Module> = Vec::new();
    let mut peers: Vec<Peer> = Vec::new();
    let mut host_responses: Vec<(String, String)> = Vec::new();
    let mut host_calls: Vec<String> = Vec::new();
    let mut warns: Vec<(String, Option<String>)> = Vec::new();
    let mut wit_world: Option<String> = None;
    let mut wit_world_id: Option<StructId> = None;
    let mut component_name: Option<String> = None;
    let mut live_objects: Option<u32> = None;
    let mut live_objects_known_leak = false;
    let mut live_objects_per_call: Option<Vec<u32>> = None;
    let mut no_other_errors = false;
    let mut no_diagnostic: Vec<String> = Vec::new();
    // Trials accumulate as the clauses are walked: a `(call …)` sets the PENDING call, and the next
    // result clause (`output`/`error`/`trap`) CLOSES a trial pairing that pending call with the result.
    // A result with no preceding `(call …)` is a no-call trial. This lets a case INTERLEAVE several
    // `(call …) (output …)` pairs — each result closes one trial — while a single-result case (the
    // common shape) yields exactly one trial. A `(compiler (error …))` overrides the current trial's
    // result with the compile-time rejection (it accompanies a dynamic `(trap …)`).
    let mut trials: Vec<Trial> = Vec::new();
    let mut pending_call: Option<Call> = None;

    for &clause in &items[2..] {
        // Peel a leading `;`/`//` comment wrapper around a case clause so it dispatches on the clause head
        // (comment-preservation). The comment stays in the tree for printing; only clause CONTENTS (an
        // `(input …)` program) keep their internal comments — those are handed to the compiler, which peels.
        let clause = a.peel_comments(clause);
        match a.head_name(clause) {
            Some("input") => {
                input = a.as_form(clause, "input").and_then(|t| t.first().copied());
            }
            Some("module") => {
                // `(module "name" <prog>)` — a sibling LIBRARY file of a multi-file package case. Its
                // NAME is a string literal (the `(import "name" …)` target); its program is normalized
                // like the entry. NOTE the string-name shape is distinct from a single-module `(module
                // NAME def…)` INPUT (bare-name head), which `normalize_program` handles as the entry.
                if let Some(tail) = a.as_form(clause, "module")
                    && let Some(&name_id) = tail.first()
                    && let Some(name) = string_leaf(a, name_id)
                    && let Some(&prog) = tail.get(1)
                {
                    let (program, program_ast) = normalize_program_text_and_ast(a, prog);
                    modules.push(Module {
                        name,
                        program,
                        program_ast,
                    });
                }
            }
            Some("peer") => {
                // `(peer "<iface>" <prog>)` — a separately-compiled PEER provider the entry binds across the
                // live boundary (cross-component). The INTERFACE is a string literal (the `(extern <iface>
                // …)` target + the `--peer <iface>=<path>` key); its program is normalized like the entry (a
                // full program exporting `iface`, compiled to its OWN component — NOT linked like a module).
                if let Some(tail) = a.as_form(clause, "peer")
                    && let Some(&iface_id) = tail.first()
                    && let Some(interface) = string_leaf(a, iface_id)
                    && let Some(&prog) = tail.get(1)
                {
                    let (program, program_ast) = normalize_program_text_and_ast(a, prog);
                    peers.push(Peer {
                        interface,
                        program,
                        program_ast,
                    });
                }
            }
            Some("call") => {
                // `(call <export> <arg>…)` — the export to invoke plus its runtime arguments. The
                // export is the first child (a name); each remaining child is an argument value-form,
                // reduced to its bare value text (the runner coerces it to the declared param type).
                // Sets the PENDING call, paired with the result clause that follows.
                if let Some(tail) = a.as_form(clause, "call")
                    && let Some(&export_id) = tail.first()
                    && let Some(export) = a.as_name(export_id)
                {
                    let args = tail[1..].iter().map(|&arg| value_of(a, arg)).collect();
                    pending_call = Some(Call {
                        export: export.to_string(),
                        args,
                        second_call: None,
                        drop_handle: false,
                        method: None,
                    });
                }
            }
            Some("call-method") => {
                // `(call-method <member> <arg>…)` — invoke a NAMED member on the value-resource the program
                // produces (a runtime value crosses as a resource in `cadenza:run/run`, exposing members like
                // `len`/`is-empty`/`to-bytes` besides `encode`). No export name: the program's sole value-
                // producer makes the resource (like the `encode` escape), then the driver reaches `<member>`.
                // The args are the member's arguments. `(then …)`/`(drop)` compose (repeatable / reclaim).
                if let Some(tail) = a.as_form(clause, "call-method")
                    && let Some(&member_id) = tail.first()
                    && let Some(member) = a.as_name(member_id)
                {
                    let args = tail[1..].iter().map(|&arg| value_of(a, arg)).collect();
                    pending_call = Some(Call {
                        export: String::new(),
                        args,
                        second_call: None,
                        drop_handle: false,
                        method: Some(member.to_string()),
                    });
                }
            }
            Some("drop") => {
                // `(drop)` — after the closure call(s), the host resource-drops the minted handle before the
                // run reads the result / heap balance, reclaiming the closure cell (so a `(live-objects 0)`
                // case can pin release). Sets a flag on the pending call; ignored with no pending call.
                if a.as_form(clause, "drop").is_some()
                    && let Some(call) = pending_call.as_mut()
                {
                    call.drop_handle = true;
                }
            }
            Some("then") => {
                // `(then <arg>…)` — a SECOND call on the SAME handle the PENDING `(call …)` minted (a
                // `borrow<t>` closure keeps its handle live across calls). Attaches the second call's
                // arguments to the pending call; the driver makes ONE handle, calls it twice, and renders
                // the pair as a tuple. A bare `(then)` (no args) drives a nullary second call. Ignored if
                // there is no pending call (a `(then …)` must follow a `(call …)` in the same trial).
                if let Some(tail) = a.as_form(clause, "then")
                    && let Some(call) = pending_call.as_mut()
                {
                    call.second_call = Some(tail.iter().map(|&arg| value_of(a, arg)).collect());
                }
            }
            Some("output") => {
                // `(output (: <value> <Type>))` — closes a trial. Record the value-form's canonical text
                // (the whole `(: value Type)`); the driver compares against however the run renders.
                if let Some(v) = a
                    .as_form(clause, "output")
                    .and_then(|t| t.first().copied())
                    .map(|form| value_form_text(a, form))
                {
                    trials.push(Trial {
                        call: pending_call.take(),
                        expect: Expect::Output(v),
                        diag: None,
                    });
                }
            }
            Some("error") => {
                // `(error <CODE>)` or `(error <CODE> (message "phrase") (fix …)? (no-fix)? (count N)?)` —
                // closes a trial with a compile-time rejection code, optionally pinning a message substring
                // + the diagnostic-quality facets (nested inside the clause).
                if let Some(tail) = a.as_form(clause, "error")
                    && let Some(code) = tail
                        .first()
                        .copied()
                        .and_then(|id| a.as_name(id).map(str::to_string))
                {
                    let message = message_clauses(a, tail);
                    let not_message = not_message_clauses(a, tail);
                    trials.push(Trial {
                        call: pending_call.take(),
                        expect: Expect::Error(code, message, not_message),
                        diag: diag_clause(a, tail),
                    });
                }
            }
            Some("warning") => {
                // `(warning <CODE>)` / `(warning <CODE> (message "phrase") (fix …)? (count N)?)` — a
                // NON-DENYING diagnostic: the compiler COMPILES + emits a warning with this code. Same
                // facet grammar as `(error …)`.
                if let Some(tail) = a.as_form(clause, "warning")
                    && let Some(code) = tail
                        .first()
                        .copied()
                        .and_then(|id| a.as_name(id).map(str::to_string))
                {
                    let message = message_clauses(a, tail);
                    let not_message = not_message_clauses(a, tail);
                    trials.push(Trial {
                        call: pending_call.take(),
                        expect: Expect::Warning(code, message, not_message),
                        diag: diag_clause(a, tail),
                    });
                }
            }
            Some("trap") => {
                // `(trap "<reason>")` — closes a trial with a runtime trap.
                if let Some(reason) = a
                    .as_form(clause, "trap")
                    .and_then(|t| t.first().copied())
                    .and_then(|id| string_leaf(a, id))
                {
                    trials.push(Trial {
                        call: pending_call.take(),
                        expect: Expect::Trap(reason),
                        diag: None,
                    });
                }
            }
            Some("declines") => {
                // REMOVED (operator directive; corpus (declines)=0): a bare `(declines)` marker is no longer
                // accepted in the corpus. A rejection must be coded `(error CDZxxxx)` and a should-work must be
                // a TODO `(output V)`. This hard error is the removed acceptance path — it cannot be
                // reintroduced without re-adding an `Expect::Declines` variant + this parse arm.
                return Err(
                    "(declines) is no longer supported in the corpus: a rejection must be coded \
                     `(error CDZxxxx)` and a should-work must be a TODO `(output V)`. The bare-decline \
                     marker was removed (operator directive; corpus (declines)=0)."
                        .to_string(),
                );
            }
            Some("compiler") => {
                // `(compiler (error <CODE>))` — a provable-at-compile-time rejection accompanying a
                // dynamic `(trap …)`. The compiler's recorded outcome is the rejection, so it OVERRIDES
                // the most recently closed trial's result (the `(trap …)` that precedes it in the same
                // trial). If it appears before any result, it opens+closes a trial on its own.
                if let Some(inner) = a
                    .as_form(clause, "compiler")
                    .and_then(|t| t.first().copied())
                    && a.head_name(inner) == Some("error")
                    && let Some(inner_tail) = a.as_form(inner, "error")
                    && let Some(code) = inner_tail
                        .first()
                        .copied()
                        .and_then(|id| a.as_name(id).map(str::to_string))
                {
                    let message = message_clauses(a, inner_tail);
                    let not_message = not_message_clauses(a, inner_tail);
                    let diag = diag_clause(a, inner_tail);
                    if let Some(last) = trials.last_mut() {
                        last.expect = Expect::Error(code, message, not_message);
                        if diag.is_some() {
                            last.diag = diag;
                        }
                    } else {
                        trials.push(Trial {
                            call: pending_call.take(),
                            expect: Expect::Error(code, message, not_message),
                            diag,
                        });
                    }
                }
            }
            // `(host-responses (respond E.op (: v T)) …)` — the values the host returns to the program's
            // delegated host calls, in call order. Each `respond` names its operation (`E.op`, rendered
            // dotted) and carries the value form; the gate driver passes each `(op, value)` to
            // `cdz-run --host-response op=value`. E2h.
            Some("host-responses") => {
                if let Some(tail) = a.as_form(clause, "host-responses") {
                    for &r in tail {
                        if let Some(rtail) = a.as_form(r, "respond")
                            && let Some(&op_id) = rtail.first()
                            && let Some(&val_id) = rtail.get(1)
                        {
                            let op = dotted_op(a, op_id);
                            let value = value_of(a, val_id);
                            host_responses.push((op, value));
                        }
                    }
                }
            }
            // `(host-calls (call E.op arg…) …)` — the ordered host-call sequence the run must make. Each
            // `call` names its operation (`E.op`, rendered dotted); the args are for documentation (the
            // gate verifies the op sequence). The gate compares the run's observed host calls against this.
            Some("host-calls") => {
                if let Some(tail) = a.as_form(clause, "host-calls") {
                    for &c in tail {
                        if let Some(ctail) = a.as_form(c, "call")
                            && let Some(&op_id) = ctail.first()
                        {
                            host_calls.push(dotted_op(a, op_id));
                        }
                    }
                }
            }
            // `(warns <CODE> (message "phrase")?)` — a compile WARNING the case pins (compiles clean but
            // must emit this warning). Zero or more per case, ORTHOGONAL to the primary outcome. The
            // optional message pins a substring of the warning's diagnostic prose (operator seq353 inc2).
            Some("warns") => {
                if let Some(tail) = a.as_form(clause, "warns")
                    && let Some(code) = tail
                        .first()
                        .copied()
                        .and_then(|id| a.as_name(id).map(str::to_string))
                {
                    warns.push((code, message_clauses(a, tail).into_iter().next()));
                }
            }
            // `(no-other-errors)` — a bare CASE-LEVEL no-cascade assertion: no error-severity diagnostic
            // outside the case's own `(error CODE …)` codes. Errors only (see the `Record` field doc).
            Some("no-other-errors") => no_other_errors = true,
            // `(no-diagnostic "phrase")` — a CASE-LEVEL program-scoped cross-kind message-ABSENCE pin: the
            // phrase must appear in NO diagnostic emitted for the program (any kind). Repeatable (each an
            // independent required-absence). See the `Record` field doc. Non-string / empty children are
            // ignored (a malformed clause pins nothing rather than falsely passing).
            Some("no-diagnostic") => {
                if let Some(phrase) = a
                    .as_form(clause, "no-diagnostic")
                    .and_then(|t| t.first().copied())
                    .and_then(|id| string_leaf(a, id))
                {
                    no_diagnostic.push(phrase);
                }
            }
            // `(wit-world <world-sexpr>)` — an explicit WIT world the export boundary is DECLARED by (the
            // general WIT-ABI shape), vs synthesized from the guest. Store the world subtree as one-line
            // s-expr text; the gate driver converts it to a `wit-world` binary-AST artifact for `cdz compile`.
            Some("wit-world") => {
                if let Some(id) = a
                    .as_form(clause, "wit-world")
                    .and_then(|t| t.first().copied())
                {
                    wit_world = Some(sexpr::print_from(a, id));
                    wit_world_id = Some(id);
                }
            }
            // `(component-name "cadenza:pkg/iface")` — the interface the `(wit-world …)` guest publishes its
            // export under; passed to `cdz compile --component-name` and used to qualify the run `--call`.
            Some("component-name") => {
                component_name = a
                    .as_form(clause, "component-name")
                    .and_then(|t| t.first().copied())
                    .and_then(|id| string_leaf(a, id));
            }
            // `(live-objects N)` — a CLEAN case: assert the value-heap runtime's live-cell count is EXACTLY N
            // after the run (the reachable-return residual; N=0 = fully reclaimed). Orthogonal to the
            // value/trap outcome; the gate drives this on the debug-counters runtime.
            // `(live-objects known-leak)` (seq-15 PURE-BINARY marker) = accepted-as-leaking, NOT count-checked
            // (magnitude does not matter). A legacy `(live-objects known-leak N …)` is still PARSED (the count
            // is retained in the fields) but the count is IGNORED for grading and DROPPED by render/shred — so
            // an un-migrated file still grades binary. Grading semantics live in the grade callers.
            Some("live-objects") => {
                let ids = a.as_form(clause, "live-objects").unwrap_or(&[]);
                let mut toks: Vec<String> = ids
                    .iter()
                    .map(|&id| sexpr::print_from(a, id).trim().to_string())
                    .collect();
                if toks.first().map(String::as_str) == Some("known-leak") {
                    live_objects_known_leak = true;
                    toks.remove(0);
                }
                // ONE count = uniform; 2+ = per-call positional (call i asserts count i). `live_objects`
                // keeps the FIRST (uniform / direct-gate path); `live_objects_per_call` carries the list.
                let counts: Vec<u32> = toks.iter().filter_map(|s| s.parse::<u32>().ok()).collect();
                live_objects = counts.first().copied();
                if counts.len() >= 2 {
                    live_objects_per_call = Some(counts);
                }
            }
            // `doc` — not needed to run + compare a case.
            _ => {}
        }
    }

    let input = input.ok_or_else(|| format!("case {description:?} has no (input …)"))?;
    let (program, program_ast) = normalize_program_text_and_ast(a, input);
    // The RAW input form E as its own binary-AST artifact — the input subtree VERBATIM (no
    // normalization), the exact form the `--quote-wrap` pass reifies with `(quote E)`. Encoded from the
    // live arena where the node sits (mirrors the `wit_world_ast` clone-and-encode below).
    let input_ast = {
        let mut b = Builder::new();
        let root = clone_into(a, input, &mut b);
        codec::encode(&b.finish(root))
    };

    if trials.is_empty() {
        return Err(format!("case {description:?} has no primary result clause"));
    }

    // The imposed world as its OWN binary-AST artifact: encode the `(world …)` subtree directly (its root
    // IS the compiler's `wit-world:<name>=` input, the `world_schema_tree` shape — no wrapper, no reparse).
    // `clone_into` embeds the subtree from `a` (where it is live) into a fresh arena finished on that node.
    let wit_world_ast = wit_world_id.map(|id| {
        let mut b = Builder::new();
        let root = clone_into(a, id, &mut b);
        codec::encode(&b.finish(root))
    });

    Ok(Record {
        description,
        program,
        program_ast,
        input_ast,
        modules,
        peers,
        trials,
        host_responses,
        host_calls,
        warns,
        wit_world,
        component_name,
        wit_world_ast,
        live_objects,
        live_objects_known_leak,
        live_objects_per_call,
        no_other_errors,
        no_diagnostic,
    })
}

/// Parse a `(platform-case "title" <clause>…)` into a [`PlatformRecord`]. Mirrors [`parse_case`]'s
/// clause walk. Clauses (all optional except a kickoff, which the fixpoint needs to start):
///   `(doc "…")` · `(session <alias> (reducer <prog>) (serves <family>…)?)` (1+) ·
///   `(child <alias> (reducer <prog>))` (0+ — a spawnable child reducer, not a live session) ·
///   `(kickoff <alias> (inbound <family> <value>))` (1+, declaration order — repeatable for fan-in) ·
///   `(expect-effects (effect (from <a>) (family <f>) <value>? (schema-hash <tok>)?)…)` (ordered;
///     optional `(schema-hash present)` sub-clause pins the phase-3 reify→descriptor→hash round-trip) ·
///   `(expect-messages (message (from <a>) (to <b>) (family <f>) <value>)…)` (ordered) ·
///   `(expect-delivery-failure (from <a>) (to <b>))` (0+) ·
///   `(end-state <alias> (kv <key> <value>)… (status <state>)? (close-outcome <kind>)?)` ·
///   `(events-processed <alias> <n>)` ·
///   `(expect-fault <kind>)` (0-or-1 — the run must fault with a stderr marker containing `<kind>`) ·
///   `(recover-check <alias>)` (0+ — names a session to replay-equals-recover check, I4).
fn parse_platform_case(a: &Arenas, case_id: StructId) -> Result<PlatformRecord, String> {
    let items = match a.get(case_id) {
        cadenza_syntax::ast::Struct::List(items) => items,
        _ => return Err("platform-case is not a list".into()),
    };
    let title = items
        .get(1)
        .and_then(|&id| string_leaf(a, a.peel_comments(id)))
        .ok_or("platform-case has no title string")?;

    let mut doc: Option<String> = None;
    let mut sessions: Vec<PlatformSession> = Vec::new();
    let mut children: Vec<(String, String)> = Vec::new();
    let mut kickoffs: Vec<Kickoff> = Vec::new();
    let mut expect_effects: Vec<ExpectEffect> = Vec::new();
    let mut expect_messages: Vec<ExpectMessage> = Vec::new();
    let mut expect_delivery_failures: Vec<(String, String)> = Vec::new();
    let mut end_kv: Vec<(String, String, String)> = Vec::new();
    let mut end_status: Vec<(String, String)> = Vec::new();
    let mut close_outcome: Vec<(String, String)> = Vec::new();
    let mut events_processed: Vec<(String, String)> = Vec::new();
    let mut expect_fault: Option<String> = None;
    let mut recover_checks: Vec<String> = Vec::new();

    for &clause in &items[2..] {
        // Peel a leading `;`/`//` comment wrapper around a platform-case clause (see [`parse_case`]).
        let clause = a.peel_comments(clause);
        match a.head_name(clause) {
            Some("doc") => {
                doc = a
                    .as_form(clause, "doc")
                    .and_then(|t| t.first().copied())
                    .and_then(|id| string_leaf(a, id));
            }
            // `(session <alias> (reducer <prog>) (serves <family>…)?)` — a reducer session. The reducer
            // program is normalized one-line exactly like a `(case (input …))` program (NOT compiled here);
            // `serves` is a CHILD clause listing the effect families this session handles.
            Some("session") => {
                if let Some(tail) = a.as_form(clause, "session")
                    && let Some(&alias_id) = tail.first()
                    && let Some(alias) = atom_text(a, alias_id)
                {
                    let mut program = String::new();
                    let mut serves: Vec<String> = Vec::new();
                    for &child in &tail[1..] {
                        match a.head_name(child) {
                            Some("reducer") => {
                                if let Some(prog) =
                                    a.as_form(child, "reducer").and_then(|t| t.first().copied())
                                {
                                    program = normalize_program(a, prog);
                                }
                            }
                            Some("serves") => {
                                if let Some(stail) = a.as_form(child, "serves") {
                                    for &f in stail {
                                        if let Some(fam) = atom_text(a, f) {
                                            serves.push(fam);
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    sessions.push(PlatformSession {
                        alias,
                        program,
                        serves,
                    });
                }
            }
            // `(child <alias> (reducer <prog>))` — a registered-but-not-kicked SPAWNABLE child reducer.
            // Same reducer-program normalization as `session` (NOT compiled here); no `serves` — a child
            // declares a spawn target, not a live handler. Collected in declaration order.
            Some("child") => {
                if let Some(tail) = a.as_form(clause, "child")
                    && let Some(&alias_id) = tail.first()
                    && let Some(alias) = atom_text(a, alias_id)
                {
                    let mut program = String::new();
                    for &c in &tail[1..] {
                        if a.head_name(c) == Some("reducer")
                            && let Some(prog) =
                                a.as_form(c, "reducer").and_then(|t| t.first().copied())
                        {
                            program = normalize_program(a, prog);
                        }
                    }
                    children.push((alias, program));
                }
            }
            // `(kickoff <alias> (inbound <family> <value>))` — a seed event. Repeatable: every clause is
            // collected in DECLARATION ORDER (a single-kickoff case yields a one-element Vec, rendering
            // byte-identically to before). All seeds are enqueued in this order before the drive loop.
            Some("kickoff") => {
                if let Some(tail) = a.as_form(clause, "kickoff")
                    && let Some(&alias_id) = tail.first()
                    && let Some(alias) = atom_text(a, alias_id)
                    && let Some(&inbound_id) = tail.get(1)
                    && let Some(itail) = a.as_form(inbound_id, "inbound")
                    && let Some(&fam_id) = itail.first()
                    && let Some(inbound) = atom_text(a, fam_id)
                {
                    let value = itail
                        .get(1)
                        .map(|&v| value_form_text(a, v))
                        .unwrap_or_default();
                    kickoffs.push(Kickoff {
                        alias,
                        inbound,
                        value,
                    });
                }
            }
            // `(expect-effects (effect (from <a>) (family <f>) <value>?)…)` — ordered emitted effects.
            Some("expect-effects") => {
                if let Some(tail) = a.as_form(clause, "expect-effects") {
                    for &e in tail {
                        if let Some(etail) = a.as_form(e, "effect") {
                            let from = child_name_arg(a, etail, "from");
                            let family = child_name_arg(a, etail, "family");
                            if let (Some(from), Some(family)) = (from, family) {
                                // Optional `(schema-hash <tok>)` sub-clause (phase-3 present-vs-absent pin).
                                let schema_hash = etail
                                    .iter()
                                    .find_map(|&c| a.as_form(c, "schema-hash"))
                                    .and_then(|shtail| shtail.first().copied())
                                    .and_then(|tok| atom_text(a, tok));
                                let value = etail
                                    .iter()
                                    .find(|&&c| {
                                        a.head_name(c) != Some("from")
                                            && a.head_name(c) != Some("family")
                                            && a.head_name(c) != Some("schema-hash")
                                    })
                                    .map(|&v| value_form_text(a, v));
                                expect_effects.push(ExpectEffect {
                                    from,
                                    family,
                                    value,
                                    schema_hash,
                                });
                            }
                        }
                    }
                }
            }
            // `(expect-messages (message (from <a>) (to <b>) (family <f>) <value>)…)` — ordered messages.
            Some("expect-messages") => {
                if let Some(tail) = a.as_form(clause, "expect-messages") {
                    for &m in tail {
                        if let Some(mtail) = a.as_form(m, "message") {
                            let from = child_name_arg(a, mtail, "from");
                            let to = child_name_arg(a, mtail, "to");
                            let family = child_name_arg(a, mtail, "family");
                            if let (Some(from), Some(to), Some(family)) = (from, to, family) {
                                let value = mtail
                                    .iter()
                                    .find(|&&c| {
                                        !matches!(
                                            a.head_name(c),
                                            Some("from") | Some("to") | Some("family")
                                        )
                                    })
                                    .map(|&v| value_form_text(a, v))
                                    .unwrap_or_default();
                                expect_messages.push(ExpectMessage {
                                    from,
                                    to,
                                    family,
                                    value,
                                });
                            }
                        }
                    }
                }
            }
            // `(expect-delivery-failure (from <a>) (to <b>))` — a message whose delivery must fail.
            Some("expect-delivery-failure") => {
                if let Some(tail) = a.as_form(clause, "expect-delivery-failure")
                    && let Some(from) = child_name_arg(a, tail, "from")
                    && let Some(to) = child_name_arg(a, tail, "to")
                {
                    expect_delivery_failures.push((from, to));
                }
            }
            // `(end-state <alias> (kv <key> <value>)… (status <state>)? (close-outcome <kind>)?)` — per-session end assertions.
            Some("end-state") => {
                if let Some(tail) = a.as_form(clause, "end-state")
                    && let Some(&alias_id) = tail.first()
                    && let Some(alias) = atom_text(a, alias_id)
                {
                    for &child in &tail[1..] {
                        match a.head_name(child) {
                            Some("kv") => {
                                if let Some(ktail) = a.as_form(child, "kv")
                                    && let Some(&key_id) = ktail.first()
                                    && let Some(key) = atom_text(a, key_id)
                                    && let Some(&val_id) = ktail.get(1)
                                {
                                    end_kv.push((alias.clone(), key, value_form_text(a, val_id)));
                                }
                            }
                            Some("status") => {
                                if let Some(stail) = a.as_form(child, "status")
                                    && let Some(&st_id) = stail.first()
                                    && let Some(st) = atom_text(a, st_id)
                                {
                                    end_status.push((alias.clone(), st));
                                }
                            }
                            // `(close-outcome <kind>)` — a bare token Success|Failure distinguishing how a
                            // self-closed session closed (both otherwise report `status closed`).
                            Some("close-outcome") => {
                                if let Some(ctail) = a.as_form(child, "close-outcome")
                                    && let Some(&kind_id) = ctail.first()
                                    && let Some(kind) = atom_text(a, kind_id)
                                {
                                    close_outcome.push((alias.clone(), kind));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            // `(events-processed <alias> <n>)` — the processed-log length the session must reach.
            Some("events-processed") => {
                if let Some(tail) = a.as_form(clause, "events-processed")
                    && let Some(&alias_id) = tail.first()
                    && let Some(alias) = atom_text(a, alias_id)
                    && let Some(&n_id) = tail.get(1)
                {
                    events_processed.push((alias, value_of(a, n_id)));
                }
            }
            // `(expect-fault <kind>)` — a single bare token (e.g. `SettleUnbounded`); the run must fault
            // with a stderr marker containing this kind. At most one; a later one overrides. The reader
            // only carries the token — grading is xtask's (v-platform-conformance's domain).
            Some("expect-fault") => {
                if let Some(tail) = a.as_form(clause, "expect-fault")
                    && let Some(&kind_id) = tail.first()
                    && let Some(kind) = atom_text(a, kind_id)
                {
                    expect_fault = Some(kind);
                }
            }
            // `(recover-check <alias>)` — a single bare alias token naming a session to
            // replay-equals-recover check (I4). 0-or-more; declaration order preserved. The reader
            // only carries the alias — the assert (recover-equal) is xtask's (v-platform-conformance).
            Some("recover-check") => {
                if let Some(tail) = a.as_form(clause, "recover-check")
                    && let Some(&alias_id) = tail.first()
                    && let Some(alias) = atom_text(a, alias_id)
                {
                    recover_checks.push(alias);
                }
            }
            _ => {}
        }
    }

    if kickoffs.is_empty() {
        return Err(format!("platform-case {title:?} has no (kickoff …)"));
    }
    if sessions.is_empty() {
        return Err(format!("platform-case {title:?} has no (session …)"));
    }

    Ok(PlatformRecord {
        title,
        doc,
        sessions,
        children,
        kickoffs,
        expect_effects,
        expect_messages,
        expect_delivery_failures,
        end_kv,
        end_status,
        close_outcome,
        events_processed,
        expect_fault,
        recover_checks,
    })
}

/// A `(<head> <atom>)` child clause's argument (e.g. `(from "worker")` → `"worker"`), searched among a
/// clause's children — the addressing shape shared by effect/message `(from …)`/`(to …)`/`(family …)`.
/// The atom is a string or bare name (via [`atom_text`]), matching the alias spelling elsewhere.
fn child_name_arg(a: &Arenas, tail: &[StructId], head: &str) -> Option<String> {
    tail.iter().find_map(|&child| {
        a.as_form(child, head)
            .and_then(|t| t.first().copied())
            .and_then(|id| atom_text(a, id))
    })
}

/// Normalize a case's `input` occurrence to the runnable export shape, returning one-line s-expr text:
///   - `(do … (export …))` → unchanged
///   - `(module name def…)` → `(do def… (export main))`
///   - a bare expression `E` → `(do (def (main) E) (export main))`
fn normalize_program(a: &Arenas, input: StructId) -> String {
    normalize_program_text_and_ast(a, input).0
}

/// Normalize `input` to the runnable export-shape program, once, yielding BOTH serializations from one
/// build (no round-trip through the other's format): the one-line s-expression TEXT (the stdout/xtask
/// path), and the BINARY AST bytes (`codec::encode`, the shred's `program.ast`). The program's ROOT is
/// the normalized `(do (def …) … (export …))` form itself — the SINGLE-FORM shape `sexpr::read` (and thus
/// `cdz convert --to binary` / the compiler's binary-AST input) expects. It is NOT the synthetic multi-form
/// `(do …)` document `sexpr::read_all` wraps a whole corpus FILE in: the program is already one `(do …)`
/// form, and re-wrapping it (`(do (do …))`) would bury the `(export …)` a level too deep, so the compiler
/// would see nothing public. The text is `print_from` of the same root (identical one-line rendering).
pub(crate) fn normalize_program_text_and_ast(a: &Arenas, input: StructId) -> (String, Vec<u8>) {
    let mut b = Builder::new();
    let prog = build_normalized_program(a, input, &mut b);
    let arenas = b.finish(prog);
    (sexpr::print_from(&arenas, prog), codec::encode(&arenas))
}

/// Build the normalized program node into `b` and return its `StructId` (the `(do (def (main) …)
/// (export main))` form) — the shared core of both serializations. Does NOT finish `b`.
fn build_normalized_program(a: &Arenas, input: StructId, b: &mut Builder) -> StructId {
    match a.head_name(input) {
        // A `(do …)` input that ALREADY declares `(export …)` is a full program — cloned verbatim. A
        // `(do …)` WITHOUT an export is a bare SEQUENCING-block VALUE (`(do 1 2 3)`, `(do (record …) 42)`),
        // an expression whose value is the program result: it falls through to the `_` arm and is wrapped
        // as `(do (def (main) <the-do>) (export main))`, like any bare expression.
        Some("do") if do_block_has_export(a, input) => clone_into(a, input, b),
        Some("module") => {
            // Rebuild `(do <module's forms after the name> (export main))`.
            let forms = match a.get(input) {
                cadenza_syntax::ast::Struct::List(items) => &items[2..], // skip `module` head + the name
                _ => &[][..],
            };
            let do_head = b.name("do");
            let mut children = vec![do_head];
            for &f in forms {
                children.push(clone_into(a, f, b));
            }
            children.push(export_main(b));
            b.list(children)
        }
        _ => {
            // Bare expression E → (do (def (main) E) (export main)).
            let do_head = b.name("do");
            let def_head = b.name("def");
            let main_name = b.name("main");
            let main_sig = b.list(vec![main_name]);
            let e = clone_into(a, input, b);
            let def_main = b.list(vec![def_head, main_sig, e]);
            let export = export_main(b);
            b.list(vec![do_head, def_main, export])
        }
    }
}

/// Whether a `(do …)` form declares an `(export …)` among its top-level forms — the tell that it is a
/// FULL PROGRAM (module body) rather than a bare sequencing-block VALUE. A full program is passed
/// verbatim; a value-block is wrapped as `main`'s body (see [`normalize_program`]).
fn do_block_has_export(a: &Arenas, do_form: StructId) -> bool {
    match a.get(do_form) {
        cadenza_syntax::ast::Struct::List(items) => {
            items.iter().any(|&f| a.head_name(f) == Some("export"))
        }
        _ => false,
    }
}

/// Build `(export main)` in `b`.
fn export_main(b: &mut Builder) -> StructId {
    let export = b.name("export");
    let main = b.name("main");
    b.list(vec![export, main])
}

/// Deep-clone occurrence `id` from `a` into builder `b`, returning the new occurrence id.
fn clone_into(a: &Arenas, id: StructId, b: &mut Builder) -> StructId {
    match a.get(id) {
        cadenza_syntax::ast::Struct::Atom(l) => {
            let leaf = a.leaf(*l).clone();
            b.atom_leaf(leaf)
        }
        cadenza_syntax::ast::Struct::List(items) => {
            let children: Vec<StructId> = items.iter().map(|&c| clone_into(a, c, b)).collect();
            b.list(children)
        }
    }
}

/// The canonical value-form text of an `(output …)` payload. For `(: <value> <Type>)` we keep the
/// whole form's text; the driver's comparison logic decides how to match it against a rendered run.
fn value_form_text(a: &Arenas, form: StructId) -> String {
    sexpr::print_from(a, form)
}

/// The bare VALUE text of an argument form. A `(call …)` argument is written as a `(: <value> <Type>)`
/// value-form (the same form `output` uses); the runner takes just the value and coerces it to the
/// export's declared parameter type, so strip the annotation to `<value>`. An argument written as a
/// bare value (no `(: …)` wrapper) is taken verbatim.
fn value_of(a: &Arenas, form: StructId) -> String {
    if let Some(tail) = a.as_form(form, ":")
        && let Some(&value) = tail.first()
    {
        return sexpr::print_from(a, value);
    }
    sexpr::print_from(a, form)
}

/// Render a host operation reference in DOTTED form `E.op` — the form the runner observes and matches. An
/// operation is written `(. E op)` (member access) in the corpus; render its `E.op`. A bare name (or any
/// other shape) passes through via `sexpr::print_from`, so this only rewrites the member-access spelling.
fn dotted_op(a: &Arenas, id: StructId) -> String {
    if let Some(tail) = a.as_form(id, ".")
        && tail.len() == 2
        && let (Some(e), Some(op)) = (a.as_name(tail[0]), a.as_name(tail[1]))
    {
        return format!("{e}.{op}");
    }
    sexpr::print_from(a, id)
}

/// The text of an ATOM used as an identifier in a platform case — a bare NAME (`worker`) OR a quoted
/// STRING leaf (`"worker"`). Platform cases spell aliases/families/keys/statuses as strings (the
/// co-designed canonical form, e.g. `(session "worker" …)`), but a bare name is equally meaningful; this
/// accepts either so the reader is robust to both spellings. `None` for any non-atom.
fn atom_text(a: &Arenas, id: StructId) -> Option<String> {
    a.as_name(id)
        .map(str::to_string)
        .or_else(|| string_leaf(a, id))
}

/// The string a `Str` leaf carries, if `id` is one.
fn string_leaf(a: &Arenas, id: StructId) -> Option<String> {
    match a.get(id) {
        cadenza_syntax::ast::Struct::Atom(l) => match a.leaf(*l) {
            cadenza_syntax::ast::Leaf::Str(s) => Some(s.to_string()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SHRED FAITHFULNESS: the whole-stream render equals concatenating the per-record renders. The nix
    /// corpus pipeline (`design/DESIGN-corpus-nix-per-case-caching.md`) shreds a file into one `.rec` per
    /// case via `render(&[record])`; this pins that a per-case file is byte-identical to that case's slice
    /// of the stdout stream, so the shred and the stream can never disagree.
    #[test]
    fn per_record_render_concatenates_to_the_stream() {
        let recs = read(
            r#"(case "one" (input 1) (output (: 1 Int64)))
               (case "two" (input 2) (output (: 2 Int64)))
               (case "err" (input bogus) (error CDZ0201))"#,
        )
        .unwrap();
        assert_eq!(recs.len(), 3);
        let stream = render(&recs);
        let concat: String = recs
            .iter()
            .map(|r| render(std::slice::from_ref(r)))
            .collect();
        assert_eq!(
            stream, concat,
            "per-record renders must concatenate to the whole-stream render"
        );
    }

    /// A single-result case (the common shape) parses to ONE trial — no call, one output.
    #[test]
    fn a_single_result_case_is_one_trial() {
        let recs = read(r#"(case "x" (input 5) (output (: 5 Int64)))"#).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].trials.len(), 1);
        assert!(recs[0].trials[0].call.is_none());
        assert!(matches!(&recs[0].trials[0].expect, Expect::Output(v) if v == "(: 5 Int64)"));
    }

    /// An `(error CODE (message …) (fix …) (count N))` case parses the NESTED diagnostic-quality facets
    /// into `Trial.diag`; a `(warning CODE …)` parses to `Expect::Warning` (+ its facets). This is the
    /// REPEATED `(message …)` clauses collect into the Vec (ALL required substrings, AND) — the multi-part
    /// diagnostic form (e.g. a coercion error naming the rule AND both operand types); a single clause keeps
    /// the historical one-element form; none = code-only.
    #[test]
    fn repeated_message_clauses_collect_all() {
        let recs = read(
            r#"(case "multi" (input 1_)
                 (error CDZ0301 (message "no implicit conversion") (message "Float64") (message "Int64")))
               (case "single" (input 1_) (error CDZ0201 (message "sep")))
               (case "none" (input 1_) (error CDZ0201))"#,
        )
        .unwrap();
        assert!(matches!(&recs[0].trials[0].expect, Expect::Error(c, ms, _)
            if c == "CDZ0301" && ms.as_slice() == ["no implicit conversion", "Float64", "Int64"]));
        assert!(matches!(&recs[1].trials[0].expect, Expect::Error(c, ms, _)
            if c == "CDZ0201" && ms.as_slice() == ["sep"]));
        assert!(
            matches!(&recs[2].trials[0].expect, Expect::Error(c, ms, _) if c == "CDZ0201" && ms.is_empty())
        );
    }

    /// seq-29 `(not "phrase")` message-ABSENCE pin: parses into the third field of `Expect::Error`,
    /// separate from the positive `(message …)` substrings, and renders back verbatim.
    #[test]
    fn not_message_clause_parses_and_renders() {
        let recs = read(
            r#"(case "err" (input 1_) (error CDZ0201 (message "malformed") (not "internal error")))"#,
        )
        .unwrap();
        assert!(
            matches!(&recs[0].trials[0].expect, Expect::Error(c, ms, neg)
            if c == "CDZ0201" && ms.as_slice() == ["malformed"] && neg.as_slice() == ["internal error"])
        );
        let text = to_records(
            r#"(case "err" (input 1_) (error CDZ0201 (message "malformed") (not "internal error")))"#,
        )
        .unwrap();
        assert!(
            text.contains(r#"(not "internal error")"#),
            "not-clause renders: {text}"
        );
    }

    /// authoring end of C1 — the counterpart to `cdz_corpus_grade`'s decode of the shredded clauses.
    #[test]
    fn diagnostic_quality_facets_parse_from_error_and_warning() {
        let recs = read(
            r#"(case "fix" (input 1_)
                 (error CDZ0201 (message "sep") (fix (kind replace) (replacement "1") (verified)) (count 2)))
               (case "no-fix" (input 1_)
                 (error CDZ0201 (no-fix) (once)))
               (case "warn" (input (do (def (main) 0) (export main)))
                 (warning CDZ0305 (message "dead") (fix (replacement-contains "unreachable") (unverified))))"#,
        )
        .unwrap();
        assert_eq!(recs.len(), 3);

        // (1) error + full fix + count.
        let d = recs[0].trials[0].diag.as_ref().expect("fix case has diag");
        assert_eq!(d.count, Some(2));
        assert!(!d.no_fix);
        let fx = d.fix.as_ref().expect("fix present");
        assert_eq!(fx.kind.as_deref(), Some("replace"));
        assert_eq!(fx.replacement, Some(ReplMatch::Exact("1".into())));
        assert_eq!(fx.verified, Some(true));

        // (2) no-fix + once (== count 1).
        let d = recs[1].trials[0]
            .diag
            .as_ref()
            .expect("no-fix case has diag");
        assert!(d.no_fix);
        assert_eq!(d.count, Some(1));
        assert!(d.fix.is_none());

        // (3) warning result kind + substring fix + unverified.
        assert!(matches!(&recs[2].trials[0].expect, Expect::Warning(c, m, _)
            if c == "CDZ0305" && m.as_slice() == ["dead"]));
        let fx = recs[2].trials[0]
            .diag
            .as_ref()
            .and_then(|d| d.fix.as_ref())
            .expect("warning fix present");
        assert_eq!(
            fx.replacement,
            Some(ReplMatch::Contains("unreachable".into()))
        );
        assert_eq!(fx.verified, Some(false));

        // A plain code+message case pins NO diag (the common form stays clause-free).
        let plain = read(r#"(case "p" (input 1_) (error CDZ0201 (message "sep")))"#).unwrap();
        assert!(plain[0].trials[0].diag.is_none());
    }

    /// A `(platform-case …)` parses the session/kickoff/end-state shape and RENDERS the flat fixed-arity
    /// record lines v-platform-conformance's grade path consumes. Full parse→render pipeline (the two-crate
    /// discipline: a record line the reader never emits would silently fail the grade downstream).
    #[test]
    fn platform_case_parses_and_renders_the_fixed_arity_record_lines() {
        // The CANONICAL co-designed form spells aliases/families/keys/statuses as quoted STRINGS
        // (`(session "worker" …)`), so the reader must read a string leaf here, not only a bare name.
        let recs = read_platform(
            r#"(platform-case "worker asks a clock then messages a reporter"
                 (doc "one kickoff; runs to a fixpoint")
                 (session "worker"   (reducer (do (def (main) 0) (export main))))
                 (session "reporter" (reducer (do (def (main) 0) (export main))))
                 (session "clock"    (reducer (do (def (main) 0) (export main))) (serves "now"))
                 (kickoff "worker" (inbound "start" (: unit Unit)))
                 (expect-effects
                   (effect (from "worker") (family "now"))
                   (effect (from "worker") (family "log") (: "t=0" String)))
                 (expect-messages
                   (message (from "worker") (to "reporter") (family "message") (: "done" String)))
                 (expect-delivery-failure (from "worker") (to "closed"))
                 (end-state "worker"   (status "quiescent"))
                 (end-state "reporter" (kv "seen" (: 1 Int64)) (status "quiescent"))
                 (events-processed "worker" 3))"#,
        )
        .unwrap();
        assert_eq!(recs.len(), 1);
        let r = &recs[0];
        assert_eq!(r.title, "worker asks a clock then messages a reporter");
        assert_eq!(r.doc.as_deref(), Some("one kickoff; runs to a fixpoint"));
        assert_eq!(r.sessions.len(), 3);
        assert_eq!(r.sessions[2].alias, "clock");
        assert_eq!(r.sessions[2].serves, vec!["now".to_string()]);
        assert_eq!(r.kickoffs.len(), 1);
        assert_eq!(r.kickoffs[0].alias, "worker");
        assert_eq!(r.kickoffs[0].inbound, "start");
        assert_eq!(r.kickoffs[0].value, "(: unit Unit)");
        // Ordered effects: the payload-less one carries None, the second keeps its value form.
        assert_eq!(r.expect_effects.len(), 2);
        assert_eq!(r.expect_effects[0].family, "now");
        assert_eq!(r.expect_effects[0].value, None);
        assert_eq!(
            r.expect_effects[1].value.as_deref(),
            Some("(: \"t=0\" String)")
        );
        assert_eq!(r.expect_messages.len(), 1);
        assert_eq!(r.expect_messages[0].to, "reporter");
        assert_eq!(
            r.expect_delivery_failures,
            vec![("worker".to_string(), "closed".to_string())]
        );
        assert_eq!(
            r.end_kv,
            vec![(
                "reporter".to_string(),
                "seen".to_string(),
                "(: 1 Int64)".to_string()
            )]
        );
        assert_eq!(r.end_status.len(), 2);
        assert_eq!(
            r.events_processed,
            vec![("worker".to_string(), "3".to_string())]
        );

        // Render emits the confirmed fixed-arity lines (the cdz-corpus→xtask contract).
        let out = render_platform(&recs);
        assert!(out.contains("platform-case\tworker asks a clock then messages a reporter\n"));
        assert!(out.contains("session\tclock\t"));
        assert!(out.contains("serves\tclock\tnow\n"));
        assert!(out.contains("kickoff\tworker\tstart\t(: unit Unit)\n"));
        assert!(out.contains("expect-effect\tworker\tnow\n")); // no value column when payload-less
        assert!(out.contains("expect-effect\tworker\tlog\t(: \"t=0\" String)\n"));
        assert!(out.contains("expect-message\tworker\treporter\tmessage\t(: \"done\" String)\n"));
        assert!(out.contains("expect-delivery-failure\tworker\tclosed\n"));
        assert!(out.contains("end-kv\treporter\tseen\t(: 1 Int64)\n"));
        assert!(out.contains("end-status\tworker\tquiescent\n"));
        assert!(out.contains("events-processed\tworker\t3\n"));
        // No (expect-fault …) clause → the record carries None and no line is rendered.
        assert_eq!(r.expect_fault, None);
        assert!(!out.contains("expect-fault"));
        // No (schema-hash …) sub-clause on any effect → None, and no expect-effect-schema-hash line
        // (absent ⇒ byte-identical to pre-clause platform cases).
        assert!(r.expect_effects.iter().all(|e| e.schema_hash.is_none()));
        assert!(!out.contains("expect-effect-schema-hash"));
    }

    /// An optional `(schema-hash present)` sub-clause under `(effect …)` carries a present-vs-absent token
    /// through to `ExpectEffect.schema_hash` and renders a SEPARATE `expect-effect-schema-hash` line keyed on
    /// `(from, family)` — the phase-3 reify→descriptor→hash round-trip pin (v-platform-conformance case 32).
    /// A present sub-clause must NOT be mis-read as the effect's value form, and effects without it stay None.
    #[test]
    fn platform_case_carries_an_expect_effect_schema_hash_present_pin() {
        let src = r#"(platform-case "a reified world-effect observes its phase-3 schema-hash"
                 (session "s" (reducer (do (def (main) 0) (export main))))
                 (kickoff "s" (inbound "start" (: unit Unit)))
                 (expect-effects
                   (effect (from "s") (family "effect/Beat") (schema-hash present))
                   (effect (from "s") (family "log") (: "t=0" String))))"#;
        let recs = read_platform(src).unwrap();
        assert_eq!(recs.len(), 1);
        let r = &recs[0];
        assert_eq!(r.expect_effects.len(), 2);
        // The schema-hash-carrying effect: present, and its value form is NOT the schema-hash sub-clause.
        assert_eq!(r.expect_effects[0].family, "effect/Beat");
        assert_eq!(r.expect_effects[0].schema_hash.as_deref(), Some("present"));
        assert_eq!(r.expect_effects[0].value, None);
        // The sibling effect keeps its value form and carries no schema-hash.
        assert_eq!(
            r.expect_effects[1].value.as_deref(),
            Some("(: \"t=0\" String)")
        );
        assert_eq!(r.expect_effects[1].schema_hash, None);

        // Renders one separate line for the present effect, keyed on (from, family); the sibling emits none.
        let out = render_platform(&recs);
        assert!(out.contains("expect-effect-schema-hash\ts\teffect/Beat\tpresent\n"));
        assert_eq!(out.matches("expect-effect-schema-hash").count(), 1);
        // Re-read the rendered sexp is a fixed point on the schema-hash observation.
        let r2 = &read_platform(src).unwrap()[0];
        assert_eq!(r2.expect_effects[0].schema_hash.as_deref(), Some("present"));
    }

    /// An optional `(expect-fault <kind>)` clause carries a single bare token through to the record and
    /// renders one `expect-fault\t<kind>` line (v-platform-conformance grades the fault on the xtask side).
    /// The reader is order-insensitive to the clause and round-trips it via the sexp form.
    #[test]
    fn platform_case_carries_an_expect_fault_kind_through_read_and_render() {
        let src = r#"(platform-case "an unbounded ping-pong is a graded fault, never a hang"
                 (session "a" (reducer (do (def (main) 0) (export main))) (serves "ping"))
                 (session "b" (reducer (do (def (main) 0) (export main))) (serves "pong"))
                 (kickoff "a" (inbound "start" (: unit Unit)))
                 (expect-fault SettleUnbounded))"#;
        let recs = read_platform(src).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].expect_fault.as_deref(), Some("SettleUnbounded"));

        // Renders exactly one fixed-arity line, after events-processed and before the terminator.
        let out = render_platform(&recs);
        assert!(out.contains("expect-fault\tSettleUnbounded\n"));

        // The clause round-trips through the sexp form (parse → render-to-sexp → re-parse is stable);
        // here we assert the rendered stream is fixed-point under a re-read of the ORIGINAL sexp.
        let recs2 = read_platform(src).unwrap();
        assert_eq!(render_platform(&recs2), out);
    }

    /// `(recover-check <alias>)` clauses (0+, I4 replay-equals-recover) collect into `recover_checks` in
    /// DECLARATION ORDER and render one `recover-check\t<alias>` line each; a case with no such clause
    /// renders byte-identically to before (no line). The reader only carries the aliases — the recover-equal
    /// assert is xtask's (v-platform-conformance's domain).
    #[test]
    fn platform_case_carries_recover_check_aliases_through_read_and_render() {
        let src = r#"(platform-case "a recovered session replays equal to a full replay"
                 (session "a" (reducer (do (def (main) 0) (export main))) (serves "tick"))
                 (session "b" (reducer (do (def (main) 0) (export main))) (serves "tock"))
                 (kickoff "a" (inbound "start" (: unit Unit)))
                 (recover-check a)
                 (recover-check b))"#;
        let recs = read_platform(src).unwrap();
        assert_eq!(recs.len(), 1);
        // Two aliases, in declaration order.
        assert_eq!(
            recs[0].recover_checks,
            vec!["a".to_string(), "b".to_string()]
        );

        // Renders one fixed-arity line per alias, after expect-fault and before the terminator.
        let out = render_platform(&recs);
        assert!(out.contains("recover-check\ta\n"));
        assert!(out.contains("recover-check\tb\n"));
        assert_eq!(out.matches("recover-check").count(), 2);
        // Stable under a re-read of the original sexp.
        let recs2 = read_platform(src).unwrap();
        assert_eq!(render_platform(&recs2), out);
    }

    /// A platform-case with NO `(recover-check …)` clause carries an empty `recover_checks` and renders
    /// no `recover-check` line — byte-identical to before the clause existed (existing cases unchanged).
    #[test]
    fn platform_case_without_recover_check_renders_no_line() {
        let src = r#"(platform-case "an ordinary case with no such clause"
                 (session "a" (reducer (do (def (main) 0) (export main))) (serves "tick"))
                 (kickoff "a" (inbound "start" (: unit Unit))))"#;
        let recs = read_platform(src).unwrap();
        assert!(recs[0].recover_checks.is_empty());
        // Assert on the rendered LINE (not a bare substring — a title could legitimately contain the word).
        let out = render_platform(&recs);
        assert!(!out.contains("recover-check\t"));
    }

    /// Multiple `(kickoff …)` clauses (the operator-approved fan-in shape) collect into `kickoffs` in
    /// DECLARATION ORDER and render one `kickoff` line each — a single-kickoff case is a one-element Vec
    /// rendering byte-identically to before (so the 23 existing single-kickoff cases are unchanged).
    #[test]
    fn platform_case_collects_multiple_kickoffs_in_declaration_order() {
        let src = r#"(platform-case "two senders fan in to one reporter"
                 (session "s1" (reducer (do (def (main) 0) (export main))))
                 (session "s2" (reducer (do (def (main) 0) (export main))))
                 (session "r"  (reducer (do (def (main) 0) (export main))) (serves "count"))
                 (kickoff "s1" (inbound "start" (: 1 Int64)))
                 (kickoff "s2" (inbound "start" (: 2 Int64))))"#;
        let recs = read_platform(src).unwrap();
        assert_eq!(recs.len(), 1);
        // Two seeds, in source order.
        assert_eq!(recs[0].kickoffs.len(), 2);
        assert_eq!(recs[0].kickoffs[0].alias, "s1");
        assert_eq!(recs[0].kickoffs[0].value, "(: 1 Int64)");
        assert_eq!(recs[0].kickoffs[1].alias, "s2");
        assert_eq!(recs[0].kickoffs[1].value, "(: 2 Int64)");
        // Renders one line per kickoff, in order (the grader delivers each before draining).
        let out = render_platform(&recs);
        assert!(out.contains("kickoff\ts1\tstart\t(: 1 Int64)\n"));
        assert!(out.contains("kickoff\ts2\tstart\t(: 2 Int64)\n"));
        assert_eq!(out.matches("kickoff\t").count(), 2);
        // Re-read of the same sexp renders identically (fixed-point).
        assert_eq!(render_platform(&read_platform(src).unwrap()), out);
    }

    /// An optional `(child <alias> (reducer <prog>))` clause registers a spawnable child reducer: it
    /// collects into `children` (alias, normalized-program) in declaration order and renders one
    /// `child\t<alias>\t<program>` line, structurally mirroring `session` but with no `serves`. Absent =
    /// no line, so existing cases stay byte-identical. Grading (compile + `--child-reducer`) is xtask's.
    #[test]
    fn platform_case_registers_spawnable_children() {
        let src = r#"(platform-case "a supervisor spawns a worker child"
                 (session "sup" (reducer (do (def (main) 0) (export main))) (serves "spawn"))
                 (child "worker" (reducer (do (def (main) 1) (export main))))
                 (kickoff "sup" (inbound "start" (: unit Unit))))"#;
        let recs = read_platform(src).unwrap();
        assert_eq!(recs.len(), 1);
        // One live session, one registered (not kicked) child.
        assert_eq!(recs[0].sessions.len(), 1);
        assert_eq!(recs[0].children.len(), 1);
        assert_eq!(recs[0].children[0].0, "worker");
        assert!(recs[0].children[0].1.contains("(def (main) 1)"));
        // Renders one child line (program normalized one-line), distinct from the session line.
        let out = render_platform(&recs);
        assert!(out.contains("child\tworker\t"));
        assert_eq!(
            out.matches("\nchild\t").count() + out.starts_with("child\t") as usize,
            1
        );
        assert!(out.contains("session\tsup\t"));
        // Fixed-point under a re-read of the original sexp.
        assert_eq!(render_platform(&read_platform(src).unwrap()), out);

        // A case with NO child clause renders no child line (byte-identical to today).
        let no_child = read_platform(
            r#"(platform-case "no child"
                 (session "w" (reducer (do (def (main) 0) (export main))))
                 (kickoff "w" (inbound "start" (: unit Unit))))"#,
        )
        .unwrap();
        assert!(no_child[0].children.is_empty());
        assert!(!render_platform(&no_child).contains("child\t"));
    }

    /// An optional `(close-outcome <kind>)` sub-clause under `(end-state <alias> …)` records how a
    /// self-closed session closed (Success vs Failure — both otherwise report `status closed`). It
    /// collects into `close_outcome` (alias, kind) and renders one `end-close-outcome\t<alias>\t<kind>`
    /// line, mirroring the `(status …)` sub-clause. Absent = no line (byte-identical).
    #[test]
    fn platform_case_end_state_carries_a_close_outcome() {
        let src = r#"(platform-case "two self-closing sessions distinguish Success vs Failure close"
                 (session "ok"  (reducer (do (def (main) 0) (export main))))
                 (session "bad" (reducer (do (def (main) 0) (export main))))
                 (kickoff "ok" (inbound "start" (: unit Unit)))
                 (end-state "ok"  (status "closed") (close-outcome Success))
                 (end-state "bad" (status "closed") (close-outcome Failure)))"#;
        let recs = read_platform(src).unwrap();
        assert_eq!(recs.len(), 1);
        // Both closed (status), but distinct close-outcomes.
        assert_eq!(recs[0].end_status.len(), 2);
        assert_eq!(
            recs[0].close_outcome,
            vec![
                ("ok".to_string(), "Success".to_string()),
                ("bad".to_string(), "Failure".to_string()),
            ]
        );
        let out = render_platform(&recs);
        assert!(out.contains("end-close-outcome\tok\tSuccess\n"));
        assert!(out.contains("end-close-outcome\tbad\tFailure\n"));
        // Fixed-point under a re-read of the original sexp.
        assert_eq!(render_platform(&read_platform(src).unwrap()), out);

        // An end-state with no close-outcome renders no end-close-outcome line (byte-identical to today).
        let no_co = read_platform(
            r#"(platform-case "no close-outcome"
                 (session "w" (reducer (do (def (main) 0) (export main))))
                 (kickoff "w" (inbound "start" (: unit Unit)))
                 (end-state "w" (status "quiescent")))"#,
        )
        .unwrap();
        assert!(no_co[0].close_outcome.is_empty());
        assert!(!render_platform(&no_co).contains("end-close-outcome"));
    }

    /// A platform-case MUST carry a kickoff and at least one session — the fixpoint has nothing to run
    /// otherwise. A missing kickoff is a hard parse error (fail loud, like a case with no result clause).
    #[test]
    fn platform_case_without_a_kickoff_is_an_error() {
        let err = read_platform(
            r#"(platform-case "no kickoff" (session worker (reducer (do (def (main) 0) (export main)))))"#,
        )
        .unwrap_err();
        assert!(err.contains("no (kickoff"));
    }

    /// Interleaved `(call …) (output …)` pairs parse to one trial each, in order — the multi-call form.
    #[test]
    fn interleaved_call_result_pairs_are_separate_trials() {
        let recs = read(
            r#"(case "x"
                 (input (do (def (main (: b Bool)) (match b (true 1) (false 2))) (export main)))
                 (call main (: true Bool))  (output (: 1 Int64))
                 (call main (: false Bool)) (output (: 2 Int64)))"#,
        )
        .unwrap();
        assert_eq!(recs[0].trials.len(), 2);
        let t0 = &recs[0].trials[0];
        assert_eq!(t0.call.as_ref().unwrap().args, vec!["true".to_string()]);
        assert!(matches!(&t0.expect, Expect::Output(v) if v == "(: 1 Int64)"));
        let t1 = &recs[0].trials[1];
        assert_eq!(t1.call.as_ref().unwrap().args, vec!["false".to_string()]);
        assert!(matches!(&t1.expect, Expect::Output(v) if v == "(: 2 Int64)"));
    }

    /// A case may mix result KINDS across its trials — one `(output …)`, one `(trap …)`.
    #[test]
    fn trials_may_have_different_result_kinds() {
        let recs = read(
            r#"(case "x"
                 (input (do (def (main (: x Int64)) (<< x 63)) (export main)))
                 (call main (: -1 Int64)) (output (: -9223372036854775808 Int64))
                 (call main (: 1 Int64))  (trap "integer overflow"))"#,
        )
        .unwrap();
        assert_eq!(recs[0].trials.len(), 2);
        assert!(matches!(&recs[0].trials[0].expect, Expect::Output(_)));
        assert!(matches!(&recs[0].trials[1].expect, Expect::Trap(r) if r == "integer overflow"));
    }

    /// A `(declines)` clause is now a HARD ERROR (operator directive; corpus (declines)=0): the bare-decline
    /// marker was REMOVED — parsing any case with `(declines)` fails, so the acceptance path is gone. A
    /// rejection must be coded `(error CDZxxxx)`; a should-work must be a TODO `(output V)`.
    #[test]
    fn a_declines_clause_is_now_a_hard_error() {
        let err = read(
            r#"(case "x"
                 (input (do (def (mk) (fn ((: x Int64)) unit)) (export mk)))
                 (declines))"#,
        )
        .err()
        .expect("a `(declines)` clause must be rejected, but parsing succeeded");
        assert!(
            err.contains("(declines) is no longer supported"),
            "a `(declines)` clause must be rejected with the removal error, got: {err:?}"
        );

        // A coded `(declines CDZ0900 …)` (the former seq-286 form) is ALSO rejected — the acceptance path
        // is gone entirely, not merely the codeless form.
        let err2 = read(r#"(case "y" (input 1_) (declines CDZ0900 (message "not yet")))"#)
            .err()
            .expect("a coded `(declines CDZ0900 …)` must also be rejected");
        assert!(err2.contains("(declines) is no longer supported"));
    }

    /// A `(then <arg>…)` after a `(call …)` records a SECOND call on the same handle (borrow<t>
    /// repeatability): the pending call carries `second_call = Some(args)`, and it stays ONE trial.
    #[test]
    fn a_then_clause_records_a_second_call_on_the_pending_call() {
        let recs = read(
            r#"(case "x"
                 (input (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ k x))) (export adder)))
                 (call adder (: 10 Int64) (: 5 Int64))
                 (then (: 7 Int64))
                 (output (: (tuple 15 17) (Tuple Int64 Int64))))"#,
        )
        .unwrap();
        assert_eq!(
            recs[0].trials.len(),
            1,
            "a `(then …)` stays in the same trial"
        );
        let call = recs[0].trials[0].call.as_ref().unwrap();
        assert_eq!(call.export, "adder");
        assert_eq!(call.args, vec!["10".to_string(), "5".to_string()]);
        assert_eq!(call.second_call, Some(vec!["7".to_string()]));
    }

    /// A bare `(then)` (no args) records a nullary second call — `second_call = Some(vec![])`, distinct
    /// from `None` (no second call at all), so a nullary-arg closure is repeatable too.
    #[test]
    fn a_bare_then_clause_records_a_nullary_second_call() {
        let recs = read(
            r#"(case "x"
                 (input (do (def (mk) (fn () 7)) (export mk)))
                 (call mk)
                 (then)
                 (output (: (tuple 7 7) (Tuple Int64 Int64))))"#,
        )
        .unwrap();
        let call = recs[0].trials[0].call.as_ref().unwrap();
        assert_eq!(call.second_call, Some(Vec::new()));
    }

    /// A `(then …)` serializes to a `then-call\t<n>` marker line (n = arg count) plus one `then-arg\t<v>`
    /// line each, after the `arg` lines — how the gate driver learns to drive a two-call case.
    #[test]
    fn render_emits_then_call_and_then_arg_lines() {
        let text = to_records(
            r#"(case "x"
                 (input (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ k x))) (export adder)))
                 (call adder (: 10 Int64) (: 5 Int64))
                 (then (: 7 Int64))
                 (output (: (tuple 15 17) (Tuple Int64 Int64))))"#,
        )
        .unwrap();
        assert!(
            text.contains("then-call\t1\n"),
            "then-call marker with arg count, got: {text:?}"
        );
        assert!(
            text.contains("then-arg\t7\n"),
            "then-arg line, got: {text:?}"
        );
        // The ordinary one-call form emits NO then-* line (back-compat).
        let plain = to_records(
            r#"(case "y"
                 (input (do (def (main (: x Int64)) (+ x 1)) (export main)))
                 (call main (: 5 Int64)) (output (: 6 Int64)))"#,
        )
        .unwrap();
        assert!(
            !plain.contains("then-call"),
            "one-call form has no then-call line"
        );
    }

    /// A `(drop)` clause sets `drop_handle` on the pending call and serializes to a `drop-handle` line;
    /// a case without it stays `false` and emits no such line (back-compat).
    #[test]
    fn a_drop_clause_sets_drop_handle_and_serializes() {
        let recs = read(
            r#"(case "x"
                 (input (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k))) (export adder)))
                 (call adder (: 10 Int64) (: 5 Int64))
                 (drop)
                 (output (: 15 Int64))
                 (live-objects 0))"#,
        )
        .unwrap();
        let call = recs[0].trials[0].call.as_ref().unwrap();
        assert!(call.drop_handle, "(drop) sets drop_handle");
        let text = to_records(
            r#"(case "x"
                 (input (do (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k))) (export adder)))
                 (call adder (: 10 Int64) (: 5 Int64))
                 (drop)
                 (output (: 15 Int64)) (live-objects 0))"#,
        )
        .unwrap();
        assert!(
            text.contains("drop-handle\t1\n"),
            "drop-handle line, got: {text:?}"
        );
        // A case without (drop) has no drop-handle line and drop_handle=false.
        let plain = read(
            r#"(case "y"
                 (input (do (def (main (: x Int64)) (+ x 1)) (export main)))
                 (call main (: 5 Int64)) (output (: 6 Int64)))"#,
        )
        .unwrap();
        assert!(!plain[0].trials[0].call.as_ref().unwrap().drop_handle);
    }

    /// A `(call-method <member> …)` clause sets `method` on the pending call (no export) and serializes to
    /// a `call-method\t<member>` line (plus `arg` lines), not a `call` line.
    #[test]
    fn a_call_method_clause_names_a_value_resource_member() {
        let recs = read(
            r#"(case "vm"
                 (input (do (def (main) ((. Bytes of) (list ((. UInt8 wrap) 65)))) (export main)))
                 (call-method len)
                 (output (: 1 UInt32)))"#,
        )
        .unwrap();
        let call = recs[0].trials[0].call.as_ref().unwrap();
        assert_eq!(call.method.as_deref(), Some("len"));
        assert!(call.export.is_empty(), "a method case has no export");
        let text = to_records(
            r#"(case "vm"
                 (input (do (def (main) ((. Bytes of) (list ((. UInt8 wrap) 65)))) (export main)))
                 (call-method len)
                 (output (: 1 UInt32)))"#,
        )
        .unwrap();
        assert!(
            text.contains("call-method\tlen\n"),
            "call-method line, got: {text:?}"
        );
        assert!(
            !text.contains("\ncall\t"),
            "a method case emits no call line, got: {text:?}"
        );
    }

    /// A `(peer "<iface>" <prog>)` clause records a separately-compiled peer component (interface +
    /// normalized provider program) on the record, and serializes to a `peer\t<iface>\t<program>` line —
    /// distinct from a `(module …)` (which links into the entry's component).
    #[test]
    fn a_peer_clause_records_a_cross_component_provider() {
        let src = r#"(case "x"
             (input (do (extern cadenza:math/api (op f (-> Int64 Int64)))
                        (def (main (: x Int64)) (* (cadenza:math/api.f x) 10)) (export main)))
             (peer "cadenza:math/api" (do (def (f (: x Int64)) (+ x 1)) (export f)))
             (call main (: 5 Int64))
             (output (: 60 Int64)))"#;
        let recs = read(src).unwrap();
        assert_eq!(recs[0].peers.len(), 1, "one peer recorded");
        assert_eq!(recs[0].peers[0].interface, "cadenza:math/api");
        assert!(
            recs[0].peers[0].program.contains("(export f)"),
            "peer program normalized: {}",
            recs[0].peers[0].program
        );
        assert!(
            !recs[0].peers[0].program_ast.is_empty(),
            "peer program AST built"
        );
        let text = to_records(src).unwrap();
        assert!(
            text.contains("peer\tcadenza:math/api\t"),
            "peer line serialized, got: {text:?}"
        );
    }

    /// The flat record stream emits one `call?`/`arg*`/`expect` group per trial (round-trips the shape).
    #[test]
    fn render_emits_a_group_per_trial() {
        let text = to_records(
            r#"(case "x"
                 (input (do (def (main (: b Bool)) b) (export main)))
                 (call main (: true Bool))  (output (: true Bool))
                 (call main (: false Bool)) (output (: false Bool)))"#,
        )
        .unwrap();
        // Two `expect` lines (one per trial) and two `call` lines.
        assert_eq!(
            text.matches("\nexpect\t").count() + text.starts_with("expect\t") as usize,
            2
        );
        assert_eq!(text.matches("call\t").count(), 2);
    }

    /// A `(live-objects N)` clause parses into `Record.live_objects` and renders as a `live-objects\t<N>`
    /// line; it is orthogonal to the value outcome (the trial's `(output …)` still stands).
    #[test]
    fn live_objects_clause_parses_and_renders() {
        let recs = read(
            r#"(case "x"
                 (input (do (def (main (: a Int64) (: b Int64)) (Int64.of (+ (BigInt.of a) (BigInt.of b)))) (export main)))
                 (call main (: 40 Int64) (: 2 Int64)) (output (: 42 Int64))
                 (live-objects 0))"#,
        )
        .unwrap();
        assert_eq!(recs[0].live_objects, Some(0));
        assert_eq!(recs[0].trials.len(), 1);
        let text = to_records(
            r#"(case "x"
                 (input (do (def (main (: a Int64) (: b Int64)) (Int64.of (+ (BigInt.of a) (BigInt.of b)))) (export main)))
                 (call main (: 40 Int64) (: 2 Int64)) (output (: 42 Int64))
                 (live-objects 0))"#,
        )
        .unwrap();
        assert!(text.contains("live-objects\t0\n"));
    }

    /// seq-15 PURE-BINARY: a `(live-objects known-leak)` marker sets the `known_leak` flag and renders as a
    /// bare `live-objects\tknown-leak` line (NO count). A legacy `(live-objects known-leak N)` still parses
    /// (the count is retained but IGNORED for grading) yet renders BARE too — so an un-migrated file grades
    /// binary and round-trips to the count-free form.
    #[test]
    fn live_objects_known_leak_marker_parses_and_renders() {
        // Bare marker (the migrated form).
        let bare = r#"(case "x"
                 (input (do (type L (Cons (Tuple Int64 L)) Nil) (def (main) (L.Cons (tuple 1 (L.Nil ())))) (export main)))
                 (call main) (output (: (L.Cons (tuple 1 (L.Nil ()))) L))
                 (live-objects known-leak))"#;
        let recs = read(bare).unwrap();
        assert!(recs[0].live_objects_known_leak);
        assert_eq!(recs[0].live_objects, None);
        assert!(
            to_records(bare)
                .unwrap()
                .contains("live-objects\tknown-leak\n")
        );
        // Legacy count-bearing marker: still parses the flag, but renders BARE (count dropped).
        let legacy = r#"(case "x"
                 (input (do (type L (Cons (Tuple Int64 L)) Nil) (def (main) (L.Cons (tuple 1 (L.Nil ())))) (export main)))
                 (call main) (output (: (L.Cons (tuple 1 (L.Nil ()))) L))
                 (live-objects known-leak 2))"#;
        let recs = read(legacy).unwrap();
        assert!(recs[0].live_objects_known_leak);
        let text = to_records(legacy).unwrap();
        assert!(
            text.contains("live-objects\tknown-leak\n"),
            "legacy renders bare: {text}"
        );
        assert!(!text.contains("known-leak\t2"), "count is dropped: {text}");
    }

    /// A CLEAN `(live-objects N1 N2 N3)` clause with 2+ counts parses PER-CALL: `live_objects` = the first,
    /// and `live_objects_per_call` = the whole list; it renders tab-separated (`live-objects\t0\t0\t0`) — the
    /// arm-dependent CLEAN residual a single count cannot express. (The known-leak marker is now count-free,
    /// so per-call counts are a CLEAN-case-only feature.)
    #[test]
    fn live_objects_per_call_positional_parses_and_renders() {
        let src = r#"(case "x"
                 (input (do (def (main (: r Int64)) r) (export main)))
                 (call main (: 1 Int64)) (output (: 1 Int64))
                 (call main (: 4 Int64)) (output (: 4 Int64))
                 (call main (: 0 Int64)) (output (: 0 Int64))
                 (live-objects 0 0 0))"#;
        let recs = read(src).unwrap();
        assert_eq!(recs[0].live_objects, Some(0)); // first count (uniform / direct-gate path)
        assert_eq!(recs[0].live_objects_per_call, Some(vec![0, 0, 0]));
        assert!(!recs[0].live_objects_known_leak);
        let text = to_records(src).unwrap();
        assert!(
            text.contains("live-objects\t0\t0\t0\n"),
            "per-call render: {text}"
        );
        // A single-count clause stays uniform (no per-call list).
        let uni = r#"(case "y" (input (do (def (main) 1) (export main))) (call main) (output (: 1 Int64)) (live-objects 0))"#;
        let recs = read(uni).unwrap();
        assert_eq!(recs[0].live_objects, Some(0));
        assert_eq!(recs[0].live_objects_per_call, None);
    }

    /// A case with NO `(live-objects …)` leaves the field `None` and emits no `live-objects` line.
    #[test]
    fn no_live_objects_clause_is_none() {
        let recs = read(
            r#"(case "x"
                 (input (do (def (main (: b Bool)) b) (export main)))
                 (call main (: true Bool)) (output (: true Bool)))"#,
        )
        .unwrap();
        assert_eq!(recs[0].live_objects, None);
        let text = to_records(
            r#"(case "x"
                 (input (do (def (main (: b Bool)) b) (export main)))
                 (call main (: true Bool)) (output (: true Bool)))"#,
        )
        .unwrap();
        assert!(!text.contains("live-objects"));
    }
}
