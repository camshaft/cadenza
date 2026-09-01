//! The Lean L2 differential oracle interface (S4): `oracle-check --batch-stream`.
//!
//! Authoritative wire contract: `implementation/oracle-lean/Oracle/Batch.lean` (the source the landed
//! `oracle-check` binary is built from). The **assertion** model the operator described ("run inputs in
//! rcdzc, collect the results, pass them to the lean oracle in batches; while lean is judging the
//! output, rcdzc works on the next batch"):
//!
//! cdz-smith runs a program under rcdzc (the wasm backend), captures rcdzc's OUTPUT (value/trap), and
//! hands the oracle a batch of TRIALS `(trial <program-ast> (args <v>…)? <output>)` where `<output>` is
//! `(value <ast-value>)` or `(trap "<reason>")`. The oracle re-derives the output and ASSERTS it matches
//! rcdzc's — per-trial `holds` / `mismatch` / `skip`. A `mismatch` is a candidate rcdzc bug; `skip` is a
//! coverage gap. cdz-smith pipelines: judge batch N while running/compiling batch N+1.
//!
//! ## Everything is the binary AST — no bespoke frame
//!
//! Per the operator's steer ("why aren't we using the AST? Lean already has an encoder/decoder"), the
//! WHOLE wire is the binary AST, encoded/decoded by the one `cadenza-ast` codec both sides already have
//! — there is NO hand-rolled length-prefix envelope (uleb128 survives only INSIDE the codec):
//!
//! ```text
//! REQUEST  = one cdzast blob:  (batch <trial1> <trial2> …)
//!            <trialN> = (trial <program> (args <v>…) (value <ast-value>)|(trap "<reason>"))
//! RESPONSE = one cdzast blob:  (verdicts <v1> <v2> …)   -- one child per trial, in order
//!            <vN> = (holds) | (mismatch <detail-str>) | (skip <reason-str>)
//! ```
//!
//! `oracle-check --batch-stream` reads the request blob on stdin (`Ast.decode`), iterates the `(batch …)`
//! children, judges each, and writes the `(verdicts …)` blob on stdout (`Ast.encode`).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cadenza_syntax::ast::{Arenas, Builder, Leaf, Struct, StructId};

/// rcdzc's captured output for one trial — what the oracle asserts its own re-derivation against.
#[derive(Debug, Clone)]
pub enum RcdzcOutput {
    /// Ran to a value; the value as an AST (e.g. `sexpr::read` of the rendered wasm result).
    Value(Arenas),
    /// Trapped, with a reason string (compared by canonical kind on the oracle side).
    Trap(String),
}

impl RcdzcOutput {
    /// Bridge rcdzc's RENDERED wasm result — the `Side::Value` string the differential produces (e.g.
    /// cdz-run's `"42"` / `"(tuple 1 2)"` after the `(: … Type)` annotation is stripped) — into a
    /// trial's `(value <ast>)` by parsing the render as a value-AST. Returns `None` if it does not parse
    /// (a non-canonical render): the caller then SKIPS that trial rather than sending a malformed value.
    /// This is the pure core of the S4b bridge from the wasm differential side to a Lean trial.
    pub fn value_from_render(rendered: &str) -> Option<RcdzcOutput> {
        cadenza_syntax::sexpr::read(rendered)
            .ok()
            .map(RcdzcOutput::Value)
    }
}

/// One trial: a program, its call arguments (as value-ASTs; empty for `main`/0-args), and the output
/// rcdzc produced for it, which the oracle asserts against.
#[derive(Debug, Clone)]
pub struct Trial {
    pub program: Arenas,
    pub args: Vec<Arenas>,
    pub output: RcdzcOutput,
}

impl Trial {
    /// A `main`/0-arg trial (the first pipeline shape) with the given rcdzc output.
    pub fn main_0(program: Arenas, output: RcdzcOutput) -> Trial {
        Trial {
            program,
            args: Vec::new(),
            output,
        }
    }
}

