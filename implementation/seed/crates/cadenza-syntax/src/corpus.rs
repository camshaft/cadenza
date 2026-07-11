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
//!   needs\t<capability>            (zero or more; omitted when the case is core)
//!   ---

use crate::ast::{Arenas, Builder, StructId};
use crate::sexpr;

/// A single parsed + normalized corpus case, ready to run.
pub struct Record {
    pub description: String,
    /// The `input` rewritten to the runnable export shape, as one-line s-expression text.
    pub program: String,
    /// A `(call <export> <arg>…)` clause, if the case supplies runtime arguments to its entrypoint —
    /// the export to invoke plus the argument values to pass. `None` for the common nullary case
    /// (the driver invokes the sole export with no arguments).
    pub call: Option<Call>,
    /// The recorded oracle result: `Output(value-form)`, `Error(code)`, or `Trap(reason)`.
    pub expect: Expect,
    /// Capabilities the case declares via `(needs …)` — a generation runs it only if it realizes them.
    pub needs: Vec<String>,
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
    Error(String),
    /// `(trap "<reason>")` — the run halts with this reason.
    Trap(String),
}

/// Parse a corpus file's `text` into records. Returns an error only if the file itself does not
/// parse as s-expressions; a malformed individual case is reported as an error record inline.
pub fn read(text: &str) -> Result<Vec<Record>, String> {
    let arenas = sexpr::read_all(text).map_err(|e| format!("corpus parse error: {}", e.0))?;
    // `read_all` wraps every top-level form under a synthetic `(do …)`; the cases are its children.
    let top = match arenas.get(arenas.root) {
        crate::ast::Struct::List(items) => &items[1..], // skip the synthetic `do` head
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
        if let Some(call) = &r.call {
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
        match &r.expect {
            Expect::Output(v) => {
                out.push_str("output ");
                out.push_str(v);
            }
            Expect::Error(code) => {
                out.push_str("error ");
                out.push_str(code);
            }
            Expect::Trap(reason) => {
                out.push_str("trap ");
                out.push_str(reason);
            }
        }
        out.push('\n');
        for cap in &r.needs {
            out.push_str("needs\t");
            out.push_str(cap);
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
        crate::ast::Struct::List(items) => items,
        _ => return Err("case is not a list".into()),
    };
    // `(case "<desc>" <clause>…)` — the description is the first string child.
    let description = items
        .get(1)
        .and_then(|&id| string_leaf(a, id))
        .ok_or("case has no description string")?;

    let mut input: Option<StructId> = None;
    let mut call: Option<Call> = None;
    let mut output: Option<String> = None;
    let mut error: Option<String> = None;
    let mut compiler_error: Option<String> = None;
    let mut trap: Option<String> = None;
    let mut needs: Vec<String> = Vec::new();

    for &clause in &items[2..] {
        match a.head_name(clause) {
            Some("input") => {
                input = a.as_form(clause, "input").and_then(|t| t.first().copied());
            }
            Some("call") => {
                // `(call <export> <arg>…)` — the export to invoke plus its runtime arguments. The
                // export is the first child (a name); each remaining child is an argument value-form,
                // reduced to its bare value text (the runner coerces it to the declared param type).
                if let Some(tail) = a.as_form(clause, "call")
                    && let Some(&export_id) = tail.first()
                    && let Some(export) = a.as_name(export_id)
                {
                    let args = tail[1..].iter().map(|&arg| value_of(a, arg)).collect();
                    call = Some(Call {
                        export: export.to_string(),
                        args,
                    });
                }
            }
            Some("output") => {
                // `(output (: <value> <Type>))` — record the value-form's canonical text. The value
                // is the first child of the `:` form; we keep the whole `(: value Type)` text so the
                // driver can compare against however the run renders (value alone is the common case).
                output = a
                    .as_form(clause, "output")
                    .and_then(|t| t.first().copied())
                    .map(|form| value_form_text(a, form));
            }
            Some("error") => {
                error = a
                    .as_form(clause, "error")
                    .and_then(|t| t.first().copied())
                    .and_then(|id| a.as_name(id).map(str::to_string));
            }
            Some("trap") => {
                trap = a
                    .as_form(clause, "trap")
                    .and_then(|t| t.first().copied())
                    .and_then(|id| string_leaf(a, id));
            }
            Some("compiler") => {
                // `(compiler (error <CODE>))` — a provable-at-compile-time rejection accompanying a
                // dynamic `(trap …)`. The compiler's recorded outcome is the rejection.
                if let Some(inner) = a
                    .as_form(clause, "compiler")
                    .and_then(|t| t.first().copied())
                    && a.head_name(inner) == Some("error")
                {
                    compiler_error = a
                        .as_form(inner, "error")
                        .and_then(|t| t.first().copied())
                        .and_then(|id| a.as_name(id).map(str::to_string));
                }
            }
            Some("needs") => {
                if let Some(cap) = a
                    .as_form(clause, "needs")
                    .and_then(|t| t.first().copied())
                    .and_then(|id| a.as_name(id))
                {
                    needs.push(cap.to_string());
                }
            }
            // `doc`, `host-calls`, `host-responses` — not needed to run + compare a scalar case yet.
            _ => {}
        }
    }

    let input = input.ok_or_else(|| format!("case {description:?} has no (input …)"))?;
    let program = normalize_program(a, input);

    // Primary result precedence: a compile-time rejection (compiler-error, or a primary error) is the
    // compiler's recorded outcome; else the terminal output/trap.
    let expect = if let Some(code) = compiler_error.or(error) {
        Expect::Error(code)
    } else if let Some(v) = output {
        Expect::Output(v)
    } else if let Some(reason) = trap {
        Expect::Trap(reason)
    } else {
        return Err(format!("case {description:?} has no primary result clause"));
    };

    Ok(Record {
        description,
        program,
        call,
        expect,
        needs,
    })
}

/// Normalize a case's `input` occurrence to the runnable export shape, returning one-line s-expr text:
///   - `(do … (export …))` → unchanged
///   - `(module name def…)` → `(do def… (export main))`
///   - a bare expression `E` → `(do (def (main) E) (export main))`
fn normalize_program(a: &Arenas, input: StructId) -> String {
    match a.head_name(input) {
        Some("do") => sexpr::print_from(a, input),
        Some("module") => {
            // Rebuild `(do <module's forms after the name> (export main))` in a fresh arena.
            let forms = match a.get(input) {
                crate::ast::Struct::List(items) => &items[2..], // skip `module` head + the name
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

/// Build `(export main)` in `b`.
fn export_main(b: &mut Builder) -> StructId {
    let export = b.name("export");
    let main = b.name("main");
    b.list(vec![export, main])
}

/// Deep-clone occurrence `id` from `a` into builder `b`, returning the new occurrence id.
fn clone_into(a: &Arenas, id: StructId, b: &mut Builder) -> StructId {
    match a.get(id) {
        crate::ast::Struct::Atom(l) => {
            let leaf = a.leaf(*l).clone();
            b.atom_leaf(leaf)
        }
        crate::ast::Struct::List(items) => {
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

/// The string a `Str` leaf carries, if `id` is one.
fn string_leaf(a: &Arenas, id: StructId) -> Option<String> {
    match a.get(id) {
        crate::ast::Struct::Atom(l) => match a.leaf(*l) {
            crate::ast::Leaf::Str(s) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}
