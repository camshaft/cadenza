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

pub mod markdown;

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

/// Parse a MIGRATED markdown corpus file's `text` into records — the same `Record`s [`read`] would
/// produce for the equivalent `.sexp`. It reconstructs the s-expression corpus from the markdown
/// (via [`markdown::to_sexpr`], the inverse of [`markdown::migrate`]) and reads that, so a `.md` and
/// its source `.sexp` yield an identical record stream — which is exactly what `markdown::check`
/// verifies. This is the reader the xtask gate uses for a migrated file.
pub fn read_markdown(text: &str) -> Result<Vec<Record>, String> {
    read(&markdown::to_sexpr(text)?)
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
        out.push_str("---\n");
    }
    out
}

/// Convenience: read + render in one step (what the `corpus` command emits).
pub fn to_records(text: &str) -> Result<String, String> {
    Ok(render(&read(text)?))
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
    })
}

/// Normalize a case's `input` occurrence to the runnable export shape, returning one-line s-expr text:
///   - `(do … (export …))` → unchanged
///   - `(module name def…)` → `(do def… (export main))`
///   - a bare expression `E` → `(do (def (main) E) (export main))`
fn normalize_program(a: &Arenas, input: StructId) -> String {
    match a.head_name(input) {
        // A `(do …)` input is EITHER a full program (it already declares `(export …)`) — passed verbatim
        // — OR a bare SEQUENCING-block VALUE (`(do 1 2 3)`, `(do (record …) 42)`), which is an expression
        // whose value is the program result: wrap it as `(do (def (main) <the-do>) (export main))`, the
        // same wrapping a bare expression gets (a `do` value-block is just an expression with a `do` head).
        Some("do") if do_block_has_export(a, input) => sexpr::print_from(a, input),
        Some("do") => {
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

/// The string a `Str` leaf carries, if `id` is one.
fn string_leaf(a: &Arenas, id: StructId) -> Option<String> {
    match a.get(id) {
        cadenza_syntax::ast::Struct::Atom(l) => match a.leaf(*l) {
            cadenza_syntax::ast::Leaf::Str(s) => Some(s.clone()),
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
