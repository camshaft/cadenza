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
//!   expect\t(output <value-form>) | (error <CODE>) | (trap <reason>) | (declines)
//!   ---

/// The command surface (`CorpusArgs` + `run`), embeddable so the unified `cdz` binary can mount
/// `cdz corpus`. The standalone `cdz-corpus` bin is a thin shim over it.
pub mod cli;

use cadenza_syntax::ast::{Arenas, Builder, StructId};
use cadenza_syntax::sexpr;

/// A single parsed + normalized corpus case, ready to run.
pub struct Record {
    pub description: String,
    /// The `input` rewritten to the runnable export shape, as one-line s-expression text.
    pub program: String,
    /// Sibling LIBRARY modules of a multi-file PACKAGE case (`DESIGN-package-linking.md`), each a
    /// `(name, program-text)` from a `(module "name" <prog>)` clause — the files the ENTRY (`program`,
    /// named `main`) may `(import …)` from. Empty for the common single-file case (then `program` is
    /// compiled alone, exactly as before). When non-empty, the gate driver writes every module + the
    /// entry to a temp dir and runs `cdz compile <files> --entry main`.
    pub modules: Vec<Module>,
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
}

/// One sibling LIBRARY module of a multi-file package case — its file name (the string an `(import
/// "name" …)` names it by) and its program text, normalized to the runnable `(do … )` shape like the
/// entry. A `(module "name" <prog>)` clause produces one of these.
pub struct Module {
    /// The file name (the `(import "name" …)` target).
    pub name: String,
    /// The module's program, as one-line s-expression text (same normalization as the entry program).
    pub program: String,
}

/// One (call, expected-result) pair of a case — a single run of the compiled program.
pub struct Trial {
    /// The `(call <export> <arg>…)` for this trial, or `None` to invoke the sole export with no args.
    pub call: Option<Call>,
    /// The recorded oracle result for this trial: `Output(value-form)`, `Error(code)`, or `Trap(reason)`.
    pub expect: Expect,
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
}

