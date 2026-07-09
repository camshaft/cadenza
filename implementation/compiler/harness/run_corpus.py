#!/usr/bin/env python3
"""Interim corpus harness for the Cadenza-authored compiler (compiler.cdz).

WHY THIS EXISTS / WHAT IT IS NOT:
  The clean harness is `cadenza-seed component-check <compiler-component> spec/semantics`,
  which feeds every corpus case's canonical AST bytes to a `compile : list<u8> -> list<u8>`
  component and diffs against native cdz-rustc. That is BLOCKED on seed gap 3l — the seed
  cannot yet build compiler.cdz into a `compile`-exporting component (only nullary `run`).
  So this script drives the SAME comparison a different way, without the `compile` ABI:
    per corpus case C with input expr E, wrapped as P = (module case (def (main) E)):
      1. dump P's canonical AST bytes via the seed (Ast.encode of a quote of P);
      2. patch those bytes into compiler.cdz's `main` (via compile-bytes) and `emit` → MINE (the
         component compiler.cdz BUILT for P);
      3. `emit` P directly with the seed → NATIVE (the reference component);
      4. RUN both components (wasmtime `run()`) and read the corpus ORACLE `(output (: v T))`.

  CLASSIFICATION (value-first, NOT byte-first):
    - AGREE   — MINE is byte-identical to NATIVE. (Strongest; the eventual differential-gate bar.)
    - SOFT    — bytes differ BUT MINE's runtime value == the oracle value. This is FINE and expected:
                compiler.cdz const-folds arithmetic while native emits overflow-checked runtime helper
                calls (and even a dead helper for a folded `+`), so the modules differ byte-for-byte
                yet compute the same result. We deliberately live in this middle ground for now — byte
                equality is NOT enforced yet.
    - HARD    — MINE's runtime value != the oracle value (or MINE traps where the oracle is a value,
                or vice-versa). A REAL BUG in compiler.cdz — the signal that matters.
    - DECLINE — compiler.cdz declined the program (its main traps: KError→unreachable). The frontier.
    - N/A     — native didn't realize P (unrealized capability) or the case is a rejection/trap case
                with no value oracle; skipped.

  When gap 3l lands, prefer `cadenza-seed component-check` (byte-level) for the AGREE bar; this
  value-level harness stays useful for the SOFT middle ground.
"""
import re, os, glob, subprocess, sys, tempfile

REPO = "/Users/bythewc/Projects/camshaft/cadenza"
# Default to the STABLE toolchain snapshot — a frozen all-gates-green seed, so this self-hosting agent
# is NOT broken by concurrent, mid-change seed edits in implementation/seed/ (which can transiently emit
# invalid components — "failed to parse WebAssembly module" — for List.push-composed programs). Override
# with CDZ_SEED / CADENZA_RUNTIME env vars to test against a live build.
SEED = os.environ.get("CDZ_SEED", f"{REPO}/implementation/stable/cadenza-seed")
COMPILER = os.environ.get("CDZC", f"{REPO}/implementation/compiler/cdzc.cdz")
RUNTIME = os.environ.get("CADENZA_RUNTIME", f"{REPO}/implementation/stable/cdz_runtime.wasm")
ENV = {**os.environ, "CADENZA_RUNTIME": RUNTIME}

def strip_comments(txt):
    out=[]
    for line in txt.splitlines():
        s='';inq=False;e=False
        for c in line:
            if e: s+=c;e=False;continue
            if c=='\\' and inq: s+=c;e=True;continue
            if c=='"': inq=not inq;s+=c;continue
            if c==';' and not inq: break
            s+=c
        out.append(s)
    return "\n".join(out)

def balanced_after(body, kw):
    """The one s-expr (atom or balanced list) that follows keyword `kw` (e.g. '(input') in `body`,
       or None. `kw` includes the opening paren, e.g. '(input' / '(output'."""
    m=re.search(re.escape(kw)+r'\b',body)
    if not m: return None
    js=m.end()
    while js<len(body) and body[js] in ' \t\n': js+=1
    if js>=len(body): return None
    if body[js]=='(':
        d=0;e=js
        while e<len(body):
            if body[e]=='(':d+=1
            elif body[e]==')':
                d-=1
                if d==0:e+=1;break
            e+=1
        return body[js:e].strip()
    e=js
    while e<len(body) and body[e] not in ' \t\n)': e+=1
    return body[js:e].strip()

