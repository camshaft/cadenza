/-
`Oracle` — the Lean reference interpreter for Cadenza, used as an independent differential oracle
(design: `implementation/design/DESIGN-lean-differential-oracle.md`). This root module re-exports
the pieces of the library.
-/
import Oracle.Leb
import Oracle.Ast
import Oracle.Value
import Oracle.Eval
import Oracle.Wasm
import Oracle.WasmDiff
import Oracle.Wasm.Talos
import Oracle.Wasm.HeapHost
import Oracle.Wasm.HeapDecode
import Oracle.Symbolic
import Oracle.SymbolicSound
import Oracle.Check
import Oracle.Type
import Oracle.Frame
import Oracle.Handler
import Oracle.Batch
