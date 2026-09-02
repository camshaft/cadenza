//! Generate the RUST driver source spliced around an emitted `--target rust[-async]` module — the
//! crate-root host-call shim fns the emitted `mod prog` references (`crate::__cdz_host_<id>()`). Pure
//! string generation from a case's recorded host tape; no process/filesystem. Ported from `xtask`'s
//! `build_rust_host_shims` family. Later increments add the export-call assembly + the `rustc`/run.

use std::collections::{BTreeMap, BTreeSet};

use crate::sig::{
    env_closure_call_arg, is_env_param, parse_emitted_sig, split_factory_application,
};

/// Marshal a call's canonical-sexp arg VALUES to the Rust expressions the emitted export expects. A scalar
/// passes through, a compound (`(tuple …)`/`(record …)`) is rebuilt by `rust_call_arg`. TYPE-AWARE for a
/// BIGINT param: a corpus arg is a BARE decimal (`5`) — its `(: 5 BigInt)` annotation is stripped by the
/// corpus parser, so unlike a self-identifying `"…"` String there is nothing telling `rust_call_arg` it
/// must cross as `cdz_num::Big` (emitted verbatim `5` is an i64 literal → rustc E0308 against `fn(a:
/// cdz_num::Big)`). So read the emitted fn's param TYPES off its signature and marshal a decimal arg to a
/// `cdz_num::Big` param via `big_arg_expr` (the owned-BigInt construction). Non-BigInt params, or a
/// non-decimal arg, keep the ordinary `rust_call_arg`.
pub fn marshal_call_args(
    module: &str,
    export: &str,
    args: &[String],
    async_mode: bool,
) -> Vec<String> {
    let name = cdz_rust_render::rust_ident(export);
    let arg_param_tys: Vec<Option<String>> = parse_emitted_sig(module, &name, async_mode)
        .map(|sig| {
            sig.params
                .iter()
                .filter(|p| !is_env_param(p))
                .map(|p| p.split_once(':').map(|(_, ty)| ty.trim().to_string()))
                .collect()
        })
        .unwrap_or_default();
    args.iter()
        .enumerate()
        .map(|(i, a)| {
            let is_bigint_param = arg_param_tys
                .get(i)
                .and_then(|o| o.as_deref())
                .is_some_and(|ty| ty == "cdz_num::Big");
            if is_bigint_param && let Ok(n) = a.trim().parse::<i128>() {
                cdz_rust_render::big_arg_expr(n)
            } else {
                cdz_rust_render::rust_call_arg(a)
            }
        })
        .collect()
}

/// The ORDINARY (non-factory, non-closure) call expression for an export — `<ident>(<marshaled args>)`,
/// the common corpus case (the factory/closure-consumer builders handle the host-closure subset). `args`
/// are canonical-sexp value texts; the emitted export's parameter types drive the marshal.
pub fn ordinary_call_expr(module: &str, export: &str, args: &[String], async_mode: bool) -> String {
    let name = cdz_rust_render::rust_ident(export);
    let marshaled = marshal_call_args(module, export, args, async_mode);
    format!("{name}({})", marshaled.join(", "))
}

/// The call expression for an export, FACTORY-aware. A host-closure FACTORY (its return type names a
/// closure — rcdzc emits `pub fn f(caps…) -> Rc<dyn Fn(x)->r>`) is applied in TWO groups: the first
/// `K` = factory-capture-count marshaled args build the handle, the rest apply the returned closure —
/// `f(caps)(applied)` — the native equivalent of the wasm make/call resource ABI. A non-factory export is
/// the ordinary single call `f(args)`. Mirrors xtask `run_program_rust`'s factory-split. (The closure-
/// PARAMETER consumer application — a `build_closure_consumer_call` — and the async `block_on` harness stay
/// deferred; the corpus's factory cases need only this split.)
pub fn call_expr(module: &str, export: &str, args: &[String], async_mode: bool) -> String {
    use crate::sig::rust_factory_param_count;
    let name = cdz_rust_render::rust_ident(export);
    let marshaled = marshal_call_args(module, export, args, async_mode);
    match rust_factory_param_count(module, &name, async_mode) {
        Some(k) if k <= marshaled.len() => {
            let (caps, applied) = marshaled.split_at(k);
            format!("{name}({})({})", caps.join(", "), applied.join(", "))
        }
        _ => format!("{name}({})", marshaled.join(", ")),
    }
}

