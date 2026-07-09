//! The behavior gate: compile every executable-semantics case the seed realizes with
//! `cdz-rustc`, run the resulting component, and confirm its observable behavior equals the
//! case's recorded result (the corpus IS the oracle — constitution IX/XIV as amended
//! 2026-07-04). A case the compiler cannot yet lower is reported as `todo`, an honest
//! backlog entry, not a failure.

use cdz_compiler::ast::{self, Node};
use cdz_compiler::codegen;
use crate::host::{self, RunOutcome};

/// The capabilities the seed realizes, used to filter `(needs …)` cases. Cases needing an
/// unrealized capability are skipped by both gates.
const REALIZED: &[&str] = &[
    "collections",
    "bytes",
    "sum-type-declaration",
    "fallible-access",
    "list-growth",
    "effects",
    "boolean-connectives",
    // Element patterns `(list)`, `(list a b)`, `(list x .. rest)` over an inline/const-foldable
    // list scrutinee (core-semantics.md §A List Is Deconstructed By Element Patterns With An
    // Optional Rest). The RUNTIME case — a recursive fold over a parameter list, needing a
    // materialized list tail for the rest binder — is gated separately behind
    // `list-pattern-runtime-tail` until the list-tail primitive lands.
    "list-patterns",
    // The `Map.*` operation surface (empty/insert/lookup/remove/size + swap/take) over a
    // compile-time-known map, const-folded (collections-and-text.md §A Map Is Built By Functional
    // Construction, §Keys Are Compared By Value, §A Map Renders As Its Entries In Canonical Key
    // Order). The RUNTIME-map case (a map built from parameter/call values) is a later increment;
    // `map-patterns` (ask-61) is a separate phase, still gated.
    "maps",
];

/// One parsed case.
pub struct Case {
    pub description: String,
    pub input: Node,
    pub primary: PrimaryClause,
    pub events: Option<Vec<(String, Node)>>,
    pub needs: Vec<String>,
    /// The `(compiler (error CDZ####))` clause, if the case records one: the diagnostic a
    /// static compiler MUST reject with, where a dynamic interpreter would instead exhibit
    /// the `primary` clause. Since the seed is now a compiler (constitution VII, Amendment
    /// 0.4.0 — no dynamic carve-out), cdz-rustc is judged against THIS clause when present.
    pub compiler_error: Option<String>,
    /// The `(host-calls (call NAME (: arg Type)…) …)` clause: the ordered host calls the run MUST
    /// make (each = name + argument value forms). `Some(empty)` asserts NO host call was made;
    /// `None` means the case does not pin host calls.
    pub host_calls: Option<Vec<(String, Vec<Node>)>>,
    /// The `(host-responses (respond NAME (: value Type))…)` clause: the responses the host feeds
    /// back in call order (capabilities-and-effects.md §Suspension Is Replay From The Host's Log).
    pub host_responses: Vec<Node>,
}

/// The primary result clause — what the executable semantics records for this case.
pub enum PrimaryClause {
    Output(Node), // (: <value> <Type>)
    Trap(String),
    Exhausted,
    Error(String), // a front-end diagnostic code, e.g. CDZ0101
}

/// A case as loaded from a corpus file: parsed, or malformed with a reason.
pub enum CaseLoad {
    Parsed(Case),
    Malformed { description: String, error: String },
}

/// The first `(needs …)` capability a case requires that the seed does not realize, or
/// `None` if the seed realizes them all (so the case is runnable).
pub fn first_unrealized(needs: &[String]) -> Option<String> {
    needs.iter().find(|n| !REALIZED.contains(&n.as_str())).cloned()
}

/// Load every case in the corpus (all `.sexp` files under `dir`), file-sorted, in-file
/// order. Malformed cases are surfaced, not dropped.
pub fn load_cases(dir: &str) -> std::io::Result<Vec<CaseLoad>> {
    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map_or(false, |x| x == "sexp"))
        .collect();
    files.sort();

    let mut loads = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path)?;
        let nodes = match ast::read_all(&text) {
            Ok(ns) => ns,
            Err(e) => {
                loads.push(CaseLoad::Malformed {
                    description: format!("{}: PARSE ERROR", path.display()),
                    error: e.to_string(),
                });
                continue;
            }
        };
        for node in &nodes {
            if node.head_name() == Some("case") {
                match parse_case(node) {
                    Ok(c) => loads.push(CaseLoad::Parsed(c)),
                    Err(e) => loads.push(CaseLoad::Malformed {
                        description: "malformed case".into(),
                        error: e,
                    }),
                }
            }
        }
    }
    Ok(loads)
}

