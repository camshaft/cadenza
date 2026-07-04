; String operations — witnesses collections-and-text.md. The compiler needs string operations
; for error messages, name encoding (export names → bytes in wasm), and instruction tag dispatch.
; String equality is already witnessed in 01-literals; these cover the remaining operations
; the compiler needs.

(case "string concatenation"
  (doc    "The compiler builds error messages and export names via string concatenation.")
  (input  (String.concat "hello" " world"))
  (output (: "hello world" String)))

(case "string length"
  (input  (String.len "hello"))
  (output (: 5 Int64)))

(case "string to bytes (UTF-8)"
  (doc    "The compiler encodes export names as UTF-8 bytes for wasm sections. String.to-bytes
           produces the UTF-8 byte sequence of the string.")
  (input  (Bytes.len (String.to-bytes "run")))
  (output (: 3 Int64)))

(case "string to bytes encodes multi-byte characters"
  (doc    "UTF-8 encodes non-ASCII characters as multiple bytes.")
  (input  (Bytes.len (String.to-bytes "café")))
  (output (: 5 Int64)))

(case "string equality"
  (input  (= "hello" "hello"))
  (output (: true Bool)))

(case "string inequality"
  (input  (= "hello" "world"))
  (output (: false Bool)))

(case "string indexing returns a character"
  (input  (String.at "hello" 1))
  (output (: "e" String)))

(case "string slicing"
  (input  (String.slice "hello world" 0 5))
  (output (: "hello" String)))