/// Build the call for a CLOSURE-PARAMETER CONSUMER export — one that takes a `Rc<dyn Fn(…)>` param supplied
/// by a companion PRODUCER export (rcdzc splits `(fn …)`-consuming defs across sibling exports; a host has
/// no closure literal, so the harness synthesizes it). Returns `None` when `export` is not a consumer (no
/// closure param) — the caller falls back to the factory/ordinary call.
///
/// Each source param is threaded LEFT-TO-RIGHT onto the flat call `args`: a CLOSURE param pairs to a producer
/// (a sibling whose emitted closure type matches — a FACTORY `fn mk(caps) -> Rc<dyn Fn…>` supplying
/// `prog::mk(<caps>)`, or a PEELED nullary `fn mk(x)->r` supplying `Rc::new(prog::mk as fn(x)->r)`) and
/// consumes that producer's capture args; a non-closure param consumes one verbatim arg. When a Tuple-arg vs
/// Record-arg producer's ERASED `Rc<dyn Fn>` type collides, the pre-erasure Cadenza shapes
/// (`cdz-param-shapes` / `cdz-produces-closure`) disambiguate.
///
/// ASYNC differs in two ways (mirroring the emit's lifted-closure convention): the producer scan also matches
/// `pub async fn`, and a FACTORY-built closure is driven through `block_on(prog::mk(&mut env, caps…))` — the
/// producer is an `async fn`, and threading `&mut env` into BOTH the producer AND the consumer call in one
/// expression would be two overlapping `&mut env` borrows (E0499). So each async-built closure is `let __gN`-
/// bound FIRST (sequential borrows), then the consumer is called with the bound names, and the whole thing is
/// returned as a fully-driven `{ let __g0 = …; block_on(prog::<name>(&mut env, <args>)) }` block (already
/// `prog::`-qualified + `block_on`-wrapped, so the caller uses it VERBATIM). An async PEELED producer's
/// fn-item is `fn(&mut E, …) -> impl Future`, not the sync `fn(…) -> ret` the coercion needs, so an async
/// peeled-producer consumer returns `None` (declined — the async factory-producer consumer is the bulk).
/// Ported from xtask `build_closure_consumer_call`.
pub fn build_closure_consumer_call(
    module: &str,
    name: &str,
    args: &[String],
    async_mode: bool,
) -> Option<String> {
    use crate::sig::{
        closure_param_type, closure_ret_type, is_closure_param, is_env_param, names_closure_value,
        param_type_of, parse_emitted_sig,
    };
    use cdz_rust_render::{cdz_param_shapes, cdz_produces_closure, cdz_return_type};

    let sig = parse_emitted_sig(module, name, async_mode)?;
    let source_params: Vec<&&str> = sig.params.iter().filter(|p| !is_env_param(p)).collect();
    if !source_params.iter().any(|p| is_closure_param(p)) {
        return None; // not a consumer — let the factory/ordinary path handle it
    }

    // Enumerate PRODUCER exports (module order): a FACTORY (result is a closure) or a PEELED nullary fn whose
    // fn-item coerces to the closure. Each carries its erased closure type + pre-erasure `shape` for pairing.
    enum Producer {
        Factory {
            ident: String,
            closure_ty: String,
            cap: usize,
            shape: Option<String>,
        },
        Peeled {
            ident: String,
            closure_ty: String,
            fn_ty: String,
            shape: Option<String>,
        },
    }
    let mut producers: Vec<Producer> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // Scan BOTH `pub fn` (sync) and `pub async fn` (async) headers — in async mode a FACTORY producer is an
    // `async fn` returning `Rc<dyn EnvClosure>`, so its header carries the `async` keyword.
    for (i, _) in module
        .match_indices("pub fn ")
        .chain(module.match_indices("pub async fn "))
    {
        let rest = &module[i..];
        let Some(after_kw) = rest
            .strip_prefix("pub async fn ")
            .or_else(|| rest.strip_prefix("pub fn "))
        else {
            continue;
        };
        let ident: String = after_kw
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if ident.is_empty() || ident == name || !seen.insert(ident.clone()) {
            continue;
        }
        let Some(psig) = parse_emitted_sig(module, &ident, async_mode) else {
            continue;
        };
        let src_params: Vec<&&str> = psig.params.iter().filter(|p| !is_env_param(p)).collect();
        if names_closure_value(&psig.ret_head) {
            let Some(cty) = closure_ret_type(&psig.ret_head) else {
                continue;
            };
            producers.push(Producer::Factory {
                ident: ident.clone(),
                closure_ty: cty,
                cap: src_params.len(),
                shape: cdz_return_type(module, &ident),
            });
        } else {
            let param_types: Vec<String> = src_params.iter().map(|p| param_type_of(p)).collect();
            let ret = psig
                .ret_head
                .trim()
                .trim_start_matches("->")
                .trim()
                .to_string();
            producers.push(Producer::Peeled {
                closure_ty: format!("std::rc::Rc<dyn Fn({}) -> {}>", param_types.join(", "), ret),
                fn_ty: format!("fn({}) -> {}", param_types.join(", "), ret),
                shape: cdz_produces_closure(module, &ident),
                ident: ident.clone(),
            });
        }
    }

    let consumer_shapes = cdz_param_shapes(module, name);
    let mut used = vec![false; producers.len()];
    let mut arg_i = 0usize;
    let mut call_args: Vec<String> = Vec::with_capacity(source_params.len());
    let mut closure_param_idx = 0usize;
    // ASYNC: a FACTORY-built closure is driven through `block_on(prog::mk(&mut env, caps))` (the producer is
    // an `async fn`), and threading `&mut env` into both the producer AND the consumer call at once is two
    // overlapping `&mut env` borrows (E0499). So bind each async-built closure to a `let __gN` FIRST and
    // collect those statements as a prelude the caller splices before the consumer call. Empty in sync mode.
    let mut async_lets: Vec<String> = Vec::new();
    // A producer matches when its ERASED closure type equals the consumer param's AND (when both shapes are
    // known) the pre-erasure shapes agree — the shape guard only NARROWS, never admits an erased mismatch.
    let ty_matches = |prod: &Producer, cty: &str, want_shape: Option<&str>| {
        let (closure_ty, prod_shape) = match prod {
            Producer::Factory {
                closure_ty, shape, ..
            }
            | Producer::Peeled {
                closure_ty, shape, ..
            } => (closure_ty, shape.as_deref()),
        };
        closure_ty.as_str() == cty
            && match (want_shape, prod_shape) {
                (Some(w), Some(p)) => w == p,
                _ => true,
            }
    };
    for p in &source_params {
        if let Some(cty) = closure_param_type(p) {
            let want_shape = consumer_shapes.get(closure_param_idx).map(|s| s.as_str());
            closure_param_idx += 1;
            // First UNUSED matching producer (deterministic), else REUSE a matching one (the host mints a
            // fresh handle per param, so one producer can supply several closure params).
            let pi = producers
                .iter()
                .enumerate()
                .position(|(pi, prod)| !used[pi] && ty_matches(prod, cty, want_shape))
                .or_else(|| {
                    producers
                        .iter()
                        .position(|prod| ty_matches(prod, cty, want_shape))
                })?;
            used[pi] = true;
            match &producers[pi] {
                Producer::Factory { ident, cap, .. } => {
                    if arg_i + cap > args.len() {
                        return None;
                    }
                    let caps = &args[arg_i..arg_i + cap];
                    arg_i += cap;
                    if async_mode {
                        // `block_on(prog::mk(&mut env, caps))` yields the (sync) closure handle; bind it to a
                        // fresh `__gN` so the consumer call's `&mut env` borrow doesn't overlap the producer's.
                        let g = format!("__g{}", async_lets.len());
                        let envcaps = if caps.is_empty() {
                            "&mut env".to_string()
                        } else {
                            format!("&mut env, {}", caps.join(", "))
                        };
                        async_lets.push(format!("let {g} = block_on(prog::{ident}({envcaps}));"));
                        call_args.push(g);
                    } else {
                        call_args.push(format!("prog::{ident}({})", caps.join(", ")));
                    }
                }
                Producer::Peeled {
                    ident,
                    fn_ty,
                    closure_ty,
                    ..
                } => {
                    // An async peeled producer's fn-item is `fn(&mut E, …) -> impl Future`, not the sync
                    // `fn(…) -> ret` this coercion needs — DECLINE (a follow-up sub-slice; the async factory-
                    // producer consumer is the bulk that lands here).
                    if async_mode {
                        return None;
                    }
                    call_args.push(format!(
                        "(std::rc::Rc::new(prog::{ident} as {fn_ty}) as {closure_ty})"
                    ));
                }
            }
        } else {
            if arg_i >= args.len() {
                return None;
            }
            call_args.push(args[arg_i].clone());
            arg_i += 1;
        }
    }
    // SYNC: return the bare `<name>(<args>)` — the caller's `call_or_await`/render prepends `prog::`.
    // ASYNC: return the FULLY-DRIVEN block itself — already `prog::`-qualified + `block_on`-wrapped, so the
    // caller uses it verbatim (its `call_or_await` must NOT re-wrap it).
    if async_mode {
        let lets = async_lets.join(" ");
        Some(format!(
            "{{ {lets} block_on(prog::{name}(&mut env, {})) }}",
            call_args.join(", ")
        ))
    } else {
        Some(format!("{name}({})", call_args.join(", ")))
    }
}

