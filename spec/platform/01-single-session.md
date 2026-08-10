# 01-single-session

Platform-conformance suite — I1: a single Cadenza reducer session, one kick-off event, no effects. The runtime/platform analog of the compiler corpus (DESIGN-platform-conformance-suite.md, seq359): a (platform-case ..) declares reducer SESSIONS + exactly ONE (kickoff ..); the gate compiles each reducer to the cadenza:agent-kernel/fold world, drives it through the REAL kernel via cdz-session-run, and grades the observed end-state. I1 is the single-session, no-effect, drive-to-quiescence proof.

### a counter session bumps kv count on its kick-off message

```platform-case
```

One session, no effects. The kick-off is an inbound \`message\`; the reducer reads kv\[count\] (absent -\> 0), writes back byte (count+1) via its bound cadenza:agent-kernel/kv, and returns no effects. Folding one message on an empty session leaves kv\[count\] = 1, the session quiescent, and 2 events on the log (genesis seq0 + the one inbound).

```cdz session worker
type EffectKind =
  | Shell()
  | Http()
  | Model()
  | Now()
  | Timer()
  | Emit()

type EffectRequest =
  | Mk(Record(
    kind : EffectKind,
    target : String,
    payload : Option(Bytes),
    correlation : Option(Bytes)
  ))

effect Kv =
  | get : Bytes -> Option(Bytes)
  | put : `->`(Bytes, Bytes, Unit)

bind(Kv, "cadenza:agent-kernel/kv")

def bump-count(prev: Option(Bytes)) -> Bytes =
  let prev-byte = match prev with
      | Some(b) => (match Bytes.at(b, 0) with
        | Some(v) => v
        | None() => 0
      )
      | None() => 0 in
  Bytes.of([UInt8.wrap(prev-byte + 1)])

def apply(ct: Record(family : String, version : UInt(32)), payload: Option(Bytes), resumes: Option(
  Bytes
)) -> List(EffectRequest) = match resumes with
  | Some(_) => []
  | None() => if ct.family == "message" then
    host Kv
    in
    (
      Kv.put(String.to-bytes("count"), bump-count(Kv.get(String.to-bytes("count"))));
      []
    )
  else
    []

export { apply }
```

```kickoff
worker message unit : Unit
```

```end-status worker
quiescent
```

```end-kv worker
count 1 : Int64
```

```events-processed
worker 2
```

### the same counter IGNORES a non-message kick-off (else-branch writes no state)

```platform-case
```

The negative companion of the counter case: the SAME reducer, but the kick-off family is \`tick\`, which its \`apply\` does not match — so the else-branch returns no effects and writes NO kv. This pins that a no-op fold leaves the session quiescent with an EMPTY kv (no spurious \`count\` key) and still 2 events on the log (genesis + the one inbound). It witnesses the else-branch of the fold, and that the grader's kv assertions are a POSITIVE check (a case that asserts no kv key does not require one to exist).

```cdz session worker
type EffectKind =
  | Shell()
  | Http()
  | Model()
  | Now()
  | Timer()
  | Emit()

type EffectRequest =
  | Mk(Record(
    kind : EffectKind,
    target : String,
    payload : Option(Bytes),
    correlation : Option(Bytes)
  ))

effect Kv =
  | get : Bytes -> Option(Bytes)
  | put : `->`(Bytes, Bytes, Unit)

bind(Kv, "cadenza:agent-kernel/kv")

def bump-count(prev: Option(Bytes)) -> Bytes =
  let prev-byte = match prev with
      | Some(b) => (match Bytes.at(b, 0) with
        | Some(v) => v
        | None() => 0
      )
      | None() => 0 in
  Bytes.of([UInt8.wrap(prev-byte + 1)])

def apply(ct: Record(family : String, version : UInt(32)), payload: Option(Bytes), resumes: Option(
  Bytes
)) -> List(EffectRequest) = match resumes with
  | Some(_) => []
  | None() => if ct.family == "message" then
    host Kv
    in
    (
      Kv.put(String.to-bytes("count"), bump-count(Kv.get(String.to-bytes("count"))));
      []
    )
  else
    []

export { apply }
```

```kickoff
worker tick unit : Unit
```

```end-status worker
quiescent
```

```events-processed
worker 2
```

### a session writes TWO kv keys on its kick-off (multi-key end-state)

```platform-case
```

Exercises the grader's MULTI-KEY end-state path: on a \`message\` kick-off the reducer writes two distinct kv keys (a=7, b=9) via its bound kv, no effects. Pins that BOTH keys are asserted independently (a case with several (kv …) clauses requires every one to match), not just the first. Distinct one-byte values (07/09) also witness the value decoder at more than one number.

```cdz session worker
type EffectKind =
  | Shell()
  | Http()
  | Model()
  | Now()
  | Timer()
  | Emit()

type EffectRequest =
  | Mk(Record(
    kind : EffectKind,
    target : String,
    payload : Option(Bytes),
    correlation : Option(Bytes)
  ))

effect Kv =
  | get : Bytes -> Option(Bytes)
  | put : `->`(Bytes, Bytes, Unit)

bind(Kv, "cadenza:agent-kernel/kv")

def apply(ct: Record(family : String, version : UInt(32)), payload: Option(Bytes), resumes: Option(
  Bytes
)) -> List(EffectRequest) = match resumes with
  | Some(_) => []
  | None() => if ct.family == "message" then
    host Kv
    in
    (
      Kv.put(String.to-bytes("a"), Bytes.of([UInt8.wrap(7)]));
      Kv.put(String.to-bytes("b"), Bytes.of([UInt8.wrap(9)]));
      []
    )
  else
    []

export { apply }
```

```kickoff
worker message unit : Unit
```

```end-status worker
quiescent
```

```end-kv worker
a 7 : Int64
b 9 : Int64
```

```events-processed
worker 2
```

### a session stores a fixed mid-range byte value (decoder past the counter's 1)

```platform-case
```

A reducer that writes a FIXED byte 42 under \`answer\` on its message kick-off (not a bump, not derived) — witnessing the value decoder at a mid-range number (0x2a) and a fresh key name, so the grader's (: n Int64) → one-byte hex path is pinned beyond the count=1 the other cases use. Guards that an arbitrary stored byte round-trips through the end-kv comparison, not just the value 1.

```cdz session worker
type EffectKind =
  | Shell()
  | Http()
  | Model()
  | Now()
  | Timer()
  | Emit()

type EffectRequest =
  | Mk(Record(
    kind : EffectKind,
    target : String,
    payload : Option(Bytes),
    correlation : Option(Bytes)
  ))

effect Kv =
  | get : Bytes -> Option(Bytes)
  | put : `->`(Bytes, Bytes, Unit)

bind(Kv, "cadenza:agent-kernel/kv")

def apply(ct: Record(family : String, version : UInt(32)), payload: Option(Bytes), resumes: Option(
  Bytes
)) -> List(EffectRequest) = match resumes with
  | Some(_) => []
  | None() => if ct.family == "message" then
    host Kv
    in
    (
      Kv.put(String.to-bytes("answer"), Bytes.of([UInt8.wrap(42)]));
      []
    )
  else
    []

export { apply }
```

```kickoff
worker message unit : Unit
```

```end-status worker
quiescent
```

```end-kv worker
answer 42 : Int64
```

```events-processed
worker 2
```
