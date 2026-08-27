/-
`cdz-oracle` — the oracle executable. Reads a request frame on stdin, answers each trial, and writes
the verdict response frame on stdout (mirroring `cdz run`'s stdout=result / stderr=diagnostic
convention: a malformed request is a diagnostic on stderr + a non-zero exit, never a bogus verdict).
The byte formats are `Oracle.Frame` (see `FRAME.md`).
-/
import Oracle

open Oracle Oracle.Frame

/-- Read every byte available on a stream. -/
partial def readAll (s : IO.FS.Stream) (acc : ByteArray) : IO ByteArray := do
  let chunk ← s.read 65536
  if chunk.isEmpty then
    return acc
  else
    readAll s (acc ++ chunk)

def main : IO UInt32 := do
  let stdin ← IO.getStdin
  let input ← readAll stdin ByteArray.empty
  match decodeRequest input with
  | .error e =>
    (← IO.getStderr).putStrLn s!"cdz-oracle: {e}"
    return 1
  | .ok req =>
    let resp := handle req
    (← IO.getStdout).write (encodeResponse resp)
    return 0