/// Assemble the full RUST driver source for a corpus case, covering SYNC and ASYNC exports plus the
/// host-closure factory/consumer application. It wraps the emitted `module` in `mod prog { … }` (so its
/// `pub fn main` becomes `prog::main`, not a duplicate crate `main`), splices the host-response shim fns
/// (`build_rust_host_shims`), and emits a `fn main` that calls the export, renders the result to cdz-run's
/// canonical text via `cdz_render_expr` (driven by the backend's `// cdz-*` render notes the module carries),
/// and prints it. When `async_mode`, the export is an `async fn` taking the gas/yield env first, so the
/// driver splices the `ASYNC_GATE_HARNESS` (a no-limit `GateEnv` + a minimal `block_on`) and drives
/// `block_on(prog::export(&mut env, args))` — a concrete `GateEnv` coerces to the `&mut dyn DynCdzEnv`
/// boundary via cdz-rt's blanket `impl<E: CdzEnv> DynCdzEnv for E`. A DIVERGING export (`-> !` / `Any` / `?N`)
/// is just CALLED (it traps) — binding + printing a `!` is a build error, and the panic IS the recorded
/// `(trap …)` outcome; an export with no parsed `cdz-return` note falls back to `Display` (`{}`), the scalar
/// shape. Pure string generation; the result rendering + note parsing live in `cdz-rust-render`.
pub fn build_driver_source(
    module: &str,
    export: &str,
    args: &[String],
    host_responses: &[(String, String)],
    host_calls: &[String],
    async_mode: bool,
) -> String {
    use cdz_rust_render::{
        cdz_newtype_descriptors, cdz_qty_at, cdz_render_expr, cdz_return_type, cdz_scale,
        cdz_sum_descriptors, cdz_sum_params, cdz_sum_qualified_heads, cdz_unit_form,
    };

    // Is this a host-closure FACTORY (return type names a closure) or a CONSUMER (takes a closure param)?
    // Both cross a String/Bytes RESULT through the wasm boundary's serialized `list<u8>` form, so their
    // result is rendered specially (below); a plain export keeps `cdz_render_expr`. Computed FIRST because the
    // async call assembly (below) needs `is_factory` to split the factory/application seam.
    let ident = cdz_rust_render::rust_ident(export);
    let is_factory = crate::sig::rust_factory_param_count(module, &ident, async_mode).is_some();
    let is_consumer = !is_factory
        && crate::sig::parse_emitted_sig(module, &ident, async_mode).is_some_and(|s| {
            s.params
                .iter()
                .filter(|p| !crate::sig::is_env_param(p))
                .any(|p| crate::sig::is_closure_param(p))
        });

    // A CLOSURE-PARAMETER CONSUMER export (takes a `Rc<dyn Fn>` param) synthesizes the closure from a sibling
    // producer — checked FIRST (a consumer's own return is not a closure, so the factory path would miss it).
    // Else the factory-aware/ordinary call. In SYNC mode the consumer-call returns a bare `name(prog::producer
    // (caps), …)` whose producers are already `prog::`-qualified; in ASYNC mode it returns a fully-driven
    // `{ let __g0 = …; block_on(prog::name(&mut env, …)) }` block (used verbatim below).
    let call = build_closure_consumer_call(module, &ident, args, async_mode)
        .unwrap_or_else(|| call_expr(module, export, args, async_mode));

    // The call the driver's `fn main` evaluates. SYNC prepends `prog::` to the bare call (`prog::adder(3)(5)`).
    // ASYNC threads the gas/yield `env` as the export's FIRST arg and drives its future with `block_on` — a
    // concrete `GateEnv` coerces to the `&mut dyn DynCdzEnv` boundary via cdz-rt's blanket
    // `impl<E: CdzEnv> DynCdzEnv for E`, so the driver passes `&mut env`, NOT a `__CdzE`. The answer must MATCH
    // the sync/wasm oracle (gas metering is invisible to the result), so it grades identically. Three async
    // call shapes (a consumer block is already fully driven, so it is passed through):
    //  - non-factory nullary `export()`       → `block_on(prog::export(&mut env))`
    //  - non-factory with args `export(a, b)` → `block_on(prog::export(&mut env, a, b))`
    //  - FACTORY `export(caps…)(applied…)`    → env threads into the FACTORY call only; the returned async
    //    `Rc<dyn EnvClosure>` is applied via `handle.call(&mut env, arg).await` (bind the `block_on(factory)`
    //    handle to `__h` FIRST so its `.call` env borrow doesn't overlap the factory's, E0499).
    let call_or_await = if async_mode {
        if call.starts_with('{') {
            // A consumer call is already `prog::`-qualified + `block_on`-wrapped; pass it VERBATIM (the arg-
            // threading rewrites below would double-wrap it). Discriminated by the leading `{` (no other call
            // shape starts with a block).
            call.clone()
        } else if let Some((factory_call, application)) = is_factory
            .then(|| split_factory_application(&call))
            .flatten()
        {
            // `factory_call` = `export(caps…)` (caps may be empty); `application` = `(applied…)`.
            let caps = factory_call
                .strip_prefix(&format!("{ident}("))
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or("");
            let factory = if caps.is_empty() {
                format!("prog::{ident}(&mut env)")
            } else {
                format!("prog::{ident}(&mut env, {caps})")
            };
            // `EnvClosure::call` takes ONE `A` (a multi-arg closure tuples its args, matching the lifted
            // convention + the emit's CallClosure) — `env_closure_call_arg` builds it from the flat applied args.
            let applied = application
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or("")
                .trim();
            let arg = env_closure_call_arg(applied);
            format!("{{ let __h = block_on({factory}); block_on(__h.call(&mut env, {arg})) }}")
        } else if call.ends_with("()") {
            format!("block_on(prog::{ident}(&mut env))")
        } else {
            let arglist = call
                .strip_prefix(&format!("{ident}("))
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or("");
            format!("block_on(prog::{ident}(&mut env, {arglist}))")
        }
    } else {
        format!("prog::{call}")
    };

    // A host-closure FACTORY export's `cdz-return` note is the returned closure's CURRIED arrow
    // (`(-> Int64 Int64)`); `call_expr` applies the factory to full arity, so the value rendered is the
    // closure's FINAL result — peel the arrow to that type so `cdz_render_expr` renders it structurally
    // (a bare arrow would render as a closure and not compile). A non-factory export keeps its note.
    // NOTE-KEY: the backend keys its `// cdz-*` notes by the emitted `rust_ident` (`cdz-return[mk_b]`),
    // NOT the raw source name — so a HYPHENATED export (`mk-b`) must look up by `ident`, else the note
    // misses, `ret_ty` is `None`, and the render falls to the bare-`Display` arm (E0277 on a `Vec`/compound).
    let ret_ty = cdz_return_type(module, &ident).map(|t| {
        if is_factory {
            crate::sig::peel_arrow_result(&t)
        } else {
            t
        }
    });

    // A diverging result type (`-> !`) — its `cdz-return` note is `Any` / `!` / a bare `?N` result var.
    let diverging = ret_ty.as_deref().is_some_and(|t| {
        t == "Any" || t == "!" || (t.starts_with('?') && t[1..].chars().all(|c| c.is_ascii_digit()))
    });

    // FLAG-GATED value-doc path (`CDZ_VALUE_DOC`, default-OFF): when rcdzc emitted a self-contained
    // `pub fn __cdz_doc_<ident>() -> String` for this export (marked `// cdz-value-doc: <ident>`), the driver
    // just prints that fn's `CDZDOC:<hex>` result — the Ty-direct render replacing the type-note-driven
    // `cdz_render_expr`/`value_doc_render_scalar` string walk. `interpret_run_stdout` (the read side) decodes
    // the marker. Marker absent (flag off) => the ordinary render path below (byte-identical). A factory/
    // consumer export keeps its special render (no nullary __cdz_doc). NOTE keyed by `ident` (== rust_ident),
    // matching the other `// cdz-*` notes.
    // Gate on the MARKER's PRESENCE — rcdzc emits `// cdz-value-doc: <ident>` iff its compile had
    // CDZ_VALUE_DOC set to non-"0" (the nix corpus-rust-exec layer sets it for the flip), so the marker IS
    // the authoritative signal (no env re-check — the emit decision already happened + is recorded here).
    let value_doc = !is_factory
        && !is_consumer
        && module
            .lines()
            .any(|l| l.trim() == format!("// cdz-value-doc: {ident}"));
    let body = if value_doc {
        format!("fn main() {{ println!(\"{{}}\", prog::__cdz_doc_{ident}()); }}\n")
    } else if diverging {
        format!("fn main() {{ {call_or_await}; }}\n")
    } else {
        match ret_ty.as_deref() {
            Some(ty) => {
                let sums = cdz_sum_descriptors(module);
                let newtypes = cdz_newtype_descriptors(module);
                let sum_params = cdz_sum_params(module);
                let qualified_heads = cdz_sum_qualified_heads(module);
                let unit_form = cdz_unit_form(module, &ident);
                let unit_scale = cdz_scale(module, &ident);
                let qty_at = cdz_qty_at(module, &ident);
                let general = || {
                    cdz_render_expr(
                        ty,
                        &sums,
                        &newtypes,
                        &sum_params,
                        unit_form.as_deref(),
                        unit_scale,
                        &qty_at,
                        &qualified_heads,
                    )
                };
                // NOTE: the interim scalar-only value-doc path (`value_doc_render_scalar`, built driver-side by
                // cdz-rust-render from the type STRING) is SUPERSEDED by the marker-driven `__cdz_doc` body
                // above — under CDZ_VALUE_DOC a covered export (a scalar IS covered) carries the
                // `// cdz-value-doc:` marker, so `value_doc` is true and this render-select is not reached. The
                // rcdzc-side `__cdz_doc` walk (Ty-DIRECT, no type-string re-parse) is the end-state that lets us
                // delete cdz_render_at/parse_head_type; the driver-side scalar builder was the bootstrap.
                let render = if (is_factory || is_consumer) && (ty == "String" || ty == "Bytes") {
                    // A host-closure String/Bytes RESULT crosses the wasm boundary as `list<u8>` → the corpus
                    // records the canonical byte-int list `#list(104 105)`/`#list()`, NOT `"hi"`/`b"…"`. Render
                    // `__r` (a `String`/`Vec<u8>`) as the #list byte form (also avoids the E0277 `Vec<u8>:
                    // Display`). Mirrors xtask.
                    cdz_render_bytes_list(ty)
                } else if is_factory && factory_result_is_value_form_sum(ty, &sums) {
                    // A host-closure FACTORY SUM RESULT (Option/Result/user-sum) crosses value-ENCODED → the
                    // corpus records the TYPE-ANNOTATED value form `(: (Some 5) (Option Int64))` (the wasm
                    // `call`-method value-encode shape), nested in the case's own `output (: <that> <type>)`.
                    // A plain sum export renders the bare `(Some 5)` (the grader unwraps one annotation level),
                    // but a factory sum result needs the INNER annotation too — wrap the bare value in
                    // `(: <value> <type-surface>)`. Mirrors xtask's factory-sum branch.
                    let inner = general();
                    let ty_surface = if ty.contains(' ') && !ty.starts_with('(') {
                        format!("({ty})")
                    } else {
                        ty.to_string()
                    };
                    format!("format!(\"(: {{}} {ty_surface})\", {inner})")
                } else {
                    general()
                };
                format!(
                    "fn main() {{ let __r = {call_or_await}; println!(\"{{}}\", {render}); }}\n"
                )
            }
            // Unknown return type (no emitted note) — fall back to `{}` (a scalar via Display).
            None => format!("fn main() {{ println!(\"{{}}\", {call_or_await}); }}\n"),
        }
    };

    let host_shims = build_rust_host_shims(module, host_responses, host_calls);
    // In async mode the driver needs a `GateEnv` (a no-limit gas meter — the gate checks ANSWERS, not fuel
    // bounds) + a tiny `block_on` executor (`ASYNC_GATE_HARNESS`), and `fn main` must bind `let mut env`
    // before the call — spliced in by rewriting the `fn main() {` header. A sync driver keeps the plain
    // assembly (no env, no harness).
    if async_mode {
        let full = format!("mod prog {{\n{module}\n}}\n{host_shims}{ASYNC_GATE_HARNESS}\n{body}");
        full.replace("fn main() {", "fn main() { let mut env = GateEnv;")
    } else {
        format!("mod prog {{\n{module}\n}}\n{host_shims}{body}")
    }
}