/// One symbolic-equivalence trial (v-lean-oracle T2 / #5719): prove the ORIGINAL program and its
/// `--target-cadenza` round-trip are functionally equivalent over ALL inputs. Unlike a [`Trial`] (which
/// asserts a captured OUTPUT), an equiv trial carries only the two PROGRAMS — the oracle binds fresh
/// symbolic input vars itself. Verdicts reuse the existing protocol: `(holds)` = proven-equivalent,
/// `(skip <reason>)` = cannot-prove — where the reason distinguishes `equiv: boundary: …` (hit the
/// incompleteness limit: let/match/collections/calls/recursion → degrade to the sampled cadenza-diff net)
/// from `equiv: normalized-but-different` (both sides fully normalized yet differ = a STRONG suspected
/// cadenza-backend miscompile to CONFIRM with a sampled run, then route to v-cadenza-backend).
#[derive(Debug, Clone)]
pub struct EquivTrial {
    /// The original program AST — a self-contained `(do (def (main …) BODY) (export main))`.
    pub orig: Arenas,
    /// Its `--target-cadenza` round-trip program AST (`program1.ast` from the cadenza build).
    pub cadenza: Arenas,
}

/// rcdzc's frontend `cdz check` verdict, carried in a [`TypecheckItem`] (design §1.2/§1.3): `Accept`,
/// `Reject(code)` (a CODED error-severity fault — the CDZ code, e.g. `"CDZ0203"`), or `Decline` (a
/// CODELESS "not yet implemented"). The false-reject-vs-capability-gap triage keys on which of the last
/// two rcdzc carried (a coded reject over a Lean-accept = a bug; a codeless decline = a known gap).
#[derive(Debug, Clone)]
pub enum RcdzcVerdict {
    Accept,
    Reject(String),
    Decline,
}

/// A TYPING assertion (design §1.3): the Lean type oracle infers a typing verdict for `program` and
/// compares it against rcdzc's carried `rcdzc_verdict`. The fuzzer drives this on rcdzc's REJECTED (and,
/// from T2, ACCEPTED) programs — a Lean-accepts over a coded reject is a FALSE-REJECT, a Lean-rejects over
/// an accept is a FALSE-ACCEPT (soundness hole). Verdicts reuse the existing protocol: `(holds)` = agree,
/// `(mismatch <detail>)` = a finding (detail names the direction), `(skip <reason>)` = the oracle declined.
#[derive(Debug, Clone)]
pub struct TypecheckItem {
    /// The program AST — a self-contained `(do (def (main …) BODY) (export main))`.
    pub program: Arenas,
    /// rcdzc's `cdz check` accept/reject/decline verdict for it.
    pub rcdzc_verdict: RcdzcVerdict,
}

/// One item in a batch: an output-assertion [`Trial`], a symbolic-equivalence [`EquivTrial`], or a typing
/// [`TypecheckItem`]. The oracle iterates batch children and emits one verdict per child IN ORDER, reusing
/// the verdict protocol for all — so a batch may freely MIX the kinds (each needs no decoder change).
#[derive(Debug, Clone)]
pub enum BatchItem {
    Trial(Trial),
    Equiv(EquivTrial),
    Typecheck(TypecheckItem),
}

/// Encode a batch of mixed [`BatchItem`]s as one `(batch <item>…)` binary-AST blob — the entire REQUEST
/// (the oracle `Ast.decode`s it and iterates the children; there is no separate frame). Each item is a
/// `(trial …)` or an `(equiv <P> <P'>)` node.
pub fn encode_batch(items: &[BatchItem]) -> Vec<u8> {
    let mut b = Builder::new();
    let mut kids = vec![b.name("batch")];
    for it in items {
        kids.push(match it {
            BatchItem::Trial(t) => build_trial(t, &mut b),
            BatchItem::Equiv(e) => build_equiv(e, &mut b),
            BatchItem::Typecheck(t) => build_typecheck(t, &mut b),
        });
    }
    let root = b.list(kids);
    cadenza_syntax::codec::encode(&b.finish(root))
}