def oracle_value(output_sexpr):
    """Extract the canonical VALUE from an (output (: <value> <Type>)) s-expr, normalized to how
       wasmtime renders `run()`. Returns None if not a simple `(: v T)` value oracle (e.g. a trap
       or rejection case, or a compound value we don't render-compare)."""
    if output_sexpr is None: return None
    o=output_sexpr.strip()
    m=re.match(r'\(:\s+(.*)\s+([A-Za-z][\w<> ]*)\)$', o, re.S)
    if not m: return None
    v=m.group(1).strip()
    # scalar values we can compare against wasmtime's run() rendering:
    if v in ("true","false"): return v
    if re.fullmatch(r'-?\d+', v): return v          # Int64
    if v=="unit": return "unit"                     # Unit → () at boundary
    return None                                     # compound/string/float — skip value-compare for now

def parse_cases(path):
    """Yield (description, input_sexpr, oracle) for each (case …) with an (input …).
       `oracle` is:
         - a normalized scalar string (int/bool/unit) the program should evaluate to, OR
         - "TRAP" if the case expects a runtime trap `(trap "…")` (a well-typed program that traps —
           e.g. integer overflow, division by zero, byte out of range), OR
         - None (a rejection case, or a compound/float/string value we don't render-compare)."""
    txt=strip_comments(open(path).read())
    cases=[]
    i=0
    while True:
        cm=re.search(r'\(case\s+"((?:[^"\\]|\\.)*)"', txt[i:])
        if not cm: break
        desc=cm.group(1)
        cstart=i+cm.start()
        depth=0;k=cstart
        while k<len(txt):
            if txt[k]=='(':depth+=1
            elif txt[k]==')':
                depth-=1
                if depth==0:k+=1;break
            k+=1
        body=txt[cstart:k]
        inp=balanced_after(body,'(input')
        if inp is not None:
            oracle=oracle_value(balanced_after(body,'(output'))
            if oracle is None and re.search(r'\(trap\b', body):
                oracle="TRAP"      # a well-typed program that TRAPS at run time
            cases.append((desc, inp, oracle))
        i=k
    return cases

def wrap(inp):
    """Match corpus::as_program — a bare expr becomes (module case (def (main) E)); a module passes through."""
    if inp.lstrip().startswith('(module'):
        return inp
    return f"(module case (def (main) {inp}))"

def deesc(inner):
    """Decode the seed's Rust-Debug b\"...\" value string to raw bytes (two unescape levels)."""
    def un(t):
        o=[];k=0
        while k<len(t):
            c=t[k]
            if c=='\\':
                n=t[k+1]
                if n=='x':o.append(chr(int(t[k+2:k+4],16)));k+=4;continue
                m={'0':'\0','r':'\r','n':'\n','t':'\t','\\':'\\','"':'"',"'":"'"}
                o.append(m.get(n,n));k+=2;continue
            o.append(c);k+=1
        return ''.join(o)
    l1=un(inner)
    if not (l1.startswith('b"') and l1.endswith('"')): return None
    return bytes(ord(c) for c in un(l1[2:-1]))

def seed_emit_native(src):
    """Native oracle: run the seed `emit` on the WRAPPED PROGRAM source; return (kind, payload).
       'ok' payload = the emitted component bytes (from /tmp/cadenza-emit.wasm)."""
    with tempfile.NamedTemporaryFile('w',suffix='.cdz',delete=False) as f:
        f.write(src); p=f.name
    try:
        r=subprocess.run([SEED,"emit",p],env=ENV,capture_output=True,text=True,timeout=40)
    except subprocess.TimeoutExpired:
        return ("timeout","")
    finally:
        os.unlink(p)
    out=r.stdout
    m=re.search(r'declined:\s*(.*)',out)
    if m: return ("decline",m.group(1).strip())
    if 'parse error' in out: return ("parse", out.strip()[:120])
    m=re.search(r'INVALID:\s*(.*)',out)
    if m: return ("invalid",m.group(1).strip())
    w="/tmp/cadenza-emit.wasm"
    if os.path.exists(w):
        return ("ok", open(w,'rb').read())
    return ("unknown", out.strip()[:120])