/// The async gate driver's harness: a no-limit `GateEnv` implementing the emitted `cdz_rt::CdzEnv` (the gate
/// checks ANSWERS, not fuel bounds, so `consume` never blocks/panics) + a minimal `block_on` executor (a real
/// Waker is unneeded — the emitted futures never register one; they only `.await` `consume`, which is `Ready`
/// immediately, so a busy-poll loop drives them to completion). A concrete `GateEnv` coerces to the emitted
/// exports' `&mut dyn DynCdzEnv` boundary via cdz-rt's blanket `impl<E: CdzEnv> DynCdzEnv for E`. Spliced into
/// the async driver before `fn main`. Copied verbatim from xtask's `ASYNC_GATE_HARNESS`.
const ASYNC_GATE_HARNESS: &str = r#"
struct GateEnv;
impl cdz_rt::CdzEnv for GateEnv {
    async fn consume(&mut self, _gas: u64) {}
}
fn block_on<F: core::future::Future>(mut f: F) -> F::Output {
    use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn noop(_: *const ()) {}
    fn clone(_: *const ()) -> RawWaker { raw() }
    fn raw() -> RawWaker { RawWaker::new(core::ptr::null(), &VT) }
    static VT: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    let w = unsafe { Waker::from_raw(raw()) };
    let mut cx = Context::from_waker(&w);
    let mut f = unsafe { core::pin::Pin::new_unchecked(&mut f) };
    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut cx) { return v; } }
}
"#;

