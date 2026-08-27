/-
The `cdz-oracle` wire boundary — the request the oracle reads on stdin and the verdict response it
writes on stdout. This realizes OQ-A of `implementation/design/DESIGN-lean-differential-oracle.md`
with the doc's chosen default: a length-prefixed frame carrying nothing but the two frozen byte
formats. Modules and values cross as raw `ast-encoding.md` / `deterministic-value-form.md` bytes;
this module owns only the thin LEB128 envelope around them (counts and blob lengths), never their
interpretation. See `FRAME.md` for the byte layout.

L0.1 models the envelope end-to-end but interprets neither modules nor values — every trial yields
`Outcome.unsupported`. The decoders here are total (`Except String`); the semantics arrive in later
increments (L0.2 decodes the module bytes, L1.1 evaluates).
-/
import Oracle.Leb

namespace Oracle.Frame

open Oracle.Leb

/-- One evaluation trial against the (shared) reduced program: an entry symbol, its runtime
argument values, and the fixed host responses fed in call order. Args/host-responses are raw
`deterministic-value-form.md` bytes at this layer. -/
structure Trial where
  entry : String
  args : Array ByteArray
  hostResponses : Array ByteArray
  deriving Inhabited

/-- A full oracle request: the modules (raw `ast-encoding.md` bytes) plus the trials to run. -/
structure Request where
  modules : Array ByteArray
  trials : Array Trial
  deriving Inhabited

/-- The verdict algebra (design §1.1). `unsupported`/`diverges` are coverage-gaps the harness
skips, never a differential mismatch. -/
inductive Outcome where
  | value (bytes : ByteArray)
  | trap (kind : String)
  | error (code : String)
  | diverges
  | unsupported (reason : String)
  deriving Inhabited

/-- The per-trial verdict: an outcome plus the ordered host calls the trial made. -/
structure Verdict where
  outcome : Outcome
  hostCalls : Array ByteArray := #[]
  deriving Inhabited

/-- The response is one verdict per trial, in order. -/
abbrev Response := Array Verdict

/-- Outcome tag bytes on the wire. -/
def tagValue : UInt8 := 0
def tagTrap : UInt8 := 1
def tagError : UInt8 := 2
def tagDiverges : UInt8 := 3
def tagUnsupported : UInt8 := 4

/-! ### Encoding -/

/-- Append a LEB128-length-prefixed blob. -/
def pushBlob (acc : ByteArray) (b : ByteArray) : ByteArray :=
  (encodeInto acc b.size) ++ b

/-- Append a LEB128-length-prefixed UTF-8 string. -/
def pushStr (acc : ByteArray) (s : String) : ByteArray :=
  pushBlob acc s.toUTF8

/-- Append a LEB128-count-prefixed array of blobs. -/
def pushBlobs (acc : ByteArray) (bs : Array ByteArray) : ByteArray :=
  bs.foldl pushBlob (encodeInto acc bs.size)

/-- Encode a request frame (used by the self-test and, later, by tooling that drives the oracle). -/
def encodeRequest (r : Request) : ByteArray := Id.run do
  let mut acc := encodeInto ByteArray.empty r.modules.size
  for m in r.modules do
    acc := pushBlob acc m
  acc := encodeInto acc r.trials.size
  for t in r.trials do
    acc := pushStr acc t.entry
    acc := pushBlobs acc t.args
    acc := pushBlobs acc t.hostResponses
  return acc

/-- Encode one verdict. -/
def encodeVerdict (acc : ByteArray) (v : Verdict) : ByteArray :=
  let acc := match v.outcome with
    | .value bytes => pushBlob (acc.push tagValue) bytes
    | .trap kind => pushStr (acc.push tagTrap) kind
    | .error code => pushStr (acc.push tagError) code
    | .diverges => acc.push tagDiverges
    | .unsupported reason => pushStr (acc.push tagUnsupported) reason
  pushBlobs acc v.hostCalls

/-- Encode the response frame the oracle writes on stdout. -/
def encodeResponse (r : Response) : ByteArray :=
  r.foldl encodeVerdict (encodeInto ByteArray.empty r.size)

/-! ### Decoding (total, `Except String`) -/

/-- Read `n` elements via `f`, structurally recursive on `n`. -/
def readN (f : Cursor → Except String (α × Cursor)) :
    Cursor → Nat → Array α → Except String (Array α × Cursor)
  | c, 0, acc => .ok (acc, c)
  | c, (n + 1), acc => do
    let (x, c) ← f c
    readN f c n (acc.push x)

/-- Decode a LEB128-count-prefixed array via a per-element decoder. -/
def readArray (c : Cursor) (f : Cursor → Except String (α × Cursor)) :
    Except String (Array α × Cursor) := do
  let (n, c) ← c.readUleb
  readN f c n #[]

/-- Decode a length-prefixed UTF-8 string. -/
def readStr (c : Cursor) : Except String (String × Cursor) := do
  let (b, c) ← c.readLenPrefixed
  match String.fromUTF8? b with
  | some s => .ok (s, c)
  | none => .error "frame: invalid UTF-8 in string field"

/-- Decode one trial. -/
def readTrial (c : Cursor) : Except String (Trial × Cursor) := do
  let (entry, c) ← readStr c
  let (args, c) ← readArray c Cursor.readLenPrefixed
  let (hostResponses, c) ← readArray c Cursor.readLenPrefixed
  .ok ({ entry, args, hostResponses }, c)

/-- Decode a full request frame; rejects trailing bytes. -/
def decodeRequest (bytes : ByteArray) : Except String Request := do
  let c := Cursor.ofBytes bytes
  let (modules, c) ← readArray c Cursor.readLenPrefixed
  let (trials, c) ← readArray c readTrial
  if c.atEnd then
    .ok { modules, trials }
  else
    .error s!"frame: {c.remaining} trailing byte(s) after request"

/-- Decode one verdict. -/
def readVerdict (c : Cursor) : Except String (Verdict × Cursor) := do
  let (tag, c) ← c.readByte
  let (outcome, c) ←
    if tag == tagValue then
      let (b, c) ← c.readLenPrefixed; pure (Outcome.value b, c)
    else if tag == tagTrap then
      let (s, c) ← readStr c; pure (Outcome.trap s, c)
    else if tag == tagError then
      let (s, c) ← readStr c; pure (Outcome.error s, c)
    else if tag == tagDiverges then
      pure (Outcome.diverges, c)
    else if tag == tagUnsupported then
      let (s, c) ← readStr c; pure (Outcome.unsupported s, c)
    else
      .error s!"frame: unknown outcome tag {tag.toNat}"
  let (hostCalls, c) ← readArray c Cursor.readLenPrefixed
  .ok ({ outcome, hostCalls }, c)

/-- Decode a full response frame; rejects trailing bytes. -/
def decodeResponse (bytes : ByteArray) : Except String Response := do
  let c := Cursor.ofBytes bytes
  let (verdicts, c) ← readArray c readVerdict
  if c.atEnd then
    .ok verdicts
  else
    .error s!"frame: {c.remaining} trailing byte(s) after response"

end Oracle.Frame