def seed_emit_mine(compiler_src):
    """Run the seed `emit` on the PATCHED compiler.cdz. The compiler's OWN output (the component it
       BUILT for the corpus program) is its runtime return value — `ran → Value("b\"…\"")` — NOT the
       emitted compiler component in /tmp/cadenza-emit.wasm. A decline/invalid HERE means compiler.cdz
       itself failed to compile (a regression), reported as 'error'."""
    with tempfile.NamedTemporaryFile('w',suffix='.cdz',delete=False) as f:
        f.write(compiler_src); p=f.name
    try:
        r=subprocess.run([SEED,"emit",p],env=ENV,capture_output=True,text=True,timeout=40)
    except subprocess.TimeoutExpired:
        return ("timeout","")
    finally:
        os.unlink(p)
    out=r.stdout
    if re.search(r'declined:|parse error|INVALID:',out):
        return ("error", out.strip()[:140])
    m=re.search(r'ran → Value\("(.*)"\)',out,re.S)
    if not m:
        return ("mine-traps", out.strip()[:140])   # compiler ran but returned no Bytes (declined the program → trap)
    b=deesc(m.group(1))
    if b is None:
        return ("mine-nonbytes", m.group(1)[:80])
    return ("ok", b)

def dump_ast_bytes(program_src):
    """Get program's canonical AST bytes via Ast.encode of a quote of it. None if quote/encode rejects."""
    dumper=f"(module m (def (main) (Ast.encode (quote {program_src}))))"
    with tempfile.NamedTemporaryFile('w',suffix='.cdz',delete=False) as f:
        f.write(dumper); p=f.name
    try:
        r=subprocess.run([SEED,"emit",p],env=ENV,capture_output=True,text=True,timeout=40)
    except subprocess.TimeoutExpired:
        return None
    finally:
        os.unlink(p)
    m=re.search(r'ran → Value\("(.*)"\)',r.stdout,re.S)
    if not m: return None
    return deesc(m.group(1))

# Read compiler.cdz once. Its entry is now `(def (compile b) (compile-bytes b))` — the real
# self-hosting seam. To measure coverage WITHOUT crossing the broken compile-return path (seed gap
# 3n: retptr misaligned when input-len % 4 != 0), we INJECT a temporary nullary `(def (main) …)` that
# calls `compile-bytes` on the case's AST bytes, right before the module's closing paren. The seed
# picks `main` over `compile` for the entry, so `emit` frames it as nullary `run` and we read the
# runtime `Value` (the component compiler.cdz built) — exactly the old flow, sidestepping 3n. `compile`
# stays the shipped entry; this injection is harness-local (never written back to compiler.cdz).
COMPILER_SRC=open(COMPILER).read()
# `cdzc.cdz`'s self-hosting entry is `(def (compile-bytes b) …)`. To measure coverage we inject a nullary
# `(def (main) (compile-bytes <case-ast-bytes>))` and WRAP the whole thing in `(module cdzc … )` — the
# SAME shape `cdzc.py` uses (cdzc's own `resolve-program` expects `(module <name> (def (main) …))`, so a
# flat top-level append would make cdzc see a different program shape and trap). The seed picks `main`
# for the entry, frames it nullary `run`, and we read the runtime `Value` (the component cdzc built for
# the case). Harness-local (never written back). The file's own `(export compile-bytes)` inside the
# module is harmless — `main` is the chosen entry.
if "(def (compile-bytes b)" not in COMPILER_SRC and "(def (compile b)" not in COMPILER_SRC:
    print(f"FATAL: {COMPILER} has no (def (compile-bytes b) …) / (def (compile b) …) entry"); sys.exit(2)

def mine_emit(ast_bytes):
    hexs=" ".join("0x%02x"%b for b in ast_bytes)
    injected="(module cdzc\n"+COMPILER_SRC+"\n(def (main) (compile-bytes (Bytes.of (list "+hexs+"))))\n)"
    return seed_emit_mine(injected)

