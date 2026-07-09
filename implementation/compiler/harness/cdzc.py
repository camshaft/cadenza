#!/usr/bin/env python3
"""One harness for the Cadenza-authored compiler `cdzc.cdz`. Standardizes on ONE runner —
`cadenza-seed emit` — which already compiles, LINKS THE RUNTIME, and runs `main`, printing
`ran → Value(...)` / `ran → Trap(...)`. We do NOT re-invent runtime linking or run bare wasmtime
(that path can't resolve the `cadenza:runtime/heap` imports → "box-int has the wrong type"): the seed
binary is the runner. See implementation/seed/crates/cadenza-seed/src/host.rs.

Three operations, all via `emit`:

  eval(src)              → Outcome   run a plain Cadenza program (seed-gap probes; no cdzc)
  probe(expr)            → Outcome   run `expr` INSIDE cdzc's module (inject `(def (main) expr)`),
                                     so cdzc's own defs — decode/resolve/lower/select/serialize — are
                                     in scope. For inspecting a pipeline STAGE.
  compile(program_src)   → Outcome   compile a Cadenza program WITH cdzc end-to-end: get its AST bytes
                                     (Ast.encode∘quote), inject `(def (main) (compile-bytes <bytes>))`,
                                     run → cdzc's OUTPUT is the emitted component bytes (Outcome.bytes),
                                     then optionally RUN that component (compile_run) for its value.

An Outcome is (kind, payload):
  kind='value'  payload=str      the rendered `run()` value ("3", "\"hi\"", "b\"AB\"")
  kind='bytes'  payload=bytes    a Bytes value, decoded (cdzc output components arrive here)
  kind='trap'   payload=str      a runtime trap (overflow, div0, an internal decline→unreachable)
  kind='decline'payload=str      the seed DECLINED to compile the program (a seed gap)
  kind='invalid'payload=str      the seed emitted an invalid component (a bug)
  kind='error'  payload=str      emit failed / parse error / no recognizable output

Usage (CLI):
  cdzc.py self                          # does cdzc self-compile on the stable seed?
  cdzc.py eval  '(module m (def (main) (+ 1 2)))'
  cdzc.py probe '(match (decode <BYTES-OF "(module c (def (main) 42))">) ((Ast.List xs)(List.len xs))(_ -1))'
  cdzc.py compile '(module c (def (main) 42))'      # end-to-end: cdzc compiles it, then runs the result
  cdzc.py astbytes '(module c (def (main) 42))'     # show the CBOR AST bytes hex

`<BYTES-OF "...">` in a probe expr is a macro: it's replaced by the literal
`(Bytes.of (list 0x.. ..))` of that program's AST bytes, so probes needn't hand-transcribe hex.
"""
import re, os, sys, subprocess, tempfile

REPO = "/Users/bythewc/Projects/camshaft/cadenza"
SEED = os.environ.get("CDZ_SEED", f"{REPO}/implementation/stable/cadenza-seed")
RUNTIME = os.environ.get("CADENZA_RUNTIME", f"{REPO}/implementation/stable/cdz_runtime.wasm")
CDZC = f"{REPO}/implementation/compiler/cdzc.cdz"
ENV = {**os.environ, "CADENZA_RUNTIME": RUNTIME}


class Outcome:
    __slots__ = ("kind", "payload", "raw")
    def __init__(self, kind, payload, raw=""):
        self.kind, self.payload, self.raw = kind, payload, raw
    def __repr__(self):
        p = self.payload
        if self.kind == "bytes":
            p = f"<{len(p)} bytes> {p[:24].hex()}"
        return f"{self.kind}: {p}"
    def __eq__(self, other):   # Outcome("value","3") == ("value","3")
        return (self.kind, self.payload) == tuple(other)


def _deesc(inner):
    """Decode the seed's Rust-Debug `b"..."` value string to raw bytes (two unescape levels)."""
    def un(t):
        o = []; k = 0
        while k < len(t):
            c = t[k]
            if c == '\\':
                n = t[k + 1]
                if n == 'x':
                    o.append(chr(int(t[k + 2:k + 4], 16))); k += 4; continue
                o.append({'0': '\0', 'r': '\r', 'n': '\n', 't': '\t',
                          '\\': '\\', '"': '"', "'": "'"}.get(n, n)); k += 2; continue
            o.append(c); k += 1
        return ''.join(o)
    l1 = un(inner)
    if not (l1.startswith('b"') and l1.endswith('"')):
        return None
    return bytes(ord(c) for c in un(l1[2:-1]))