fn parse_case(node: &Node) -> Result<Case, String> {
    let items = match node {
        Node::List(items) => items,
        _ => return Err("case is not a list".into()),
    };
    let description = match items.get(1) {
        Some(Node::Str(s)) => s.clone(),
        _ => return Err("case missing description".into()),
    };
    let mut input: Option<Node> = None;
    let mut primary: Option<PrimaryClause> = None;
    let mut events: Option<Vec<(String, Node)>> = None;
    let mut needs: Vec<String> = Vec::new();
    let mut compiler_error: Option<String> = None;
    let mut host_calls: Option<Vec<(String, Vec<Node>)>> = None;
    let mut host_responses: Vec<Node> = Vec::new();

    for clause in &items[2..] {
        match clause.head_name() {
            // (compiler (error CDZ####)) — the diagnostic a static compiler rejects with.
            Some("compiler") => {
                if let Node::List(c) = clause {
                    if let Some(inner) = c.get(1) {
                        if inner.head_name() == Some("error") {
                            if let Node::List(e) = inner {
                                if let Some(Node::Name(code)) = e.get(1) {
                                    compiler_error = Some(code.clone());
                                }
                            }
                        }
                    }
                }
            }
            Some("input") => {
                if let Node::List(c) = clause {
                    input = Some(c[1].clone());
                }
            }
            Some("output") => {
                if let Node::List(c) = clause {
                    primary = Some(PrimaryClause::Output(c[1].clone()));
                }
            }
            Some("trap") => {
                if let Node::List(c) = clause {
                    if let Some(Node::Str(reason)) = c.get(1) {
                        primary = Some(PrimaryClause::Trap(reason.clone()));
                    }
                }
            }
            Some("exhausted") => primary = Some(PrimaryClause::Exhausted),
            Some("error") => {
                if let Node::List(c) = clause {
                    if let Some(Node::Name(code)) = c.get(1) {
                        primary = Some(PrimaryClause::Error(code.clone()));
                    }
                }
            }
            Some("events") => {
                if let Node::List(c) = clause {
                    let mut evs = Vec::new();
                    for ev in &c[1..] {
                        if let Node::List(ep) = ev {
                            if let Some(Node::Str(kind)) = ep.get(1) {
                                let payload = ep.get(2).cloned().unwrap_or(Node::Name("unit".into()));
                                evs.push((kind.clone(), payload));
                            }
                        }
                    }
                    events = Some(evs);
                }
            }
            Some("needs") => {
                if let Node::List(c) = clause {
                    if let Some(Node::Name(cap)) = c.get(1) {
                        needs.push(cap.clone());
                    }
                }
            }
            // (host-calls (call NAME (: arg Type)…) …) — the ordered host calls the run must make.
            // An empty `(host-calls)` asserts none were made.
            Some("host-calls") => {
                if let Node::List(c) = clause {
                    let mut calls = Vec::new();
                    for call in &c[1..] {
                        if let Node::List(cp) = call {
                            if matches!(cp.first(), Some(Node::Name(h)) if h == "call") {
                                // The call name `log.emit` reads as the dotted tree `(. log emit)`
                                // (reader sugar); recover its flat `effect.op` string. Each remaining
                                // element is an arg value form `(: v T)` (or a bare value); keep the
                                // whole form for rendering comparison.
                                if let Some(nm) = dotted_flat_name(cp.get(1)) {
                                    let args: Vec<Node> = cp[2..].to_vec();
                                    calls.push((nm, args));
                                }
                            }
                        }
                    }
                    host_calls = Some(calls);
                }
            }
            // (host-responses (respond NAME (: value Type))…) — responses fed back in call order.
            // NAME is the dotted `effect.op` (parsed but not asserted here — responses are keyed by
            // position, not name; the value at index 2 is what the host feeds back).
            Some("host-responses") => {
                if let Node::List(c) = clause {
                    for r in &c[1..] {
                        if let Node::List(rp) = r {
                            if matches!(rp.first(), Some(Node::Name(h)) if h == "respond") {
                                if let Some(v) = rp.get(2) {
                                    host_responses.push(v.clone());
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(Case {
        description,
        input: input.ok_or("case missing (input …)")?,
        primary: primary.ok_or("case missing a primary clause")?,
        events,
        needs,
        compiler_error,
        host_calls,
        host_responses,
    })
}

/// The verdict of running one case through the behavior gate.
pub enum CaseStatus {
    /// Compiled, ran, and matched the recorded result.
    Passed,
    /// Needs a capability the seed does not realize.
    Skipped(String),
    /// The compiler cannot yet lower this construct (an honest backlog entry).
    Todo(String),
    /// Compiled and ran, but the observable behavior contradicts the record. FAILING.
    Failed { expected: String, observed: String },
}

pub struct CaseResult {
    pub description: String,
    pub status: CaseStatus,
}

/// Run the whole corpus through the behavior gate.
pub fn run_corpus(dir: &str) -> std::io::Result<Vec<CaseResult>> {
    let loads = load_cases(dir)?;
    // The compiler MUST NOT panic on any input — a panic is a defect, not a defined outcome
    // (self-hosting-and-bootstrap.md §"An Unsupported Construct Is Declined, Not Miscompiled":
    // an unhandled construct is *declined*, never a crash). Silence the default panic hook for
    // the gate run so a caught panic is reported as a per-case FAIL rather than aborting the
    // whole gate and spewing a backtrace; `run_case_guarded` catches it below.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut results = Vec::new();
    for load in loads {
        match load {
            CaseLoad::Parsed(c) => results.push(run_case_guarded(c)),
            CaseLoad::Malformed { description, error } => results.push(CaseResult {
                description,
                status: CaseStatus::Failed { expected: "well-formed case".into(), observed: error },
            }),
        }
    }
    std::panic::set_hook(prev_hook);
    Ok(results)
}

/// Run one case, catching any panic the compiler raises and reporting it as a FAIL. The
/// compiler must never panic (it declines or rejects), so a caught panic is a genuine
/// contradiction of the "decline, not crash" invariant — surfaced here per-case rather than
/// aborting the gate.
fn run_case_guarded(case: Case) -> CaseResult {
    let description = case.description.clone();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_case(case))) {
        Ok(result) => result,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".to_string());
            CaseResult {
                description,
                status: CaseStatus::Failed {
                    expected: "compiler declines or rejects (never panics)".into(),
                    observed: format!("compiler PANICKED: {}", first_line(&msg)),
                },
            }
        }
    }
}

/// Wrap a case's input as a program for the OLD compiler: a whole `(module …)` verbatim, or a bare
/// expression as the body of a nullary `main` in a throwaway module. (The old compiler detects the
/// entry by the `main`/`compile` name convention.)
pub fn as_program(input: &Node) -> Node {
    if input.head_name() == Some("module") {
        input.clone()
    } else {
        Node::List(vec![
            Node::Name("module".into()),
            Node::Name("case".into()),
            Node::List(vec![
                Node::Name("def".into()),
                Node::List(vec![Node::Name("main".into())]),
                input.clone(),
            ]),
        ])
    }
}

/// Wrap a case's input as a program for the NEW compiler (rcdzc): the implicit-module `(do …)` shape
/// with an explicit `(export …)`. Bridges the CURRENT corpus (bare exprs / old `(module …)`) to the
/// new shape so rcdzc can grade the existing cases before the corpus files are refactored:
/// - a bare expression `E` → `(do (def (main) E) (export main))`;
/// - an old `(module name def…)` → `(do def… (export main))` (its `main` becomes the export);
/// - an already-new `(do … (export …))` → passed through unchanged.
/// The ABI is derived from the exported function's signature, never its name — `main` here is just
/// the conventional entry name the current corpus uses, made explicit as an export.
pub fn as_program_v2(input: &Node) -> Node {
    // Already the new shape (a `do` sequence containing an `(export …)`): pass through.
    if input.head_name() == Some("do") {
        return input.clone();
    }
    let export_main = Node::List(vec![Node::Name("export".into()), Node::Name("main".into())]);
    if input.head_name() == Some("module") {
        // `(module name def…)` → `(do def… (export main))`.
        if let Node::List(items) = input {
            let mut forms: Vec<Node> = items[2..].to_vec();
            forms.push(export_main);
            let mut do_form = vec![Node::Name("do".into())];
            do_form.append(&mut forms);
            return Node::List(do_form);
        }
    }
    // A bare expression → `(do (def (main) E) (export main))`.
    let def_main = Node::List(vec![
        Node::Name("def".into()),
        Node::List(vec![Node::Name("main".into())]),
        input.clone(),
    ]);
    Node::List(vec![Node::Name("do".into()), def_main, export_main])
}

fn run_case(case: Case) -> CaseResult {
    if let Some(need) = first_unrealized(&case.needs) {
        return CaseResult { description: case.description, status: CaseStatus::Skipped(need) };
    }
    // Wrap the input in the program shape the SELECTED compiler expects: the old compiler takes the
    // `(module …)`/`main`-convention shape; rcdzc (`v2`) takes the implicit-module `(do … (export …))`
    // shape. Both grade against the same corpus oracle (run the emitted component, check the value).
    let program = if crate::compiler::use_v2() {
        as_program_v2(&case.input)
    } else {
        as_program(&case.input)
    };
    let compiled = crate::compiler::compile_program(&program);

    // The diagnostic code cdz-rustc must reject with, if any: a `(compiler (error CDZ####))`
    // clause (a program a static compiler rejects where a dynamic interpreter would run it), OR
    // a primary `(error CDZ####)` clause (a front-end rejection every generation makes, e.g.
    // an unbound name). Under constitution VII (Amendment 0.4.0) the seed is a compiler with no
    // dynamic carve-out, so it is judged against the rejection, not a running component.
    let expected_reject = case.compiler_error.clone().or(match &case.primary {
        PrimaryClause::Error(code) => Some(code.clone()),
        _ => None,
    });
    if let Some(expected_code) = &expected_reject {
        return match &compiled {
            Err(d) if d.code() == Some(expected_code.as_str()) => {
                CaseResult { description: case.description, status: CaseStatus::Passed }
            }
            Err(d) if d.code().is_some() => CaseResult {
                description: case.description,
                status: CaseStatus::Failed {
                    expected: format!("reject {expected_code}"),
                    observed: format!("reject {}", d.code().unwrap()),
                },
            },
            // A plain decline (not-yet-checked type rule) is an honest todo, not a failure.
            Err(_) => CaseResult {
                description: case.description,
                status: CaseStatus::Todo(format!("type rule {expected_code} not yet checked")),
            },
            Ok(_) => CaseResult {
                description: case.description,
                status: CaseStatus::Failed {
                    expected: format!("reject {expected_code}"),
                    observed: "emitted a running component (ill-typed program not rejected)".into(),
                },
            },
        };
    }

    // No compiler-error clause: this case has a runnable primary result (output/trap/exhausted),
    // so the corpus asserts the program is well-formed and produces that result. A plain, UNCODED
    // decline ("not yet lowered") is an honest todo. But a CODED rejection (`d.code().is_some()`)
    // is the compiler definitively asserting the program is ILL-TYPED — a direct contradiction of
    // a case with no `(compiler (error …))` clause, which says every generation runs it. That is a
    // FAIL (the compiler wrongly rejects a valid program), not a todo.
    let component = match compiled {
        Ok(bytes) => bytes,
        Err(d) if d.code().is_some() => {
            return CaseResult {
                description: case.description,
                status: CaseStatus::Failed {
                    expected: describe_primary(&case.primary),
                    observed: format!("wrongly rejected a valid program: {}", d),
                },
            }
        }
        Err(d) => return CaseResult { description: case.description, status: CaseStatus::Todo(d.0) },
    };

    // The component must validate and run.
    if let Err(e) = host::validate_component(&component) {
        return CaseResult {
            description: case.description,
            status: CaseStatus::Failed {
                expected: describe_primary(&case.primary),
                observed: format!("emitted invalid component: {}", first_line(&e.to_string())),
            },
        };
    }
    let manifest: Vec<String> = Vec::new(); // host imports are bound from the component itself
    // Seed the host's response log from the `(host-responses …)` fixture (value forms → `Val`).
    let responses: Vec<host::Val> =
        case.host_responses.iter().filter_map(val_of_form).collect();
    let (outcome, state) = match host::run_component_with_responses(&component, &manifest, &responses) {
        Ok(r) => r,
        Err(e) => {
            return CaseResult {
                description: case.description,
                status: CaseStatus::Failed {
                    expected: describe_primary(&case.primary),
                    observed: format!("failed to run: {}", first_line(&e.to_string())),
                },
            }
        }
    };

    let status = compare(&case, &outcome, &state);
    CaseResult { description: case.description, status }
}

/// Compare a run's observable behavior against the case's recorded clauses.
fn compare(case: &Case, outcome: &RunOutcome, state: &host::HostState) -> CaseStatus {
    let primary_ok = match (&case.primary, outcome) {
        (PrimaryClause::Output(form), RunOutcome::Value(rendered)) => {
            // Two independent checks that must BOTH hold. (1) The observed text equals the
            // expected render of the recorded value form. (2) For a float output, the observed
            // text must PARSE BACK to the exact recorded f64 (bit-identical). Check (2) is
            // independent of the renderer — `parse` is the inverse of `format`, computed by
            // different code — so it catches a renderer that is not injective even though both
            // sides of check (1) route through the same `display_float`. Without it a saturating
            // renderer (`f as i64`, which clamps every whole float ≥ 2^63 to one string) passed
            // trivially: expected and observed both saturated to the same wrong text
            // (deterministic-value-form.md §"Numeric Values Serialize Deterministically" —
            // distinct floats MUST serialize distinctly; see the gate-blindspot learning).
            expected_render(form).map_or(false, |e| &e == rendered)
                && float_output_round_trips(form, rendered)
                && string_output_round_trips(form, rendered)
        }
        // A wasm trap reproduces both a recorded trap and recorded exhaustion (the component
        // boundary signals a bounded halt as a trap; self-hosting-and-bootstrap.md).
        (PrimaryClause::Trap(_), RunOutcome::Trap(_)) => true,
        (PrimaryClause::Exhausted, RunOutcome::Trap(_)) => true,
        // A front-end rejection (unbound name / undeclared capability) has no running
        // component; if we compiled and ran, we cannot reproduce it — that's a todo, handled
        // before here by decline. Reaching here with Error means we ran something we
        // shouldn't; treat as not-ok so it surfaces.
        _ => false,
    };

    if !primary_ok {
        return CaseStatus::Failed {
            expected: describe_primary(&case.primary),
            observed: describe_outcome(outcome),
        };
    }

    if let Some(expected_events) = &case.events {
        let observed = &state.events;
        if observed.len() != expected_events.len() {
            return CaseStatus::Failed {
                expected: format!("{} event(s)", expected_events.len()),
                observed: format!("{} event(s)", observed.len()),
            };
        }
        for ((ek, eform), (ok, op)) in expected_events.iter().zip(observed) {
            let ep = expected_render(eform).unwrap_or_default();
            // events carry string payloads verbatim; the recorded form is (: "s" String)
            let ep = ep.trim_matches('"').to_string();
            if ek != ok || &ep != op {
                return CaseStatus::Failed {
                    expected: format!("event ({ek} {ep})"),
                    observed: format!("event ({ok} {op})"),
                };
            }
        }
    }

    // Host-call observation: the ordered `(host-calls (call NAME arg…) …)` the run MUST make
    // (core-semantics.md §Host Calls Are Ordered And Part Of Observable Behavior). An empty
    // `(host-calls)` asserts none were made; `None` does not pin them.
    if let Some(expected_calls) = &case.host_calls {
        let observed = &state.calls;
        if observed.len() != expected_calls.len() {
            return CaseStatus::Failed {
                expected: format!("{} host call(s)", expected_calls.len()),
                observed: format!("{} host call(s)", observed.len()),
            };
        }
        for ((ename, eargs), ocall) in expected_calls.iter().zip(observed) {
            let erendered: Vec<String> =
                eargs.iter().map(|a| expected_render(a).unwrap_or_default()).collect();
            if ename != &ocall.name || erendered != ocall.args {
                return CaseStatus::Failed {
                    expected: format!("call ({ename} {})", erendered.join(" ")),
                    observed: format!("call ({} {})", ocall.name, ocall.args.join(" ")),
                };
            }
        }
    }

    CaseStatus::Passed
}

/// Independent injectivity oracle for a FLOAT scalar output: the observed text must parse back
/// to the exact recorded f64 (bit-identical, NaN self-equal). `parse` is the inverse of the
/// renderer's `format`, computed by different code, so this catches a non-injective renderer that
/// the render-vs-render string compare cannot (both sides launder through the same formatter). For
/// a non-float (or compound) output this is vacuously true — those are covered byte-exactly by the
/// string compare, and only float text is subject to the saturation blind spot.
fn float_output_round_trips(form: &Node, observed: &str) -> bool {
    let value = if let Some(args) = form.as_form(":") {
        match args.first() {
            Some(v) => v,
            None => return true,
        }
    } else {
        form
    };
    let expected = match value {
        Node::Float(f) => *f,
        _ => return true,
    };
    // The observed float text is the canonical form with a trailing `.0` on a whole value (and
    // `NaN` / `-0.0` special cases). Rust's f64 parser reads all of these, including `NaN`.
    match observed.parse::<f64>() {
        Ok(got) if expected.is_nan() => got.is_nan(),
        Ok(got) => got.to_bits() == expected.to_bits(),
        Err(_) => false,
    }
}

/// Independent round-trip oracle for a STRING scalar output: the observed `"…"` text must READ BACK
/// (through the reader — `ast::read`) to the exact recorded string value. The reader is the inverse
/// of the renderer, computed by DIFFERENT code, so this catches a renderer that emits an escape the
/// reader cannot read back — e.g. `\u{7}` for U+0007 or `\0` for NUL, which are NOT in the closed
/// escape set (collections-and-text.md §A String Literal's Escapes Are A Closed Set: only `\n \t \r
/// \\ \"`), so `read("\u{7}")` yields the four chars `u{7}` and the render-vs-render string compare
/// (both sides launder through the same renderer) cannot see the mismatch. The string-analogue of
/// `float_output_round_trips` (the float-saturation gate blindspot). Vacuously true for a non-string
/// (or compound) output — those are covered byte-exactly by the string compare.
fn string_output_round_trips(form: &Node, observed: &str) -> bool {
    let value = if let Some(args) = form.as_form(":") {
        match args.first() {
            Some(v) => v,
            None => return true,
        }
    } else {
        form
    };
    let expected = match value {
        Node::Str(s) => s,
        _ => return true,
    };
    // Read the observed rendered text back through the reader. It must parse to a single string node
    // equal to the recorded value. (The reader NFC-normalizes; the recorded value form is already the
    // normalized text the program produced, so a normalized `expected` compares equal.)
    match ast::read(observed) {
        Ok(Node::Str(got)) => &got == expected,
        _ => false,
    }
}

/// Recover a flat `effect.op` name from a host-call name node: a bare `Node::Name` (e.g. a
/// single-segment host function) or the dotted member-access tree `(. effect op)` the reader
/// expands `effect.op` into. Returns None for any other shape.
fn dotted_flat_name(node: Option<&Node>) -> Option<String> {
    match node {
        Some(Node::Name(n)) => Some(n.clone()),
        Some(Node::List(items)) => {
            // `(. effect op)` → "effect.op".
            if matches!(items.first(), Some(Node::Name(h)) if h == ".") {
                if let (Some(Node::Name(e)), Some(Node::Name(o))) = (items.get(1), items.get(2)) {
                    return Some(format!("{e}.{o}"));
                }
            }
            None
        }
        _ => None,
    }
}

/// Convert a recorded value form `(: <value> <Type>)` (or a bare value) to a wasmtime `Val` the
/// host feeds back as a host-call response. The type annotation picks the `Val` arm (an Int64
/// annotation → `Val::S64`); only the scalar boundary types a host function returns are supported.
fn val_of_form(form: &Node) -> Option<host::Val> {
    let (value, ty) = if let Some(args) = form.as_form(":") {
        (args.get(0)?, args.get(1).and_then(type_name_of))
    } else {
        (form, None)
    };
    match (value, ty.as_deref()) {
        (Node::Int(n), Some("Int64") | None) => Some(host::Val::S64(*n)),
        (Node::Bool(b), _) => Some(host::Val::Bool(*b)),
        (Node::Float(f), _) => Some(host::Val::Float64(*f)),
        (Node::Str(s), _) => Some(host::Val::String(s.clone().into())),
        _ => None,
    }
}

/// The type name in an annotation's type position: `Int64` from `(: 41 Int64)`.
fn type_name_of(node: &Node) -> Option<String> {
    match node {
        Node::Name(n) => Some(n.clone()),
        _ => None,
    }
}

/// Render a recorded value form `(: <value> <Type>)` to the host's comparison string, so it
/// compares directly against a component's rendered result. Scalars render to their literal;
/// compound values (tuple/list/record/sum/AST/bytes) render to their canonical s-expression
/// text — the same text the compiled component's `display()` produces.
pub fn expected_render(form: &Node) -> Option<String> {
    let value = if let Some(args) = form.as_form(":") { args.get(0)? } else { form };
    render_value_node(value)
}

/// Render a canonical value NODE to its text form. The recorded `<value>` in a corpus case is
/// already the canonical s-expression (`(Some 42)`, `(tuple 1 true)`, `(Sign.Pos unit)` — the
/// last read as `((. Sign Pos) unit)`), so this reproduces that text, collapsing the dotted
/// member sugar back to `Type.Variant`.
fn render_value_node(value: &Node) -> Option<String> {
    match value {
        Node::Int(n) => Some(n.to_string()),
        Node::Bool(b) => Some(b.to_string()),
        Node::Float(f) => Some(host::display_float(*f)),
        // Closed-escape-set render (NOT `{:?}`), matching the compiler's `string_canonical_text` and
        // the host's `render_val`, so the EXPECTED text uses the same round-trippable form the
        // component produces (a non-printable scalar renders verbatim, not `\u{…}`).
        Node::Str(s) => Some(codegen::string_canonical_text(s)),
        Node::Name(n) if n == "unit" => Some("unit".to_string()),
        Node::Name(n) => Some(n.clone()),
        Node::List(items) => {
            // `(. Type Variant)` → the qualified name `Type.Variant`.
            if items.len() == 3 {
                if let (Some(Node::Name(dot)), Node::Name(a), Node::Name(b)) =
                    (items.first(), &items[1], &items[2])
                {
                    if dot == "." {
                        return Some(format!("{a}.{b}"));
                    }
                }
            }
            // A byte sequence — however it was written in the corpus (`b"…"` reader sugar or the
            // explicit `(Bytes.of (list …))`) — reads to the SAME `(Bytes.of (list b0 b1 …))` tree,
            // and renders to the canonical `b"…"` display text the compiled component produces. This
            // is what makes the two spellings round-trip: both observe as one form.
            if let Some(bytes) = bytes_of_tree(items) {
                return Some(cdz_compiler::codegen::bytes_literal_text(&bytes));
            }
            let parts: Vec<String> = items.iter().map(render_value_node).collect::<Option<_>>()?;
            Some(format!("({})", parts.join(" ")))
        }
    }
}

/// If `items` is the canonical `(Bytes.of (list i0 i1 …))` tree — head `(. Bytes of)` applied to a
/// `(list …)` of integers each in 0..=255 — return the bytes. This is the tree both `b"…"` sugar and
/// the explicit form read to, so the oracle renders either as the canonical `b"…"` display text.
fn bytes_of_tree(items: &[Node]) -> Option<Vec<u8>> {
    if items.len() != 2 {
        return None;
    }
    // Head must be `(. Bytes of)`.
    match &items[0] {
        Node::List(h)
            if h.len() == 3
                && matches!(&h[0], Node::Name(n) if n == ".")
                && matches!(&h[1], Node::Name(n) if n == "Bytes")
                && matches!(&h[2], Node::Name(n) if n == "of") => {}
        _ => return None,
    }
    // Argument must be `(list <int in 0..=255> …)`.
    let list = match &items[1] {
        Node::List(l) if matches!(l.first(), Some(Node::Name(n)) if n == "list") => l,
        _ => return None,
    };
    let mut bytes = Vec::with_capacity(list.len().saturating_sub(1));
    for elem in &list[1..] {
        match elem {
            Node::Int(n) if (0..=255).contains(n) => bytes.push(*n as u8),
            _ => return None, // a non-literal or out-of-range byte is not a renderable value form
        }
    }
    Some(bytes)
}

fn describe_primary(p: &PrimaryClause) -> String {
    match p {
        PrimaryClause::Output(form) => format!("output {}", expected_render(form).unwrap_or_else(|| "<compound>".into())),
        PrimaryClause::Trap(r) => format!("trap {r:?}"),
        PrimaryClause::Exhausted => "exhausted".into(),
        PrimaryClause::Error(c) => format!("error {c}"),
    }
}

fn describe_outcome(o: &RunOutcome) -> String {
    match o {
        RunOutcome::Value(v) => format!("output {v}"),
        RunOutcome::Trap(r) => format!("trap {}", first_line(r)),
        RunOutcome::Suspended(c) => format!("suspended on host call {}", c.name),
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}