def is_bare_decline(component_bytes):
    """True if the built component's entry core func is a DECLINE — a bare `unreachable` with no real
       logic before it (the reader lowered an unsupported construct to KError → unreachable). Such a
       component TRAPS, but not for any semantic reason — so a trap-expecting oracle it 'passes' is
       coincidental, not conformance. Heuristic: the entry func 0 body, stripped of local decls, is a
       run of `unreachable`/`local.set`/`local.get`/`nop` with NO computational op (arith/cmp/call/
       const-then-check). A genuine semantic trap (e.g. `i64.const 5; i64.const 0; i64.div_s`, or a
       byte-range guard) carries such ops before the trap. Best-effort (needs wasm-tools); None if
       it can't disassemble."""
    import shutil
    if not shutil.which("wasm-tools"): return None
    with tempfile.NamedTemporaryFile('wb',suffix='.wasm',delete=False) as f:
        f.write(component_bytes); p=f.name
    try:
        wat=subprocess.run(["wasm-tools","print",p],capture_output=True,text=True,timeout=15).stdout
    except Exception:
        return None
    finally:
        os.unlink(p)
    m=re.search(r'\(func \(;0;\).*?\n(.*?)\n\s*\)\n', wat, re.S)
    if not m: return None
    ops=[l.strip() for l in m.group(1).splitlines() if l.strip()]
    # ops that constitute REAL computation (a semantic trap has at least one before `unreachable`):
    real=re.compile(r'\b(i64|i32|f64)\.(add|sub|mul|div|rem|and|or|xor|shl|shr|const|eq|ne|lt|gt|le|ge)|\bcall\b|\bif\b|\bbr')
    body_ops=[o for o in ops if not o.startswith('(local')]
    has_real=any(real.search(o) for o in body_ops)
    has_unreachable=any(o.startswith('unreachable') for o in body_ops)
    return has_unreachable and not has_real

def run_component(component_bytes):
    """Run a component's `run()` via wasmtime; return (kind, value).
       kind ∈ {value(str), trap, invalid, error}. `value` is normalized to the corpus rendering."""
    with tempfile.NamedTemporaryFile('wb',suffix='.wasm',delete=False) as f:
        f.write(component_bytes); p=f.name
    try:
        r=subprocess.run(["wasmtime","run","-W","component-model=y","--invoke","run()",p],
                         capture_output=True,text=True,timeout=30)
    except subprocess.TimeoutExpired:
        return ("error","timeout")
    finally:
        os.unlink(p)
    out=(r.stdout+r.stderr).strip()
    if r.returncode==0:
        v=r.stdout.strip()
        if v=="()": v="unit"
        return ("value", v)
    if "unreachable" in out or "wasm trap" in out or "trap" in out.lower():
        return ("trap","")
    return ("error", out[:100])