/// Encode a batch of output-assertion trials (the original S4b path): a thin wrapper over
/// [`encode_batch`], kept so existing trial-only callers are unchanged.
pub fn encode_batch_request(trials: &[Trial]) -> Vec<u8> {
    let items: Vec<BatchItem> = trials.iter().cloned().map(BatchItem::Trial).collect();
    encode_batch(&items)
}

/// Build one `(trial <program> (args <v>…) (value <v>)|(trap "<reason>"))` node into builder `b`.
fn build_trial(t: &Trial, b: &mut Builder) -> StructId {
    let head = b.name("trial");
    let prog = graft(&t.program, t.program.root, b);

    let mut args_kids = vec![b.name("args")];
    for a in &t.args {
        args_kids.push(graft(a, a.root, b));
    }
    let args_node = b.list(args_kids);

    let output_node = match &t.output {
        RcdzcOutput::Value(v) => {
            let head = b.name("value");
            let val = graft(v, v.root, b);
            b.list(vec![head, val])
        }
        RcdzcOutput::Trap(reason) => {
            let head = b.name("trap");
            let leaf = b.atom_leaf(Leaf::Str(reason.as_str().into()));
            b.list(vec![head, leaf])
        }
    };

    b.list(vec![head, prog, args_node, output_node])
}

/// Build one `(equiv <orig-program> <cadenza-roundtrip-program>)` node into builder `b` — the T2
/// symbolic-equivalence trial (v-lean-oracle #5719). Head is the `equiv` name-leaf; child 0 = the
/// original program AST, child 1 = its `--target-cadenza` round-trip. No args/output node: the oracle
/// binds fresh symbolic input vars and proves equivalence for all inputs.
fn build_equiv(e: &EquivTrial, b: &mut Builder) -> StructId {
    let head = b.name("equiv");
    let orig = graft(&e.orig, e.orig.root, b);
    let cadenza = graft(&e.cadenza, e.cadenza.root, b);
    b.list(vec![head, orig, cadenza])
}

/// Build one `(typecheck <program> <rcdzc-verdict>)` node — the TYPING dimension (design §1.3). The
/// carried verdict is `(accept)` | `(reject "<CODE>")` | `(decline)`. Mirrors `build_trial`/`build_equiv`;
/// the oracle runs `infer` on the program and compares against this verdict (`Oracle/Batch.lean`
/// `judgeTypecheckNode`). No verdict-protocol change — a `(typecheck …)` reuses `holds/mismatch/skip`.
fn build_typecheck(t: &TypecheckItem, b: &mut Builder) -> StructId {
    let head = b.name("typecheck");
    let prog = graft(&t.program, t.program.root, b);
    let verdict = match &t.rcdzc_verdict {
        RcdzcVerdict::Accept => {
            let h = b.name("accept");
            b.list(vec![h])
        }
        RcdzcVerdict::Reject(code) => {
            let h = b.name("reject");
            let leaf = b.atom_leaf(Leaf::Str(code.as_str().into()));
            b.list(vec![h, leaf])
        }
        RcdzcVerdict::Decline => {
            let h = b.name("decline");
            b.list(vec![h])
        }
    };
    b.list(vec![head, prog, verdict])
}

/// Copy the subtree at `id` of `src` into builder `b`, returning its new id (structural graft — the AST
/// is a tree, each occurrence copied). Splices a program / value AST into the batch tree.
fn graft(src: &Arenas, id: StructId, b: &mut Builder) -> StructId {
    match src.get(id) {
        Struct::Atom(leaf) => b.atom_leaf(src.leaf(*leaf).clone()),
        Struct::List(kids) => {
            let new: Vec<StructId> = kids.clone().into_iter().map(|k| graft(src, k, b)).collect();
            b.list(new)
        }
    }
}

/// One trial's verdict from the oracle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The oracle's re-derived output matched rcdzc's — no bug.
    Holds,
    /// The outputs disagree — a candidate rcdzc miscompile; the string is the oracle's detail.
    Mismatch(String),
    /// A coverage gap (undecodable trial, or a construct the oracle does not model yet) — never a bug.
    Skip(String),
}