/// Whether a factory's (arrow-peeled) RESULT type is a SUM that crosses the host boundary value-ENCODED —
/// an `Option`/`Result`, or a USER sum whose head token is a `cdz-sum` descriptor key. Such a result is
/// recorded type-annotated (`(: (Some 5) (Option Int64))`), so the driver wraps the rendered value. Mirrors
/// xtask's `factory_result_is_value_form_sum`.
fn factory_result_is_value_form_sum(
    ty: &str,
    sums: &std::collections::HashMap<String, Vec<(String, Vec<String>)>>,
) -> bool {
    let head = ty.trim().trim_start_matches('(');
    if head.starts_with("Option ")
        || head.starts_with("Result ")
        || head == "Option"
        || head == "Result"
    {
        return true;
    }
    // A USER sum: the head token (bare `Dir`, or the applied head of `(Box Int64)`) is a descriptor key.
    let head_token = head.split_whitespace().next().unwrap_or(head);
    sums.contains_key(head_token)
}

/// Render `__r` (a host-closure String/Bytes RESULT) as the boundary byte-int list `(104 105)` / `()`.
/// `__r` is a `String` (String result — iterate its UTF-8 bytes) or `Vec<u8>` (Bytes result). Returns a
/// Rust block expression usable as the `println!("{}", …)` arg. Mirrors xtask's `cdz_render_bytes_list`.
/// Emits the CANONICAL `#list(b0 b1 …)` compound (`#list()` when empty), matching cdz-run's `render_val`
/// and the wasm boundary — the corpus rolled to `#ctor`-everywhere, so the bare `(b0 b1 …)` form this once
/// produced regressed every `list<u8>` closure-result case (v-corpus-harness ruled `#list` canonical).
fn cdz_render_bytes_list(ty: &str) -> String {
    let iter = if ty == "String" {
        "(__r).bytes()"
    } else {
        "(__r).iter().copied()"
    };
    format!(
        "{{ let mut __s = String::from(\"#list(\"); let mut __first = true; for __b in {iter} {{ \
         if !__first {{ __s.push(' '); }} __first = false; __s.push_str(&__b.to_string()); }} \
         __s.push(')'); __s }}"
    )
}