/// The recorded primary result of a case — exactly one per the corpus vocabulary.
pub enum Expect {
    /// `(output (: <value> <Type>))` — the value the run produces, as its canonical value-form text.
    Output(String),
    /// `(error <CODE>)` (or a `(compiler (error <CODE>))` for a provable-at-compile-time trap) — the
    /// diagnostic code the compiler must reject with.
    /// The optional second field is a load-bearing SUBSTRING of the diagnostic MESSAGE the corpus pins
    /// (`(error <CODE> (message "phrase"))`), the portable-diagnostic-test capability (operator seq353):
    /// the gate additionally requires the emitted diagnostic to CONTAIN that phrase. `None` = code-only.
    Error(String, Option<String>),
    /// `(trap "<reason>")` — the run halts with this reason.
    Trap(String),
    /// `(declines)` — the compiler DECLINES to emit a component for this program: a well-formed program
    /// whose shape the compiler does not (yet) realize, so it produces no artifact rather than a value
    /// or a coded rejection (`reference-compiler.md` §A "No" Is A First-Class Value Produced Where The
    /// Decision Is Made — decline is a first-class outcome alongside reject and trap). The DISTINCTION
    /// from `(error CODE)`: an `error` is a coded well-formedness REJECTION (the program is ill-formed);
    /// a `declines` is a CODELESS decline (the program is well-formed, the compiler cannot realize its
    /// shape — e.g. a type with no boundary representation, per `component-abi.md` §A Type That Has No
    /// Defined Boundary Representation Must Not Appear In An Exported Or Imported Signature). Grades Pass
    /// when the compiler declines, Fail when it emits (the "declines rather than miscompiles" property).
    /// The optional field is a load-bearing SUBSTRING of the decline's diagnostic MESSAGE the corpus pins
    /// (`(declines (message "phrase"))`) — the gate additionally requires the decline diagnostic to
    /// CONTAIN that phrase (operator seq353). `None` = any decline passes (the historical behavior).
    Declines(Option<String>),
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
    /// The single `(kickoff <alias> (inbound <family> <value>))` — the one event that seeds the run.
    pub kickoff: Kickoff,
    /// Ordered `(expect-effects (effect (from <a>) (family <f>) <value>?)…)` — each emitted effect the
    /// run must produce, in stream order (order-verified, like `host_calls`). Value-form optional (an
    /// effect with no payload omits it).
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
    /// Per-alias `(events-processed <alias> <n>)` — the total processed-log length the session must reach
    /// (grades `Session::event_count()`).
    pub events_processed: Vec<(String, String)>,
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
#[derive(Debug)]
pub struct ExpectEffect {
    pub from: String,
    pub family: String,
    pub value: Option<String>,
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
/// diagnostic-message pin (operator seq353) shared by `(error …)` and `(declines …)`. `None` when no
/// well-formed `(message STR)` child is present.
fn message_clause(a: &Arenas, tail: &[StructId]) -> Option<String> {
    tail.iter().find_map(|&child| {
        a.as_form(child, "message")
            .and_then(|t| t.first().copied())
            .and_then(|id| string_leaf(a, id))
    })
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
    for &case_id in top {
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
    for &case_id in top {
        if arenas.head_name(case_id) == Some("platform-case") {
            records.push(parse_platform_case(&arenas, case_id)?);
        }
    }
    Ok(records)
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
        // One group of lines per TRIAL: its `call`/`arg` lines (if any) then its `expect`, which ends
        // the trial. A single-trial case emits exactly the historical `call?`/`arg*`/`expect` shape.
        for trial in &r.trials {
            if let Some(call) = &trial.call {
                out.push_str("call\t");
                out.push_str(&call.export);
                out.push('\n');
                for arg in &call.args {
                    out.push_str("arg\t");
                    out.push_str(arg);
                    out.push('\n');
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
                Expect::Error(code, message) => {
                    out.push_str("error ");
                    out.push_str(code);
                    if let Some(m) = message {
                        out.push_str(" (message \"");
                        out.push_str(m);
                        out.push_str("\")");
                    }
                }
                Expect::Trap(reason) => {
                    out.push_str("trap ");
                    out.push_str(reason);
                }
                // `declines`, plus ` (message "phrase")` when the case pins the decline's diagnostic prose;
                // bare `declines` (byte-identical to before) when it does not.
                Expect::Declines(message) => {
                    out.push_str("declines");
                    if let Some(m) = message {
                        out.push_str(" (message \"");
                        out.push_str(m);
                        out.push_str("\")");
                    }
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
///   `serves\t<alias>\t<family>` (0+) · `kickoff\t<alias>\t<inbound>\t<value>` (1) ·
///   `expect-effect\t<from>\t<family>[\t<value>]` (0+, order) ·
///   `expect-message\t<from>\t<to>\t<family>\t<value>` (0+, order) ·
///   `expect-delivery-failure\t<from>\t<to>` (0+) · `end-kv\t<alias>\t<key>\t<value>` (0+) ·
///   `end-status\t<alias>\t<status>` (0+) · `events-processed\t<alias>\t<n>` (0+) · `---` terminator.
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
        out.push_str("kickoff\t");
        out.push_str(&r.kickoff.alias);
        out.push('\t');
        out.push_str(&r.kickoff.inbound);
        out.push('\t');
        out.push_str(&r.kickoff.value);
        out.push('\n');
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
        for (alias, n) in &r.events_processed {
            out.push_str("events-processed\t");
            out.push_str(alias);
            out.push('\t');
            out.push_str(n);
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
    // `(case "<desc>" <clause>…)` — the description is the first string child.
    let description = items
        .get(1)
        .and_then(|&id| string_leaf(a, id))
        .ok_or("case has no description string")?;

    let mut input: Option<StructId> = None;
    let mut modules: Vec<Module> = Vec::new();
    let mut host_responses: Vec<(String, String)> = Vec::new();
    let mut host_calls: Vec<String> = Vec::new();
    let mut warns: Vec<(String, Option<String>)> = Vec::new();
    // Trials accumulate as the clauses are walked: a `(call …)` sets the PENDING call, and the next
    // result clause (`output`/`error`/`trap`) CLOSES a trial pairing that pending call with the result.
    // A result with no preceding `(call …)` is a no-call trial. This lets a case INTERLEAVE several
    // `(call …) (output …)` pairs — each result closes one trial — while a single-result case (the
    // common shape) yields exactly one trial. A `(compiler (error …))` overrides the current trial's
    // result with the compile-time rejection (it accompanies a dynamic `(trap …)`).
    let mut trials: Vec<Trial> = Vec::new();
    let mut pending_call: Option<Call> = None;

    for &clause in &items[2..] {
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
                    modules.push(Module {
                        name,
                        program: normalize_program(a, prog),
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
                    });
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
                    });
                }
            }
            Some("error") => {
                // `(error <CODE>)` or `(error <CODE> (message "phrase"))` — closes a trial with a
                // compile-time rejection code, optionally pinning a substring of the diagnostic message.
                if let Some(tail) = a.as_form(clause, "error")
                    && let Some(code) = tail
                        .first()
                        .copied()
                        .and_then(|id| a.as_name(id).map(str::to_string))
                {
                    let message = message_clause(a, tail);
                    trials.push(Trial {
                        call: pending_call.take(),
                        expect: Expect::Error(code, message),
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
                    });
                }
            }
            Some("declines") => {
                // `(declines)` or `(declines (message "phrase"))` — closes a trial that must produce NO
                // artifact: the compiler declines (codelessly). The optional message pins a substring of
                // the decline's diagnostic prose (so it must NAME the actionable reason, not just refuse).
                let message = a
                    .as_form(clause, "declines")
                    .and_then(|tail| message_clause(a, tail));
                trials.push(Trial {
                    call: pending_call.take(),
                    expect: Expect::Declines(message),
                });
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
                    let message = message_clause(a, inner_tail);
                    if let Some(last) = trials.last_mut() {
                        last.expect = Expect::Error(code, message);
                    } else {
                        trials.push(Trial {
                            call: pending_call.take(),
                            expect: Expect::Error(code, message),
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
                    warns.push((code, message_clause(a, tail)));
                }
            }
            // `doc` — not needed to run + compare a case.
            _ => {}
        }
    }

    let input = input.ok_or_else(|| format!("case {description:?} has no (input …)"))?;
    let program = normalize_program(a, input);

    if trials.is_empty() {
        return Err(format!("case {description:?} has no primary result clause"));
    }

    Ok(Record {
        description,
        program,
        modules,
        trials,
        host_responses,
        host_calls,
        warns,
    })
}

/// Parse a `(platform-case "title" <clause>…)` into a [`PlatformRecord`]. Mirrors [`parse_case`]'s
/// clause walk. Clauses (all optional except a kickoff, which the fixpoint needs to start):
///   `(doc "…")` · `(session <alias> (reducer <prog>) (serves <family>…)?)` (1+) ·
///   `(kickoff <alias> (inbound <family> <value>))` (exactly 1) ·
///   `(expect-effects (effect (from <a>) (family <f>) <value>?)…)` (ordered) ·
///   `(expect-messages (message (from <a>) (to <b>) (family <f>) <value>)…)` (ordered) ·
///   `(expect-delivery-failure (from <a>) (to <b>))` (0+) ·
///   `(end-state <alias> (kv <key> <value>)… (status <state>)?)` · `(events-processed <alias> <n>)`.
fn parse_platform_case(a: &Arenas, case_id: StructId) -> Result<PlatformRecord, String> {
    let items = match a.get(case_id) {
        cadenza_syntax::ast::Struct::List(items) => items,
        _ => return Err("platform-case is not a list".into()),
    };
    let title = items
        .get(1)
        .and_then(|&id| string_leaf(a, id))
        .ok_or("platform-case has no title string")?;

    let mut doc: Option<String> = None;
    let mut sessions: Vec<PlatformSession> = Vec::new();
    let mut kickoff: Option<Kickoff> = None;
    let mut expect_effects: Vec<ExpectEffect> = Vec::new();
    let mut expect_messages: Vec<ExpectMessage> = Vec::new();
    let mut expect_delivery_failures: Vec<(String, String)> = Vec::new();
    let mut end_kv: Vec<(String, String, String)> = Vec::new();
    let mut end_status: Vec<(String, String)> = Vec::new();
    let mut events_processed: Vec<(String, String)> = Vec::new();

    for &clause in &items[2..] {
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
            // `(kickoff <alias> (inbound <family> <value>))` — the single seed event.
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
                    kickoff = Some(Kickoff {
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
                                let value = etail
                                    .iter()
                                    .find(|&&c| {
                                        a.head_name(c) != Some("from")
                                            && a.head_name(c) != Some("family")
                                    })
                                    .map(|&v| value_form_text(a, v));
                                expect_effects.push(ExpectEffect {
                                    from,
                                    family,
                                    value,
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
            // `(end-state <alias> (kv <key> <value>)… (status <state>)?)` — per-session end assertions.
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
            _ => {}
        }
    }

    let kickoff = kickoff.ok_or_else(|| format!("platform-case {title:?} has no (kickoff …)"))?;
    if sessions.is_empty() {
        return Err(format!("platform-case {title:?} has no (session …)"));
    }

    Ok(PlatformRecord {
        title,
        doc,
        sessions,
        kickoff,
        expect_effects,
        expect_messages,
        expect_delivery_failures,
        end_kv,
        end_status,
        events_processed,
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
    match a.head_name(input) {
        // A `(do …)` input that ALREADY declares `(export …)` is a full program — passed verbatim. A
        // `(do …)` WITHOUT an export is a bare SEQUENCING-block VALUE (`(do 1 2 3)`, `(do (record …) 42)`),
        // an expression whose value is the program result: it falls through to the `_` arm below and is
        // wrapped as `(do (def (main) <the-do>) (export main))`, exactly like any other bare expression (a
        // `do` value-block is just an expression with a `do` head — no separate arm needed).
        Some("do") if do_block_has_export(a, input) => sexpr::print_from(a, input),
        Some("module") => {
            // Rebuild `(do <module's forms after the name> (export main))` in a fresh arena.
            let forms = match a.get(input) {
                cadenza_syntax::ast::Struct::List(items) => &items[2..], // skip `module` head + the name
                _ => &[][..],
            };
            let mut b = Builder::new();
            let do_head = b.name("do");
            let mut children = vec![do_head];
            for &f in forms {
                children.push(clone_into(a, f, &mut b));
            }
            children.push(export_main(&mut b));
            let root = b.list(children);
            sexpr::print(&b.finish(root))
        }
        _ => {
            // Bare expression E → (do (def (main) E) (export main)).
            let mut b = Builder::new();
            let do_head = b.name("do");
            let def_head = b.name("def");
            let main_name = b.name("main");
            let main_sig = b.list(vec![main_name]);
            let e = clone_into(a, input, &mut b);
            let def_main = b.list(vec![def_head, main_sig, e]);
            let export = export_main(&mut b);
            let root = b.list(vec![do_head, def_main, export]);
            sexpr::print(&b.finish(root))
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

    /// A single-result case (the common shape) parses to ONE trial — no call, one output.
    #[test]
    fn a_single_result_case_is_one_trial() {
        let recs = read(r#"(case "x" (input 5) (output (: 5 Int64)))"#).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].trials.len(), 1);
        assert!(recs[0].trials[0].call.is_none());
        assert!(matches!(&recs[0].trials[0].expect, Expect::Output(v) if v == "(: 5 Int64)"));
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
        assert_eq!(r.kickoff.alias, "worker");
        assert_eq!(r.kickoff.inbound, "start");
        assert_eq!(r.kickoff.value, "(: unit Unit)");
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

    /// A `(declines)` clause parses to a `Declines` expectation — a payloadless trial (no call, no value).
    #[test]
    fn a_declines_clause_is_a_declines_expectation() {
        let recs = read(
            r#"(case "x"
                 (input (do (def (mk) (fn ((: x Int64)) unit)) (export mk)))
                 (declines))"#,
        )
        .unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].trials.len(), 1);
        assert!(recs[0].trials[0].call.is_none());
        assert!(matches!(&recs[0].trials[0].expect, Expect::Declines(_)));
    }

    /// A `(declines)` renders to a bare `expect\tdeclines` line (no payload after the keyword).
    #[test]
    fn render_emits_a_bare_declines_expect_line() {
        let text = to_records(
            r#"(case "x"
                 (input (do (def (mk) (fn ((: x Int64)) unit)) (export mk)))
                 (declines))"#,
        )
        .unwrap();
        assert!(
            text.contains("expect\tdeclines\n"),
            "declines renders as a bare keyword line, got: {text:?}"
        );
    }

    /// A `(declines)` MAY pair with a `(call …)` (the trial's call is recorded, the expectation is a
    /// decline) — so a case that drives a specific export can still pin the decline outcome.
    #[test]
    fn a_declines_clause_pairs_with_a_pending_call() {
        let recs = read(
            r#"(case "x"
                 (input (do (def (mk (: xs (List Int64))) (fn ((: i Int64)) ((. List len) xs))) (export mk)))
                 (call mk (: 5 Int64))
                 (declines))"#,
        )
        .unwrap();
        assert_eq!(recs[0].trials.len(), 1);
        assert_eq!(recs[0].trials[0].call.as_ref().unwrap().export, "mk");
        assert!(matches!(&recs[0].trials[0].expect, Expect::Declines(_)));
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
}