def _run_emit(src, timeout=60):
    """The ONE primitive: write src, `cadenza-seed emit`, classify stdout into an Outcome."""
    with tempfile.NamedTemporaryFile('w', suffix='.cdz', delete=False) as f:
        f.write(src); p = f.name
    try:
        r = subprocess.run([SEED, "emit", p], env=ENV, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return Outcome("error", "timeout")
    finally:
        os.unlink(p)
    out = r.stdout + r.stderr
    m = re.search(r'declined:\s*(.*)', out)
    if m:
        return Outcome("decline", m.group(1).strip(), out)
    if 'parse error' in out or 'error:' in out.lower() and 'ran →' not in out:
        # a compile-time failure that isn't a clean decline
        if 'INVALID' not in out and 'ran →' not in out:
            return Outcome("error", _first(out), out)
    m = re.search(r'INVALID:\s*(.*)', out)
    if m:
        return Outcome("invalid", m.group(1).strip(), out)
    m = re.search(r'ran → Trap\((.*)\)\s*$', out, re.S | re.M)
    if m:
        return Outcome("trap", _first(m.group(1)), out)
    # The runner renders `ran → Value("<rust-debug of the rendered value>")`. Capture INSIDE the
    # outer quotes (like run_corpus.py) — the inner is a `b"…"` byte-string render for a Bytes value,
    # else a plain scalar/string render.
    m = re.search(r'ran → Value\("(.*)"\)\s*$', out, re.S | re.M)
    if m:
        inner = m.group(1)
        b = _deesc(inner)             # non-None iff inner is a `b"…"` byte-string render
        if b is not None:
            return Outcome("bytes", b, out)
        # a plain scalar/string: undo one layer of Rust-debug escaping
        v = inner.replace('\\"', '"').replace('\\\\', '\\')
        return Outcome("value", v, out)
    return Outcome("error", _first(out), out)


def _first(s):
    return next((l for l in s.splitlines() if l.strip()), "")[:200]


# ─── AST bytes ─────────────────────────────────────────────────────────────────
def ast_bytes(program_src):
    """The canonical CBOR AST bytes of a program, via the seed's own `Ast.encode∘quote`."""
    o = _run_emit(f'(module m (def (main) (Ast.encode (quote {program_src}))))')
    if o.kind != "bytes":
        raise RuntimeError(f"ast_bytes({program_src!r}) failed: {o!r}")
    return o.payload


def _bytes_lit(b):
    return "(Bytes.of (list " + " ".join(f"0x{x:02X}" for x in b) + "))"


# ─── the cdzc module head/tail (inject a main before the closing paren) ─────────
# We assemble the test module DIRECTLY from the `cdzc/*.cdz` sources (sorted), wrapping them in
# `(module cdzc … )`, rather than parsing the Makefile's `cdzc.cdz` output. The Makefile now emits the
# v2 `(export compile-bytes)` top-level form, which the STABLE seed can't parse — so the harness owns
# its own stable-compatible wrapper and stays decoupled from that format change. Injection point is
# the module's final `)`.
import glob as _glob

_CDZC_SRC_DIR = f"{REPO}/implementation/compiler/cdzc"

def _cdzc_module():
    """The cdzc sources merged into a `(module cdzc … )` the stable seed parses. Returns (head, tail)
       where `head` is everything up to (not including) the final `)` and `tail` is that `)`."""
    parts = ["(module cdzc"]
    for f in sorted(_glob.glob(f"{_CDZC_SRC_DIR}/*.cdz")):
        parts.append(open(f).read())
    body = "\n".join(parts) + "\n)"
    i = body.rfind(")")
    return body[:i], body[i:]

def _cdzc_split():
    return _cdzc_module()


_BYTES_OF = re.compile(r'<BYTES-OF\s+"((?:[^"\\]|\\.)*)"\s*>')

def _expand(expr):
    """Replace `<BYTES-OF "prog">` macros in a probe expr with the literal AST-bytes Bytes.of form."""
    return _BYTES_OF.sub(lambda m: _bytes_lit(ast_bytes(m.group(1))), expr)


# ─── the three operations ───────────────────────────────────────────────────────
def eval(src):
    """Run a plain Cadenza program (no cdzc). For seed-gap probes."""
    return _run_emit(src)


def probe(expr):
    """Run `expr` inside cdzc's module — cdzc's defs (decode/resolve/lower/select/serialize) are in
       scope. Supports the `<BYTES-OF "prog">` macro. For inspecting a single pipeline stage."""
    head, tail = _cdzc_split()
    return _run_emit(f"{head}\n(def (main) {_expand(expr)})\n{tail}")


def compile(program_src):
    """Compile `program_src` with cdzc end-to-end; Outcome.kind='bytes' is cdzc's emitted component."""
    head, tail = _cdzc_split()
    lit = _bytes_lit(ast_bytes(program_src))
    return _run_emit(f"{head}\n(def (main) (compile-bytes {lit}))\n{tail}")


def compile_run(program_src):
    """Compile with cdzc, then RUN the emitted component via the seed → the program's VALUE/TRAP.
       This is the true end-to-end oracle: does cdzc's output compute the right thing?"""
    o = compile(program_src)
    if o.kind != "bytes":
        return o                      # cdzc declined / errored before producing a component
    with tempfile.NamedTemporaryFile('wb', suffix='.wasm', delete=False) as f:
        f.write(o.payload); p = f.name
    try:
        # `emit` on a .wasm won't parse; the seed RUNS a component only via emit of source. So we
        # invoke the component through the seed's runner by writing a tiny driver is not possible —
        # instead we shell to the seed's component runner if present, else wasmtime with the runtime.
        return _run_component(p)
    finally:
        os.unlink(p)


def _run_component(path):
    """Run an already-emitted component through wasmtime, linking the runtime the way the seed does.
       cdzc's OUTPUT components export `run : () -> s64` and (for scalar bodies) import NOTHING, so
       bare wasmtime suffices; a heap-importing output would need the runtime linked (not yet emitted
       by cdzc). Kept minimal on purpose — the seed's `emit` runner is the oracle for SOURCE."""
    r = subprocess.run(["wasmtime", "run", "-W", "component-model=y", "--invoke", "run()", path],
                       capture_output=True, text=True, timeout=30, env=ENV)
    out = (r.stdout + r.stderr).strip()
    if r.returncode == 0:
        v = r.stdout.strip()
        return Outcome("value", "unit" if v == "()" else v, out)
    if "wrong type" in out or "not found in the linker" in out:
        return Outcome("error", "output component needs the runtime linked (heap import) — "
                                "use the seed runner path", out)
    if "trap" in out.lower() or "unreachable" in out:
        return Outcome("trap", "", out)
    return Outcome("error", _first(out), out)


# ─── the arithmetic backend oracle (hand-built Mir → select/serialize/frame → run) ─────────────
# These exercise cdzc's BACKEND directly from a hand-built Mir, independent of the decode front-end
# (which is gated on the seed's return-kind inference). Each builds a component with cdzc's own
# `wrap-component∘core-module-locals∘serialize-body∘select` and runs it via the seed. `mir` is a Mir
# expression (in cdzc scope); `want` is ('value', <str>) or ('trap', None).
_MIN = "-9223372036854775808"
_MAX = "9223372036854775807"
# Mir builders in the op-enum IR shape (post-IR-port): arithmetic is `(Mir.MArith (tuple <ArithOp> a b))`,
# bitwise `(Mir.MBit (tuple <BitOp> a b))`, shift `(Mir.MShift (tuple <ShiftOp> a b))`.
def _add(a, b): return f"(Mir.MArith (tuple (ArithOp.OpAdd ()) {a} {b}))"
def _sub(a, b): return f"(Mir.MArith (tuple (ArithOp.OpSub ()) {a} {b}))"
def _mul(a, b): return f"(Mir.MArith (tuple (ArithOp.OpMul ()) {a} {b}))"
def _bit(op, a, b): return f"(Mir.MBit (tuple (BitOp.{op} ()) {a} {b}))"
def _shl(a, b): return f"(Mir.MShift (tuple (ShiftOp.OpShl ()) {a} {b}))"
def _shr(a, b): return f"(Mir.MShift (tuple (ShiftOp.OpShr ()) {a} {b}))"
def _i(n): return f"(Mir.MInt {n})"

BACKEND_ORACLE = [
    (_i(42),                                    ("value", "42")),
    # checked arithmetic (value + overflow trap)
    (_add(_i(1), _i(2)),                        ("value", "3")),
    (_sub(_i(10), _i(3)),                       ("value", "7")),
    (_sub(_i(5), _i(-3)),                       ("value", "8")),
    (_sub(_add(_i(1), _i(2)), _i(1)),           ("value", "2")),
    (_mul(_i(2), _i(3)),                        ("value", "6")),
    (_mul(_i(-4), _i(5)),                       ("value", "-20")),
    (_mul(_i(0), _i(_MAX)),                     ("value", "0")),
    (_mul(_i(2), _add(_i(3), _i(4))),           ("value", "14")),
    (_mul(_i(3037000499), _i(3037000499)),      ("value", "9223372030926249001")),
    (_add(_i(_MAX), _i(1)),                     ("trap", None)),
    (_sub(_i(_MIN), _i(1)),                     ("trap", None)),
    (_mul(_i(_MAX), _i(2)),                     ("trap", None)),
    (_mul(_i(_MIN), _i(-1)),                    ("trap", None)),
    (_mul(_i(3037000500), _i(3037000500)),      ("trap", None)),
    # NEW: bitwise & | ^ / % (div/rem trap natively on /0 and MIN/-1)
    (_bit("OpAnd", _i(12), _i(10)),             ("value", "8")),    # 1100 & 1010 = 1000
    (_bit("OpOr", _i(12), _i(10)),              ("value", "14")),   # 1100 | 1010 = 1110
    (_bit("OpXor", _i(12), _i(10)),             ("value", "6")),    # 1100 ^ 1010 = 0110
    (_bit("OpDiv", _i(20), _i(6)),              ("value", "3")),    # trunc toward zero
    (_bit("OpDiv", _i(-20), _i(6)),             ("value", "-3")),
    (_bit("OpRem", _i(20), _i(6)),              ("value", "2")),
    (_bit("OpDiv", _i(5), _i(0)),               ("trap", None)),    # /0 traps
    (_bit("OpDiv", _i(_MIN), _i(-1)),           ("trap", None)),    # MIN/-1 overflow traps
    (_bit("OpRem", _i(5), _i(0)),               ("trap", None)),
    # NEW: guarded shifts << >> (count>=64 traps; left also traps on overflow)
    (_shl(_i(1), _i(4)),                        ("value", "16")),
    (_shr(_i(256), _i(4)),                      ("value", "16")),
    (_shr(_i(-256), _i(4)),                     ("value", "-16")),  # arithmetic (sign-extending)
    (_shl(_i(1), _i(64)),                       ("trap", None)),    # count >= 64 traps
    (_shl(_i(1), _i(63)),                       ("trap", None)),    # left-overflow (bit past sign) traps
    (_shl(_i(1), _i(62)),                       ("value", "4611686018427387904")),  # ok (no overflow)
]

def backend_build_run(mir_expr):
    """Build a component from a hand Mir with cdzc's backend, run it via the seed → its Outcome."""
    return probe(f"(wrap-component (core-module-locals "
                 f"(serialize-body (select {mir_expr} 0)) (mir-scratch-count {mir_expr})))")


def run_backend_oracle():
    """Run BACKEND_ORACLE; return (passes, [failures]). A 'bytes' outcome is the component — we then
       run it. To keep it single-pass here, we compare against the seed's own run of the component."""
    passes, fails = 0, []
    for mir, want in BACKEND_ORACLE:
        o = backend_build_run(mir)
        if o.kind != "bytes":
            fails.append((mir, want, o)); continue
        got = _run_component_bytes(o.payload)
        ok = (want[0] == "trap" and got.kind == "trap") or \
             (want[0] == "value" and got.kind == "value" and got.payload == want[1])
        if ok: passes += 1
        else: fails.append((mir, want, got))
    return passes, fails


def _run_component_bytes(b):
    with tempfile.NamedTemporaryFile('wb', suffix='.wasm', delete=False) as f:
        f.write(b); p = f.name
    try:
        return _run_component(p)
    finally:
        os.unlink(p)


def self_compiles():
    """Does cdzc.cdz compile on the seed through its real entry? cdzc is a LIBRARY (no entrypoint),
       so we inject a `main` that DRIVES THE PIPELINE — `(compile-bytes …)` on a tiny input — which
       makes the whole decode→resolve→lower→select→serialize→frame chain reachable and compiled.
       Success (kind='bytes', the emitted component) means the whole compiler type-checks and lowers.
       (NB: a trivial `(def (main) 0)` that references nothing instead trips a seed whole-module DCE
       quirk — function[8] invalid — unrelated to compiler correctness; drive the pipeline instead.)"""
    return compile("(module c (def (main) 42))")


# ─── CLI ─────────────────────────────────────────────────────────────────────────
def _main(argv):
    if not argv:
        print(__doc__); return 2
    cmd, rest = argv[0], argv[1:]
    if cmd == "self":
        o = self_compiles()
        ok = o.kind == "bytes"
        print("cdzc self-compiles" if ok else f"cdzc does NOT self-compile → {o!r}")
        return 0 if ok else 1
    if cmd == "astbytes":
        print(ast_bytes(rest[0]).hex()); return 0
    if cmd == "eval":
        print(repr(eval(rest[0]))); return 0
    if cmd == "probe":
        print(repr(probe(rest[0]))); return 0
    if cmd == "compile":
        print(repr(compile(rest[0]))); return 0
    if cmd == "run":
        print(repr(compile_run(rest[0]))); return 0
    if cmd == "oracle":
        passes, fails = run_backend_oracle()
        for mir, want, got in fails:
            print(f"  FAIL {mir[:56]:56s} want={want} got={got!r}")
        print(f"backend oracle: {passes}/{len(BACKEND_ORACLE)} pass, {len(fails)} FAIL")
        return 0 if not fails else 1
    print(f"unknown command: {cmd}\n{__doc__}"); return 2


if __name__ == "__main__":
    sys.exit(_main(sys.argv[1:]))