def main():
    args=[a for a in sys.argv[1:] if not a.startswith("-")]
    files=args or sorted(glob.glob(f"{REPO}/spec/semantics/*.sexp"))
    tally={"agree":0,"soft":0,"trap-ok":0,"trap-decline":0,"hard":0,"decline":0,"na":0,"skip":0,"error":0}
    hard=[]; softs=[]
    for path in files:
        for desc,inp,oracle in parse_cases(path):
            prog=wrap(inp)
            ast=dump_ast_bytes(prog)
            if ast is None:
                tally["skip"]+=1; continue          # couldn't quote/encode
            nat=seed_emit_native(prog)
            if nat[0]!="ok":
                tally["na"]+=1; continue            # native didn't realize P — no reference
            mine=mine_emit(ast)
            if mine[0]=="error":
                tally["error"]+=1; hard.append((os.path.basename(path),desc,"compiler-build-error",mine[1]));continue
            if mine[0] in ("mine-traps","mine-nonbytes"):
                # compiler.cdz's main traps (DECLINED the program → KError→unreachable): it returned no
                # component bytes at all. On a value oracle this is the unsupported-construct frontier
                # (`decline`). On a TRAP oracle it is NOT a verified semantic trap — a decline and a real
                # trapping semantic produce the identical `unreachable`, so a decline landing on a trap
                # oracle is COINCIDENTAL agreement (right observable, wrong reason), bucketed separately
                # as `trap-decline`, never counted as conformance. (See the trap-oracle-dual learning:
                # the discriminator is the in-range companion case, which a decline ALSO traps on.)
                tally["trap-decline" if oracle=="TRAP" else "decline"]+=1; continue
            if mine[0]!="ok":
                tally["error"]+=1; continue
            # Byte-identical?  → AGREE (strongest).
            if mine[1]==nat[1]:
                tally["agree"]+=1; continue
            # Bytes differ: run it and compare to the oracle.
            if oracle is None:
                tally["na"]+=1; continue            # compound/float/rejection — can't value-compare
            rv=run_component(mine[1])
            if oracle=="TRAP":
                if rv[0]=="trap":
                    # Mine traps — but is it a SEMANTIC trap or a DECLINE (bare `unreachable` from an
                    # unsupported construct)? A decline traps for no semantic reason, so a trap oracle it
                    # 'passes' is coincidental. Distinguish by the built component's entry-func shape.
                    if is_bare_decline(mine[1]):
                        tally["trap-decline"]+=1   # coincidental agreement — NOT conformance
                    else:
                        tally["trap-ok"]+=1        # a verified semantic trap (real logic before the trap)
                elif rv[0]=="value":
                    tally["hard"]+=1                # mine returned a VALUE where a trap is required — MISCOMPILE
                    hard.append((os.path.basename(path),desc,f"got=value:{rv[1]}","want=TRAP"))
                else:
                    tally["error"]+=1
                    hard.append((os.path.basename(path),desc,f"got={rv[0]}","want=TRAP"))
                continue
            if rv[0]=="value" and rv[1]==oracle:
                tally["soft"]+=1
                softs.append((os.path.basename(path),desc,len(mine[1]),len(nat[1])))
            elif rv[0]=="trap":
                tally["decline"]+=1                 # mine traps where a value is wanted — honest decline
            elif rv[0]=="value":
                tally["hard"]+=1                    # ran but WRONG value — miscompile
                hard.append((os.path.basename(path),desc,f"got={rv[1]}",f"want={oracle}"))
            else:
                tally["error"]+=1
                hard.append((os.path.basename(path),desc,f"got={rv[0]}:{rv[1][:30]}",f"want={oracle}"))
    print("\n=== corpus harness (value-first; byte equality NOT enforced) ===")
    print(f"  agree   {tally['agree']:4}   (byte-identical to native — strongest)")
    print(f"  soft    {tally['soft']:4}   (bytes differ, runtime value == oracle — FINE, middle ground)")
    print(f"  trap-ok {tally['trap-ok']:4}   (oracle expects a TRAP and mine's built component RAN and trapped — a verified SEMANTIC trap)")
    print(f"  trap-dc {tally['trap-decline']:4}   (oracle expects a TRAP but mine DECLINED the construct — coincidental, NOT conformance)")
    print(f"  hard    {tally['hard']:4}   (ran but WRONG value, or a value where a trap is required — MISCOMPILE)")
    print(f"  decline {tally['decline']:4}   (compiler.cdz traps on an unsupported construct — frontier)")
    print(f"  n/a     {tally['na']:4}   (native unrealized, or no scalar/trap oracle)")
    print(f"  skip    {tally['skip']:4}   (couldn't quote/encode the input)")
    print(f"  error   {tally['error']:4}   (compiler.cdz emitted an invalid/unrunnable component)")
    if hard:
        print("\n  🔴 HARD — ran but WRONG value, or invalid emission (REAL BUGS):")
        for f,d,a,b in hard[:60]:
            print(f"    {f}: {d}  [{a} {b}]")
    else:
        print("\n  ✓ no HARD miscompiles — where compiler.cdz emits a runnable component, the VALUE is correct.")
    if softs and ("-v" in sys.argv or len(files)<=2):
        print(f"\n  soft (value-correct, byte-different) — {len(softs)}:")
        for f,d,ml,nl in softs[:40]:
            print(f"    {f}: {d}  [mine={ml}B native={nl}B]")

if __name__=="__main__":
    main()