/// A malformed `(verdicts …)` response blob from `oracle-check`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// The response bytes did not decode as a binary AST.
    Undecodable,
    /// The root was not a `(verdicts …)` node.
    NotVerdicts,
    /// A child was not a well-formed `(holds)` / `(mismatch <str>)` / `(skip <str>)` node.
    BadVerdict,
}

/// Decode a `(verdicts <v>…)` response blob into per-trial verdicts, in order.
pub fn decode_verdicts(bytes: &[u8]) -> Result<Vec<Verdict>, FrameError> {
    let a = cadenza_syntax::codec::decode(bytes).ok_or(FrameError::Undecodable)?;
    let Struct::List(kids) = a.get(a.root) else {
        return Err(FrameError::NotVerdicts);
    };
    if kids.first().and_then(|&h| a.as_name(h)) != Some("verdicts") {
        return Err(FrameError::NotVerdicts);
    }
    let mut out = Vec::with_capacity(kids.len().saturating_sub(1));
    for &vid in &kids[1..] {
        let Struct::List(vk) = a.get(vid) else {
            return Err(FrameError::BadVerdict);
        };
        let verdict = match vk.first().and_then(|&h| a.as_name(h)) {
            Some("holds") => Verdict::Holds,
            Some("mismatch") => Verdict::Mismatch(str_child(&a, vk).ok_or(FrameError::BadVerdict)?),
            Some("skip") => Verdict::Skip(str_child(&a, vk).ok_or(FrameError::BadVerdict)?),
            _ => return Err(FrameError::BadVerdict),
        };
        out.push(verdict);
    }
    Ok(out)
}

/// The `Str` leaf payload of a single-argument node `(head <"str">)`, if present.
fn str_child(a: &Arenas, node_kids: &[StructId]) -> Option<String> {
    let &payload = node_kids.get(1)?;
    match a.get(payload) {
        Struct::Atom(leaf) => match a.leaf(*leaf) {
            Leaf::Str(s) => Some(s.to_string()),
            _ => None,
        },
        Struct::List(_) => None,
    }
}

/// Discover the `oracle-check` binary: `CDZ_SMITH_ORACLE_CHECK` env, else `oracle-check` on `PATH`.
/// Build it with `nix build .#oracle-lean` (the binary lands at `result/bin/oracle-check`).
pub fn discover_oracle_check() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CDZ_SMITH_ORACLE_CHECK") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join("oracle-check"))
        .find(|c| c.is_file())
}

/// Judge one batch by invoking `oracle-check --batch-stream`: write the `(batch …)` request blob to its
/// stdin, read the `(verdicts …)` blob from its stdout, decode it. v1 `oracle-check` reads one batch per
/// invocation (read-all-stdin → one response), so this spawns a fresh process per batch — which is the
/// async unit the pipeline overlaps (run this on a worker thread while the next batch compiles).
pub fn judge_batch(oracle_bin: &Path, trials: &[Trial]) -> std::io::Result<Vec<Verdict>> {
    run_oracle(oracle_bin, &encode_batch_request(trials))
}

/// Judge a batch of MIXED [`BatchItem`]s (output-assertion trials and/or symbolic-equivalence equiv
/// trials). Identical process protocol to [`judge_batch`] — one `--batch-stream` invocation, one
/// `(verdicts …)` response, one verdict per item in order.
pub fn judge_batch_items(oracle_bin: &Path, items: &[BatchItem]) -> std::io::Result<Vec<Verdict>> {
    run_oracle(oracle_bin, &encode_batch(items))
}