/// Kebab-normalize an EFFECT name (matching the backend's `canonical_host_op_key`): CamelCase / `_` / `-`
/// runs collapse to single `-`, lowercased, no leading/trailing `-`.
pub fn kebab_effect(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else if c == '_' || c == '-' {
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
        } else {
            out.push(c);
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Derive the crate-root host-call shim fn ident from a recorded response key (`effect.op`) — kebab-normalize
/// the EFFECT part (matching the backend's `canonical_host_op_key`), keep the op verbatim, then map the
/// dotted key's non-ident chars → `_`. MUST equal the backend's emitted `host_shim_ident` for the same op.
pub fn host_shim_ident_from_key(op_key: &str) -> String {
    let (eff, op) = op_key.split_once('.').unwrap_or(("", op_key));
    let canonical = format!("{}.{}", kebab_effect(eff), op);
    let mut s = String::with_capacity(canonical.len() + 11);
    s.push_str("__cdz_host_");
    for c in canonical.chars() {
        if c == '_' || c.is_ascii_alphanumeric() {
            s.push(c);
        } else {
            s.push('_');
        }
    }
    s
}

/// Generate the crate-root host-call shim fns the emitted `mod prog` references (`crate::__cdz_host_<id>()`).
/// A shim is generated for EVERY distinct `__cdz_host_*` symbol the module names — including UNEXERCISED
/// defs — since every referenced symbol must be DEFINED or rustc E0425s at link. A symbol matched to
/// recorded responses (by the driver-derived ident, which kebab-normalizes the response-key effect to agree
/// with the backend) returns them in order + prints `host-call\t<recorded-op>`; a unit-result op prints its
/// op and returns `()`; an unmatched symbol gets a `panic!` stub (never reached on a passing trial).
pub fn build_rust_host_shims(
    module: &str,
    host_responses: &[(String, String)],
    host_calls: &[String],
) -> String {
    // Map recorded op key → (its CANONICAL dotted key for the host-call print, values in order), by shim
    // ident. The printed `host-call\t<op>` is the CANONICAL key (kebab-normalized effect + verbatim op), NOT
    // the raw recorded key — the grader compares observed vs expected by exact string, so a source-cased
    // response key (`Param.width`) must be normalized (`param.width`) before printing.
    let mut by_ident: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    for (op, value) in host_responses {
        let ident = host_shim_ident_from_key(op);
        by_ident
            .entry(ident)
            .or_insert_with(|| {
                let (eff, opname) = op.split_once('.').unwrap_or(("", op.as_str()));
                (format!("{}.{}", kebab_effect(eff), opname), Vec::new())
            })
            .1
            .push(value.clone());
    }
    // UNIT-RESULT ops (H8): a `(host-calls …)` entry whose op has NO `(host-response …)` is a pure effect op
    // that returns the unit value (crosses the boundary only to be OBSERVED — e.g. `log.emit`). It records
    // its call NAME but no response VALUE. Keyed by shim IDENT so an op that IS in host_responses under a
    // source-cased key still matches and is NOT mis-treated as unit-result.
    let response_idents: BTreeSet<String> = host_responses
        .iter()
        .map(|(op, _)| host_shim_ident_from_key(op))
        .collect();
    let mut unit_ops: BTreeMap<String, String> = BTreeMap::new();
    for op in host_calls {
        let ident = host_shim_ident_from_key(op);
        if response_idents.contains(&ident) {
            continue; // a VALUE-result op: handled via by_ident above.
        }
        unit_ops.insert(ident, op.clone());
    }
    // Every `crate::__cdz_host_<ident>(<args>)` the module references, with its ARG COUNT (the shim's fn
    // arity must match every call site or rustc E0061s). The backend emits args as simple `__ha0, __ha1, …`
    // idents (H3), so counting the `__ha` tokens in the call's paren group gives the arity reliably.
    let mut referenced: BTreeMap<String, usize> = BTreeMap::new();
    let mut rest = module;
    while let Some(pos) = rest.find("crate::__cdz_host_") {
        let after = &rest[pos + "crate::".len()..];
        let end = after
            .find(|c: char| !(c == '_' || c.is_ascii_alphanumeric()))
            .unwrap_or(after.len());
        let ident = after[..end].to_string();
        let arity = after[end..]
            .strip_prefix('(')
            .and_then(|s| s.find(')').map(|c| &s[..c]))
            .map(|argstr| argstr.matches("__ha").count())
            .unwrap_or(0);
        referenced.entry(ident).or_insert(arity);
        rest = &after[end..];
    }
    if referenced.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (fn_name, &arity) in &referenced {
        // The shim's params are GENERIC + ignored — the arg VALUES crossed the boundary but do not select
        // the response (host_responses is keyed per-op, arg-independent) and the corpus host-call sequence
        // compares the op NAME only. `<A0: …>(_a0: A0)` accepts ANY arg type so a String/Bytes arg (H7)
        // type-checks without the driver knowing arg types.
        let generics = (0..arity)
            .map(|i| format!("A{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let generics = if generics.is_empty() {
            String::new()
        } else {
            format!("<{generics}>")
        };
        let params = (0..arity)
            .map(|i| format!("_a{i}: A{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        match by_ident.get(fn_name) {
            Some((op, values)) => {
                // RETURN TYPE keyed on the recorded response value text (matches the backend's per-result-
                // kind read): a quoted "…" → `String`; a `.`-bearing non-bool → `f64`; else `i64` (bool
                // true/false → 1/0). The `__V` response table is that type; the shim hands out one per call.
                let all_quoted = values.iter().all(|v| {
                    let t = v.trim();
                    t.starts_with('"') && t.ends_with('"') && t.len() >= 2
                });
                let is_float = !all_quoted
                    && values
                        .iter()
                        .any(|v| v.trim().contains('.') && v.trim() != "true" && v.trim() != "false");
                let (ret_ty, arr, is_owned) = if all_quoted {
                    (
                        "String".to_string(),
                        values
                            .iter()
                            .map(|v| format!("{}.to_string()", v.trim()))
                            .collect::<Vec<_>>()
                            .join(", "),
                        true,
                    )
                } else if is_float {
                    (
                        "f64".to_string(),
                        values.iter().map(|v| v.trim().to_string()).collect::<Vec<_>>().join(", "),
                        false,
                    )
                } else {
                    (
                        "i64".to_string(),
                        values
                            .iter()
                            .map(|v| match v.trim() {
                                "true" => "1".to_string(),
                                "false" => "0".to_string(),
                                other => other.to_string(),
                            })
                            .collect::<Vec<_>>()
                            .join(", "),
                        false,
                    )
                };
                let n = values.len();
                if is_owned {
                    // An owned (String/Vec) response can't live in a `static` array (non-const); build a
                    // fresh owned value per call, indexed by the call counter via a match.
                    let arms = values
                        .iter()
                        .enumerate()
                        .map(|(k, v)| format!("{k} => {}.to_string(),", v.trim()))
                        .collect::<Vec<_>>()
                        .join(" ");
                    out.push_str(&format!(
                        "#[allow(unused, non_snake_case)]\nfn {fn_name}{generics}({params}) -> {ret_ty} {{ \
                         use std::sync::atomic::{{AtomicUsize, Ordering}}; \
                         static __I: AtomicUsize = AtomicUsize::new(0); \
                         eprintln!(\"host-call\\t{op}\"); \
                         let __k = __I.fetch_add(1, Ordering::Relaxed); \
                         match __k {{ {arms} _ => unreachable!() }} }}\n"
                    ));
                } else {
                    out.push_str(&format!(
                        "#[allow(unused, non_snake_case)]\nfn {fn_name}{generics}({params}) -> {ret_ty} {{ \
                         use std::sync::atomic::{{AtomicUsize, Ordering}}; \
                         static __I: AtomicUsize = AtomicUsize::new(0); \
                         static __V: [{ret_ty}; {n}] = [{arr}]; \
                         eprintln!(\"host-call\\t{op}\"); \
                         let __k = __I.fetch_add(1, Ordering::Relaxed); \
                         __V[__k] }}\n"
                    ));
                }
            }
            // A referenced shim with NO recorded response: (a) a UNIT-RESULT op (H8) — a `()`-returning shim
            // that prints its canonical op; or (b) an UNEXERCISED def — a panic stub (never reached on a
            // passing trial) so the artifact links.
            None => match unit_ops.get(fn_name) {
                Some(op) => out.push_str(&format!(
                    "#[allow(unused, non_snake_case)]\nfn {fn_name}{generics}({params}) {{ \
                     eprintln!(\"host-call\\t{op}\"); }}\n"
                )),
                None => out.push_str(&format!(
                    "#[allow(unused, non_snake_case)]\nfn {fn_name}{generics}({params}) -> i64 {{ panic!(\"unexercised host op {fn_name}\") }}\n"
                )),
            },
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kebab_effect_normalizes() {
        assert_eq!(kebab_effect("Param"), "param");
        assert_eq!(kebab_effect("my_effect"), "my-effect");
        assert_eq!(kebab_effect("HttpClient"), "http-client");
        assert_eq!(kebab_effect(""), "");
    }

    #[test]
    fn shim_ident_matches_the_backend_mangling() {
        assert_eq!(
            host_shim_ident_from_key("Param.width"),
            "__cdz_host_param_width"
        );
        assert_eq!(host_shim_ident_from_key("io.log"), "__cdz_host_io_log");
    }

    #[test]
    fn value_response_shim_prints_canonical_op_and_returns_the_recorded_values() {
        let m = "let x = crate::__cdz_host_ask_ask();";
        let shims = build_rust_host_shims(m, &[("ask.ask".into(), "10".into())], &[]);
        assert!(
            shims.contains("fn __cdz_host_ask_ask"),
            "shim defined: {shims}"
        );
        assert!(
            shims.contains("host-call\\task.ask"),
            "prints the canonical op"
        );
        assert!(
            shims.contains("[10]") || shims.contains("[10 ]"),
            "returns the value: {shims}"
        );
        assert!(shims.contains("-> i64"), "int response → i64");
    }

    #[test]
    fn a_source_cased_response_key_prints_the_kebab_canonical_op() {
        let m = "crate::__cdz_host_param_width();";
        let shims = build_rust_host_shims(m, &[("Param.width".into(), "8".into())], &[]);
        // The IDENT is derived from the canonical form, so the source-cased key still matches the call site.
        assert!(shims.contains("fn __cdz_host_param_width"));
        assert!(
            shims.contains("host-call\\tparam.width"),
            "printed op is kebab-canonical: {shims}"
        );
    }

    #[test]
    fn a_unit_result_op_gets_a_unit_shim_that_prints_its_op() {
        let m = "crate::__cdz_host_log_emit(__ha0);";
        let shims = build_rust_host_shims(m, &[], &["log.emit".into()]);
        assert!(shims.contains("fn __cdz_host_log_emit"));
        assert!(shims.contains("host-call\\tlog.emit"));
        assert!(
            !shims.contains("-> i64"),
            "unit shim has no return type: {shims}"
        );
        assert!(
            shims.contains("<A0>"),
            "arity-1 shim is generic over its arg"
        );
    }

    #[test]
    fn an_unexercised_referenced_shim_is_a_panic_stub() {
        let m = "if false { crate::__cdz_host_dead_op(); }";
        let shims = build_rust_host_shims(m, &[], &[]);
        assert!(
            shims.contains("panic!(\"unexercised host op __cdz_host_dead_op\")"),
            "{shims}"
        );
    }

    #[test]
    fn no_referenced_shims_is_empty() {
        assert_eq!(build_rust_host_shims("fn main() {}", &[], &[]), "");
    }

    #[test]
    fn a_quoted_response_returns_owned_string_via_match() {
        let m = "crate::__cdz_host_ask_name();";
        let shims = build_rust_host_shims(m, &[("ask.name".into(), "\"hi\"".into())], &[]);
        assert!(shims.contains("-> String"), "quoted → String: {shims}");
        assert!(shims.contains(".to_string()"));
    }

    #[test]
    fn ordinary_call_marshals_scalar_and_compound_args() {
        let m = "pub fn f(a: i64, b: (i64, i64)) -> i64 { a }";
        assert_eq!(
            ordinary_call_expr(m, "f", &["20".into(), "(tuple 7 9)".into()], false),
            "f(20, (7, 9))"
        );
    }

    #[test]
    fn a_bigint_param_marshals_a_bare_decimal_via_big_arg_expr() {
        // A `cdz_num::Big` param must NOT get the bare `5` (i64 literal → E0308) — it goes through
        // big_arg_expr. A non-BigInt param keeps the verbatim scalar.
        let m = "pub fn f(a: cdz_num::Big, b: i64) -> i64 { b }";
        let got = marshal_call_args(m, "f", &["5".into(), "7".into()], false);
        assert_ne!(
            got[0], "5",
            "the BigInt arg is not a bare i64 literal: {got:?}"
        );
        assert!(
            got[0].contains("Big") || got[0].contains("big"),
            "BigInt marshal: {got:?}"
        );
        assert_eq!(got[1], "7", "the i64 arg passes through verbatim");
    }

    #[test]
    fn consumer_call_synthesizes_the_producer_closure() {
        // Consumer apply_it(g: Rc<dyn Fn(i64)->i64>, x) + factory producer make_adder(k) -> Rc<dyn Fn…>.
        // Flat call args [100, 7]: g pairs make_adder (1 cap → 100), x = 7 → apply_it(prog::make_adder(100), 7).
        let m = "// cdz-return[make_adder]: (-> Int64 Int64)\n\
                 pub fn make_adder(k: i64) -> std::rc::Rc<dyn Fn(i64) -> i64> { std::rc::Rc::new(move |x| x + k) }\n\
                 // cdz-return[apply_it]: Int64\n\
                 pub fn apply_it(g: std::rc::Rc<dyn Fn(i64) -> i64>, x: i64) -> i64 { g(x) }";
        assert_eq!(
            build_closure_consumer_call(m, "apply_it", &["100".into(), "7".into()], false)
                .as_deref(),
            Some("apply_it(prog::make_adder(100), 7)")
        );
        // A non-consumer (no closure param) → None (factory/ordinary path handles it).
        assert_eq!(
            build_closure_consumer_call("pub fn f(a: i64) -> i64 { a }", "f", &["1".into()], false),
            None
        );
        // A SYNC-emitted module parsed in async mode → None (no `pub async fn` header matches).
        assert_eq!(
            build_closure_consumer_call(m, "apply_it", &["100".into(), "7".into()], true),
            None
        );
    }

    #[test]
    fn async_consumer_call_drives_the_factory_closure_through_block_on() {
        // Async consumer apply_it(g: EnvClosure, x) + async FACTORY producer make_adder(env, k) -> Rc<dyn
        // EnvClosure>. The producer's closure is block_on-bound to __g0 FIRST (so its `&mut env` borrow doesn't
        // overlap the consumer's), then the consumer is driven with __g0 + the scalar arg. The whole thing is
        // a fully-driven `{ let __g0 = …; block_on(prog::apply_it(&mut env, __g0, 7)) }` block.
        let m = "// cdz-return[make_adder]: (-> Int64 Int64)\n\
                 pub async fn make_adder<E: cdz_rt::CdzEnv>(__cdz_env: &mut E, k: i64) -> std::rc::Rc<dyn cdz_rt::EnvClosure<i64, i64>> { todo!() }\n\
                 // cdz-return[apply_it]: Int64\n\
                 pub async fn apply_it<E: cdz_rt::CdzEnv>(__cdz_env: &mut E, g: std::rc::Rc<dyn cdz_rt::EnvClosure<i64, i64>>, x: i64) -> i64 { todo!() }";
        assert_eq!(
            build_closure_consumer_call(m, "apply_it", &["100".into(), "7".into()], true)
                .as_deref(),
            Some(
                "{ let __g0 = block_on(prog::make_adder(&mut env, 100)); block_on(prog::apply_it(&mut env, __g0, 7)) }"
            )
        );
    }

    #[test]
    fn bytes_list_render_iterates_bytes() {
        assert!(cdz_render_bytes_list("String").contains("(__r).bytes()"));
        assert!(cdz_render_bytes_list("Bytes").contains("(__r).iter().copied()"));
    }

    #[test]
    fn factory_sum_result_is_value_form_wrapped() {
        let empty = std::collections::HashMap::new();
        assert!(factory_result_is_value_form_sum("(Option Int64)", &empty));
        assert!(factory_result_is_value_form_sum("Option", &empty));
        assert!(factory_result_is_value_form_sum(
            "(Result Int64 String)",
            &empty
        ));
        assert!(!factory_result_is_value_form_sum("Int64", &empty));
        // A factory returning (Option (Tuple Int64 Int64)) → the driver wraps the value in (: v type).
        let m = "// cdz-return[mk]: (-> Int64 (Option (Tuple Int64 Int64)))\n\
                 pub fn mk() -> std::rc::Rc<dyn Fn(i64) -> Option<(i64, i64)>> { std::rc::Rc::new(|_| Some((100, 101))) }";
        let d = build_driver_source(m, "mk", &["0".into()], &[], &[], false);
        assert!(
            d.contains("(: {} (Option (Tuple Int64 Int64)))"),
            "factory sum result wrapped in the typed value form: {d}"
        );
    }

    #[test]
    fn factory_string_result_renders_as_a_byte_list() {
        let m = "// cdz-return[mk]: (-> Int64 String)\n\
                 pub fn mk() -> std::rc::Rc<dyn Fn(i64) -> String> { std::rc::Rc::new(|_| String::from(\"hi\")) }";
        let d = build_driver_source(m, "mk", &["0".into()], &[], &[], false);
        assert!(
            d.contains("(__r).bytes()"),
            "String factory result → byte list: {d}"
        );
    }

    #[test]
    fn call_expr_factory_splits_caps_from_applied() {
        // A factory `adder(k) -> Rc<dyn Fn(i64)->i64>` with call args [k=3, x=5]: K=1 capture → adder(3)(5).
        let m = "pub fn adder(k: i64) -> std::rc::Rc<dyn Fn(i64) -> i64> { std::rc::Rc::new(move |x| x + k) }";
        assert_eq!(
            call_expr(m, "adder", &["3".into(), "5".into()], false),
            "adder(3)(5)"
        );
        // A non-factory export is the ordinary single call.
        assert_eq!(
            call_expr(
                "pub fn f(a: i64, b: i64) -> i64 { a }",
                "f",
                &["1".into(), "2".into()],
                false
            ),
            "f(1, 2)"
        );
    }

    #[test]
    fn driver_factory_export_calls_split_and_renders_the_peeled_result() {
        let m = "// cdz-return[adder]: (-> Int64 Int64)\npub fn adder(k: i64) -> std::rc::Rc<dyn Fn(i64) -> i64> { std::rc::Rc::new(move |x| x + k) }";
        let d = build_driver_source(m, "adder", &["3".into(), "5".into()], &[], &[], false);
        assert!(d.contains("prog::adder(3)(5)"), "factory-split call: {d}");
        assert!(d.contains("println!"), "renders the peeled Int64 result");
    }

    #[test]
    fn a_nullary_export_call_has_empty_args() {
        assert_eq!(
            ordinary_call_expr("pub fn main() -> i64 { 42 }", "main", &[], false),
            "main()"
        );
    }

    #[test]
    fn driver_wraps_the_module_and_calls_prog_main() {
        let m = "pub fn main() -> i64 { 42 }";
        let d = build_driver_source(m, "main", &[], &[], &[], false);
        assert!(d.contains("mod prog {"), "wraps in mod prog: {d}");
        assert!(d.contains(m), "embeds the module");
        assert!(
            d.contains("fn main()") && d.contains("prog::main()"),
            "calls prog::main: {d}"
        );
        assert!(d.contains("println!"), "prints the result");
    }

    #[test]
    fn driver_renders_via_cdz_return_note_when_present() {
        let m = "// cdz-return[f]: Int64\npub fn f() -> i64 { 7 }";
        let d = build_driver_source(m, "f", &[], &[], &[], false);
        assert!(d.contains("let __r = prog::f()"), "binds the result: {d}");
        assert!(d.contains("println!"), "prints it");
    }

    #[test]
    fn hyphenated_export_looks_up_notes_by_rust_ident_not_raw_name() {
        // A HYPHENATED source export `mk-b`: the backend keys its `// cdz-*` notes by the emitted
        // `rust_ident` (`cdz-return[mk_b]`), not the raw name. `build_driver_source` receives the RAW
        // `mk-b` (as grade.rs passes it) and MUST look the note up by `ident` — else `ret_ty` is `None`,
        // the render falls to the bare-`Display` arm, and a `Vec`/compound result fails to build (E0277).
        // Regression for the corpus host-closure `mk-a`/`mk-b` List-result cases (21-host-closures);
        // every pre-existing test used a non-hyphenated name so this class was unpinned.
        let m = "// cdz-return[mk_b]: (List Int64)\npub fn mk_b() -> Vec<i64> { vec![7, 100] }";
        let d = build_driver_source(m, "mk-b", &[], &[], &[], false);
        assert!(
            d.contains("let __r"),
            "the (List Int64) note is found by rust_ident → result bound + structurally rendered, \
             not the bare-Display fallback: {d}"
        );
    }

    #[test]
    fn a_diverging_export_is_just_called_no_println() {
        let m = "// cdz-return[boom]: !\npub fn boom() -> ! { panic!() }";
        let d = build_driver_source(m, "boom", &[], &[], &[], false);
        assert!(d.contains("prog::boom()"), "calls it: {d}");
        assert!(!d.contains("println!"), "diverging → no print: {d}");
        assert!(!d.contains("let __r"), "diverging → no result binding: {d}");
    }

    #[test]
    fn async_driver_drives_a_nullary_export_via_block_on_and_splices_the_harness() {
        let m = "// cdz-return[run]: Int64\npub async fn run<E: cdz_rt::CdzEnv>(__cdz_env: &mut E) -> i64 { 7 }";
        let d = build_driver_source(m, "run", &[], &[], &[], true);
        assert!(
            d.contains("struct GateEnv"),
            "splices the async harness: {d}"
        );
        assert!(
            d.contains("let mut env = GateEnv;"),
            "binds env in main: {d}"
        );
        assert!(
            d.contains("block_on(prog::run(&mut env))"),
            "nullary async call threads &mut env: {d}"
        );
    }

    #[test]
    fn async_driver_threads_args_after_env() {
        let m = "// cdz-return[add]: Int64\npub async fn add<E: cdz_rt::CdzEnv>(__cdz_env: &mut E, a: i64, b: i64) -> i64 { a + b }";
        let d = build_driver_source(m, "add", &["3".into(), "5".into()], &[], &[], true);
        assert!(
            d.contains("block_on(prog::add(&mut env, 3, 5))"),
            "args follow &mut env: {d}"
        );
    }

    #[test]
    fn async_driver_applies_a_factory_via_env_closure_call() {
        // An async FACTORY `adder(env, k) -> Rc<dyn EnvClosure<i64,i64>>` with call args [k=3, x=5]: env
        // threads into the factory (block_on-bound to __h), then the returned handle is applied via
        // `__h.call(&mut env, 5)`.
        let m = "// cdz-return[adder]: (-> Int64 Int64)\npub async fn adder<E: cdz_rt::CdzEnv>(__cdz_env: &mut E, k: i64) -> std::rc::Rc<dyn cdz_rt::EnvClosure<i64, i64>> { todo!() }";
        let d = build_driver_source(m, "adder", &["3".into(), "5".into()], &[], &[], true);
        assert!(
            d.contains("let __h = block_on(prog::adder(&mut env, 3));")
                && d.contains("block_on(__h.call(&mut env, 5))"),
            "factory applied via EnvClosure::call: {d}"
        );
    }

    #[test]
    fn driver_splices_host_shims() {
        let m = "// cdz-return[g]: Int64\npub fn g() -> i64 { crate::__cdz_host_ask_ask() }";
        let d = build_driver_source(m, "g", &[], &[("ask.ask".into(), "10".into())], &[], false);
        assert!(d.contains("fn __cdz_host_ask_ask"), "shim spliced: {d}");
    }
}
