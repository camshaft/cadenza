; Compiler primitives for userspace contract-building (design/DESIGN-compiler-primitives.md).
; The compiler exposes three CONTRACT-AGNOSTIC primitives a userspace Cadenza program composes into a
; contract-id, with the compiler never modelling "contract": (1) IMPORT REFLECTION, (2) CONST EXECUTION,
; (3) blake3 HASHING. This file pins the primitives themselves — never a contract shape (that lives in a
; userspace .cdz library). Landed incrementally; a generation that does not realize a primitive declines
; its cases.
;
; --- Primitive 3: blake3 hashing (`Blake3.of : Bytes -> Bytes`) -------------------------------------
; `Blake3.of` is a plain, unkeyed BLAKE3-256 content hash — 32 raw bytes, NO key / derive-key / domain
; tag (all domain separation is userspace's job, design D7). It NAMES THE ALGORITHM (design D5): a future
; digest would be a DIFFERENT named function, never a silent change to a generic `Hash`. Over a
; compile-time-visible `Bytes` it FOLDS to a compile-time constant via `blake3::hash`; that constant is
; BYTE-IDENTICAL to the runtime `hash-blake3` heap op (design §9 — same crate, same bytes, both places).

(case "Blake3.of of the empty byte string is the official empty-input BLAKE3-256 digest"
  (doc    "The load-bearing byte-identity pin: `Blake3.of b\"\"` is the OFFICIAL BLAKE3 empty-input
           vector `af1349b9...e41f3262` (32 bytes). Because the compile-time fold and the runtime op both
           call the one `blake3` crate over the same bytes, this same digest is what a runtime hash of the
           same input produces (design-compiler-primitives.md §9). Executes byte-identical — the value is
           materialized and compared, not merely folded.")
  (input  (Blake3.of b""))
  (output (: b"\xaf\x13\x49\xb9\xf5\xf9\xa1\xa6\xa0\x40\x4d\xea\x36\xdc\xc9\x49\x9b\xcb\x25\xc9\xad\xc1\x12\xb7\xcc\x9a\x93\xca\xe4\x1f\x32\x62" Bytes)))

(case "Blake3.of is a 32-byte digest"
  (doc    "A BLAKE3-256 digest is always 32 bytes, whatever the input length. Pins the output width so a
           downstream `Bytes.eq` against a same-width contract-id is well-formed.")
  (input  (Bytes.len (Blake3.of b"the quick brown fox")))
  (output (: 32 Int64)))

(case "Blake3.of is deterministic — the same bytes hash to the same digest"
  (doc    "Hashing the SAME input twice yields the SAME digest — the property that makes a hash usable as
           a content-address / identity. `Bytes.eq` over the two digests is true.")
  (input  (= (Blake3.of b"abc") (Blake3.of b"abc")))
  (output (: true Bool)))

(case "Blake3.of is sensitive — a one-bit-different input hashes to a different digest"
  (doc    "Two inputs differing by a single byte hash to DIFFERENT digests, so distinct declarations get
           distinct ids (exact-hash routing depends on this). `Bytes.eq` over the two digests is false.")
  (input  (= (Blake3.of b"abc") (Blake3.of b"abd")))
  (output (: false Bool)))

(case "Blake3.of composes with Ast.encode — hashing an encoded AST folds to a 32-byte digest"
  (doc    "The shape of the contract-id path (design §4), with NO contract knowledge in the compiler:
           `Ast.encode` folds a compile-time AST value to its canonical bytes (a compile-time `ConstBytes`),
           and `Blake3.of` folds those bytes to a 32-byte digest at compile time. Here the whole
           `Blake3.of (Ast.encode …)` const-folds; its length is 32. A userspace transform would sit
           between the two, canonicalizing the declaration — but the primitives compose exactly like this.")
  (input  (Bytes.len (Blake3.of (Ast.encode (Ast.Int 7)))))
  (output (: 32 Int64)))