/// Send an already-encoded `(batch …)` request blob to `oracle-check --batch-stream` and decode the
/// `(verdicts …)` response. The shared tail of [`judge_batch`] / [`judge_batch_items`]: spawns a fresh
/// process per batch (the async unit the pipeline overlaps — run on a worker thread while the next batch
/// compiles), writes the request to stdin (drop → EOF), reads stdout, decodes.
fn run_oracle(oracle_bin: &Path, request: &[u8]) -> std::io::Result<Vec<Verdict>> {
    let mut child = Command::new(oracle_bin)
        .arg("--batch-stream")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    {
        let mut stdin = child.stdin.take().expect("stdin was piped");
        stdin.write_all(request)?;
    } // drop stdin → EOF, so the oracle's read-all-stdin returns
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "oracle-check --batch-stream exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    decode_verdicts(&output.stdout).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("bad (verdicts …) response: {e:?}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ast(source: &str) -> Arenas {
        cadenza_syntax::sexpr::read(source).expect("test source parses")
    }

    /// A batch request is one `(batch (trial …) …)` AST blob, decodable by the shared codec, with each
    /// child a well-formed `(trial <program> (args) (value|trap …))`.
    #[test]
    fn encode_batch_request_is_one_ast_blob() {
        let trials = vec![
            Trial::main_0(
                ast("(do (def (main) 42) (export main))"),
                RcdzcOutput::Value(ast("42")),
            ),
            Trial::main_0(
                ast("(do (def (main) (/ 1 0)) (export main))"),
                RcdzcOutput::Trap("div-by-zero".into()),
            ),
        ];
        let blob = encode_batch_request(&trials);
        let a = cadenza_syntax::codec::decode(&blob).expect("batch blob decodes via the AST codec");

        let Struct::List(kids) = a.get(a.root) else {
            panic!("root not a list");
        };
        assert_eq!(a.as_name(kids[0]), Some("batch"));
        assert_eq!(kids.len(), 3, "batch head + 2 trials");

        // First trial: (trial <prog> (args) (value 42)).
        let Struct::List(t0) = a.get(kids[1]) else {
            panic!("trial not a list");
        };
        assert_eq!(a.as_name(t0[0]), Some("trial"));
        let Struct::List(args0) = a.get(t0[2]) else {
            panic!("args not a list");
        };
        assert_eq!(a.as_name(args0[0]), Some("args"));
        assert_eq!(args0.len(), 1, "no args for main/0");
        let Struct::List(out0) = a.get(t0[3]) else {
            panic!("output not a list");
        };
        assert_eq!(a.as_name(out0[0]), Some("value"));

        // Second trial output: (trap "div-by-zero").
        let Struct::List(t1) = a.get(kids[2]) else {
            panic!()
        };
        let Struct::List(out1) = a.get(t1[3]) else {
            panic!()
        };
        assert_eq!(a.as_name(out1[0]), Some("trap"));
    }

    /// `value_from_render` parses a rendered wasm value into a `(value <ast>)` trial output; a trial
    /// built from it encodes into a `(batch (trial … (value 42)))`. A non-parsing render → `None` (skip).
    #[test]
    fn value_from_render_bridges_a_rendered_value_into_a_trial() {
        let out = RcdzcOutput::value_from_render("42").expect("canonical value parses");
        assert!(matches!(out, RcdzcOutput::Value(_)));
        let blob = encode_batch_request(&[Trial::main_0(
            ast("(do (def (main) 42) (export main))"),
            out,
        )]);
        let a = cadenza_syntax::codec::decode(&blob).expect("decodes");
        let Struct::List(kids) = a.get(a.root) else {
            panic!()
        };
        let Struct::List(trial) = a.get(kids[1]) else {
            panic!()
        };
        let Struct::List(output) = a.get(trial[3]) else {
            panic!()
        };
        assert_eq!(a.as_name(output[0]), Some("value"));
        // A render that isn't a well-formed value → None (the caller skips the trial).
        assert!(RcdzcOutput::value_from_render("(( not balanced").is_none());
    }

    /// An `(equiv <orig> <cadenza-roundtrip>)` node encodes with head `equiv` and the two PROGRAM
    /// children in order (no args/output node) — the T2 symbolic-equivalence wire shape (#5719).
    #[test]
    fn encode_equiv_builds_the_equiv_node() {
        let orig = ast("(do (def (main (: n Int64)) (+ n 1)) (export main))");
        let cadenza = ast("(do (def (main (: n Int64)) (+ 1 n)) (export main))");
        let items = vec![BatchItem::Equiv(EquivTrial { orig, cadenza })];
        let blob = encode_batch(&items);
        let a = cadenza_syntax::codec::decode(&blob).expect("equiv batch decodes");
        let Struct::List(kids) = a.get(a.root) else {
            panic!("root not a list");
        };
        assert_eq!(a.as_name(kids[0]), Some("batch"));
        assert_eq!(kids.len(), 2, "batch head + 1 equiv item");
        let Struct::List(eq) = a.get(kids[1]) else {
            panic!("equiv not a list");
        };
        assert_eq!(
            a.as_name(eq[0]),
            Some("equiv"),
            "head is the equiv name-leaf"
        );
        assert_eq!(
            eq.len(),
            3,
            "equiv head + 2 program children (no args/output)"
        );
        // Both children are programs — a `(do …)`-rooted list, not an atom.
        for &child in &eq[1..] {
            assert!(
                matches!(a.get(child), Struct::List(_)),
                "each equiv child is a program AST"
            );
        }
    }

    /// A batch may MIX a `(trial …)` and an `(equiv …)`; both appear as children in order (the oracle
    /// emits one verdict per child, reusing the verdict protocol for both).
    #[test]
    fn encode_batch_mixes_trial_and_equiv() {
        let items = vec![
            BatchItem::Trial(Trial::main_0(
                ast("(do (def (main) 42) (export main))"),
                RcdzcOutput::Value(ast("42")),
            )),
            BatchItem::Equiv(EquivTrial {
                orig: ast("(do (def (main) (+ 1 2)) (export main))"),
                cadenza: ast("(do (def (main) 3) (export main))"),
            }),
        ];
        let blob = encode_batch(&items);
        let a = cadenza_syntax::codec::decode(&blob).expect("mixed batch decodes");
        let Struct::List(kids) = a.get(a.root) else {
            panic!("root not a list");
        };
        assert_eq!(a.as_name(kids[0]), Some("batch"));
        assert_eq!(kids.len(), 3, "batch head + trial + equiv");
        let Struct::List(t) = a.get(kids[1]) else {
            panic!("first item not a list");
        };
        assert_eq!(a.as_name(t[0]), Some("trial"), "first child is the trial");
        let Struct::List(e) = a.get(kids[2]) else {
            panic!("second item not a list");
        };
        assert_eq!(
            a.as_name(e[0]),
            Some("equiv"),
            "second child is the equiv node"
        );
        // The trial-only wrapper produces the identical bytes as the equivalent BatchItem list.
        assert_eq!(
            encode_batch_request(&[Trial::main_0(
                ast("(do (def (main) 42) (export main))"),
                RcdzcOutput::Value(ast("42")),
            )]),
            encode_batch(&[BatchItem::Trial(Trial::main_0(
                ast("(do (def (main) 42) (export main))"),
                RcdzcOutput::Value(ast("42")),
            ))]),
            "encode_batch_request is a thin wrapper over encode_batch"
        );
    }

    /// A hand-built `(verdicts (holds) (mismatch "d") (skip "r"))` AST decodes to the three verdicts in
    /// order — the exact response shape `oracle-check` emits.
    #[test]
    fn decode_verdicts_reads_a_verdicts_ast() {
        let mut b = Builder::new();
        let holds = {
            let h = b.name("holds");
            b.list(vec![h])
        };
        let mismatch = {
            let h = b.name("mismatch");
            let s = b.atom_leaf(Leaf::Str("d".into()));
            b.list(vec![h, s])
        };
        let skip = {
            let h = b.name("skip");
            let s = b.atom_leaf(Leaf::Str("r".into()));
            b.list(vec![h, s])
        };
        let vhead = b.name("verdicts");
        let root = b.list(vec![vhead, holds, mismatch, skip]);
        let bytes = cadenza_syntax::codec::encode(&b.finish(root));

        let verdicts = decode_verdicts(&bytes).unwrap();
        assert_eq!(
            verdicts,
            vec![
                Verdict::Holds,
                Verdict::Mismatch("d".into()),
                Verdict::Skip("r".into()),
            ]
        );
    }

    /// An empty `(verdicts)` decodes to no verdicts.
    #[test]
    fn decode_empty_verdicts() {
        let mut b = Builder::new();
        let vhead = b.name("verdicts");
        let root = b.list(vec![vhead]);
        let bytes = cadenza_syntax::codec::encode(&b.finish(root));
        assert_eq!(decode_verdicts(&bytes).unwrap(), vec![]);
    }

    #[test]
    fn decode_verdicts_rejects_non_verdicts_and_garbage() {
        // Garbage bytes → Undecodable.
        assert_eq!(decode_verdicts(b"not an ast"), Err(FrameError::Undecodable));
        // A well-formed AST that isn't a (verdicts …) root → NotVerdicts.
        let other = cadenza_syntax::codec::encode(&ast("(nope)"));
        assert_eq!(decode_verdicts(&other), Err(FrameError::NotVerdicts));
    }

    /// END-TO-END against the REAL `oracle-check --batch-stream`. Runs ONLY when `CDZ_SMITH_ORACLE_CHECK`
    /// points at an AST-envelope oracle (`nix build .#oracle-lean` → `result/bin/oracle-check`), since the
    /// pre-pivot binary speaks the old uleb frame. Mirrors `Batch.lean`'s `_trialHolds`:
    /// `(trial (do (def (main) 42) (export main)) (args) (value 42))` → the oracle re-derives 42, matches
    /// rcdzc's 42 → `holds`; a lied `(value 43)` → `mismatch`, proving the assertion fires.
    #[test]
    fn end_to_end_against_oracle_check() {
        let Some(oracle) = discover_oracle_check() else {
            eprintln!(
                "skipping: no oracle-check (nix build .#oracle-lean; set CDZ_SMITH_ORACLE_CHECK)"
            );
            return;
        };
        let program = ast("(do (def (main) 42) (export main))");
        let trials = vec![
            Trial::main_0(program.clone(), RcdzcOutput::Value(ast("42"))),
            Trial::main_0(program, RcdzcOutput::Value(ast("43"))),
        ];
        let verdicts = judge_batch(&oracle, &trials).expect("oracle-check --batch-stream runs");
        assert_eq!(verdicts.len(), 2);
        assert_eq!(verdicts[0], Verdict::Holds, "42==42 must hold");
        assert!(
            matches!(verdicts[1], Verdict::Mismatch(_)),
            "claiming rcdzc produced 43 must MISMATCH the oracle's 42, got {:?}",
            verdicts[1]
        );
    }

    /// END-TO-END self-test of the `(equiv P P')` encoder against the REAL oracle, following the exact
    /// contract v-lean-oracle verified + pinned (`Oracle/Batch.lean` `_batchEquiv`, #5729/#5719): a
    /// `(batch (equiv P P))` with IDENTICAL P must judge `(holds)` (proven-equivalent), and a flipped
    /// literal (`42` vs `43`) must judge `(skip "equiv: normalized-but-different")` (cannot-prove).
    ///
    /// Runs ONLY when the oracle is discoverable, and is ROBUST to oracle VERSION SKEW: a pre-#5719
    /// oracle does not know the `(equiv …)` node and rejects it with a `(skip "…not (trial …)")`, so
    /// this treats that stale-oracle signal as "skip" rather than a failure (the NOTE below's don't-
    /// couple-to-oracle-artifact-version discipline). Only an equiv-AWARE oracle exercises the
    /// assertions; the wire-shape unit tests above cover the encoder version-independently.
    #[test]
    fn equiv_self_test_against_oracle_check() {
        let Some(oracle) = discover_oracle_check() else {
            eprintln!(
                "skipping: no oracle-check (nix build .#oracle-lean; set CDZ_SMITH_ORACLE_CHECK)"
            );
            return;
        };
        let p = ast("(do (def (main) 42) (export main))");
        // Case 1: identical programs → PROVEN equivalent (on an equiv-aware oracle).
        let holds = judge_batch_items(
            &oracle,
            &[BatchItem::Equiv(EquivTrial {
                orig: p.clone(),
                cadenza: p.clone(),
            })],
        )
        .expect("oracle judges the equiv batch");
        // A pre-#5719 oracle rejects the unknown node ("…not (trial …)"); skip rather than fail on skew.
        if let [Verdict::Skip(reason)] = holds.as_slice()
            && reason.contains("not (trial")
        {
            eprintln!(
                "skipping: oracle predates the (equiv …) node (#5719) — got stale skip {reason:?}; \
                 rebuild with `nix build .#oracle-lean`"
            );
            return;
        }
        assert_eq!(
            holds,
            vec![Verdict::Holds],
            "(equiv P P) must be proven equivalent"
        );
        // Case 2: a flipped literal → CANNOT-PROVE (normalized-but-different).
        let differ = judge_batch_items(
            &oracle,
            &[BatchItem::Equiv(EquivTrial {
                orig: p,
                cadenza: ast("(do (def (main) 43) (export main))"),
            })],
        )
        .expect("oracle judges the equiv batch");
        assert_eq!(differ.len(), 1);
        assert!(
            matches!(differ[0], Verdict::Skip(_)),
            "(equiv 42 43) must be cannot-prove (skip normalized-but-different), got {:?}",
            differ[0]
        );
    }

    /// The `(typecheck …)` encoder builds a valid `(typecheck <program> (reject "<CODE>"))` node inside a
    /// `(batch …)` — round-trips through the shared codec. Version-independent (no oracle needed).
    #[test]
    fn encode_typecheck_builds_the_typecheck_node() {
        let items = vec![BatchItem::Typecheck(TypecheckItem {
            program: ast("(do (def (main) 42) (export main))"),
            rcdzc_verdict: RcdzcVerdict::Reject("CDZ0203".into()),
        })];
        let blob = encode_batch(&items);
        let a = cadenza_syntax::codec::decode(&blob).expect("typecheck batch decodes");
        let Struct::List(kids) = a.get(a.root) else {
            panic!("root not a list");
        };
        assert_eq!(a.as_name(kids[0]), Some("batch"));
        assert_eq!(kids.len(), 2, "batch head + typecheck");
        let Struct::List(tc) = a.get(kids[1]) else {
            panic!("item not a list");
        };
        assert_eq!(
            a.as_name(tc[0]),
            Some("typecheck"),
            "child is the typecheck node"
        );
        // child 2 is the rcdzc verdict `(reject "CDZ0203")`
        let Struct::List(rv) = a.get(tc[2]) else {
            panic!("verdict not a list");
        };
        assert_eq!(
            a.as_name(rv[0]),
            Some("reject"),
            "carried verdict is (reject …)"
        );
    }

    /// END-TO-END: a `(typecheck P (reject "CDZ0203"))` against the real oracle judges `(skip …)` — T0.1's
    /// `infer` is all-declining, so every typecheck item skips. ROBUST to version skew: a pre-typecheck
    /// oracle rejects the unknown node with its OWN skip, so the `Skip` assertion holds either way. Verifies
    /// the typecheck wire round-trips through the real oracle process (the fuzzer's real invocation).
    #[test]
    fn typecheck_self_test_against_oracle_check() {
        let Some(oracle) = discover_oracle_check() else {
            eprintln!(
                "skipping: no oracle-check (nix build .#oracle-lean; set CDZ_SMITH_ORACLE_CHECK)"
            );
            return;
        };
        let v = judge_batch_items(
            &oracle,
            &[BatchItem::Typecheck(TypecheckItem {
                program: ast("(do (def (main) 42) (export main))"),
                rcdzc_verdict: RcdzcVerdict::Reject("CDZ0203".into()),
            })],
        )
        .expect("oracle judges the typecheck batch");
        assert_eq!(v.len(), 1);
        assert!(
            matches!(v[0], Verdict::Skip(_)),
            "T0.1 declining infer ⇒ a typecheck item must skip, got {:?}",
            v[0]
        );
    }
}
