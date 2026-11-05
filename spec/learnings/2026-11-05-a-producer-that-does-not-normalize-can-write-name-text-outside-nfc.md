# A producer that doesn't normalize text can write Name text outside NFC

*2026-11-05*

**What happened.** The binary format requires a `Name` leaf's text to be NFC-normalized before leaves
are deduplicated ([ast-binary-format.md](../contracts/ast-binary-format.md), Equal Trees Encode To
Identical Bytes). `cadenza-ast`'s `Builder` meets that in `Builder::leaf_name`, but only under its
`std` feature: without `std` it interns the text as given, and decode builds `Leaf::Name` directly from
the bytes (`cadenza-ast/src/ast.rs`, `Builder::leaf_name`). A read of the producers that build
`cadenza-ast` without `std`, at main 7e6fdafdb48277782eb824de185dac49258a3cd8, found:

- `cdz-runtime` takes `cadenza-ast` with `default-features = false` and never turns `std` on. Its
  `ast-encode` heap op (93; `guest.rs`, `op_ast_encode` and `encode_ast_to_arenas`) writes
  `Leaf::Name(op_str_get(payload))`, the text of the runtime `String` that an `Ast.Name` holds. A
  runtime `String` isn't normalized when it is built: `str-nfc-normalize` (heap op 89,
  `bytes_string.rs`, `op_str_nfc`) normalizes one only when a program calls it. So a program that
  builds an `Ast.Name` from decomposed text, such as `cafe` followed by U+0301, and encodes it gets
  that text in the bytes as it is.
- `cdz-runtime`'s value-form encode (heap op 62; `value_codec.rs`, `DocBuilder::name_leaf`) takes each
  `Name` from the shape descriptor the compiler bakes (type, variant and field names) or from ASCII
  literals such as `unit` and `tuple`, so its text is in NFC whenever the program's identifiers are.
- `rcdzc` takes `cadenza-ast` with `default-features = false` and `features = ["canon"]`. Its
  compile-time `Ast.encode` fold (`lower/ast_reflect.rs`, `encode_ast_value`) writes `Leaf::Name(s)` for
  a constant string `s` (`Core::ConstStr`), through a `Builder` that normalizes only in a build where
  another crate turns on `cadenza-ast`'s `std` feature. The reader normalizes a string literal, but a
  string the compiler computes at compile time can fall outside NFC: `String.from-bytes` of a constant
  `Bytes.of` folds to a constant string through `std::str::from_utf8`, which checks that the bytes are
  well-formed UTF-8 and doesn't compose them (`lower/runtime_ops.rs`, `lower_str_from_bytes`, which the
  `Prim::StrFromBytes` arm of `lower/compute.rs` calls). The comment of the `Prim::SymbolOf` arm of
  `lower/compute.rs` says the same: "`String.from-bytes` of NFD bytes folds to a `ConstStr` WITHOUT
  normalizing", and "rcdzc's core excludes `unicode-normalization`, so the compiler cannot fold the NFC
  here". So an `Ast.Name` whose text is such a constant, such as the bytes of `cafe` followed by
  U+0301, reaches `encode_ast_value`'s `Leaf::Name(s)` outside NFC in a build that leaves
  `cadenza-ast`'s `std` feature off. `String.concat` folds only two all-ASCII constants (the
  `Prim::StrConcat` arm of `lower/compute.rs`), whose join is ASCII. The other `Name` leaves of that
  file are ASCII literals, such as `->`, or copies of names from the input tree.

**Why.** The NFC rule was stated as an obligation of the encoder's deduplication, and the `std`
`Builder` meets it by normalizing as it interns. The no_std core can't normalize, since its NFC
dependency is std-only, and its comment assumed that it never builds names from unnormalized text.
That holds for decode, which builds a `Name` from bytes a producer already wrote, but not for a
producer that builds a `Name` from a runtime string, or from a constant string that the compiler folds
without normalizing it.

**The requirement it drove.** [ast-binary-format.md](../contracts/ast-binary-format.md), Equal Trees
Encode To Identical Bytes: an encoder that does not normalize text writes only `Name` leaf text that is
already in NFC. The producer side is a change of its own: it makes `cdz-runtime`'s `ast-encode`, and
`rcdzc`'s fold where a constant can fall outside NFC, write only NFC `Name` text, and states whether a
stored or hashed program can already hold such text; where one can, that change carries a container
version increment and a migration path under Additive Evolution Of This Contract.
