# The compile seam is statically typed — bytes in, bytes out, never a dynamic value

*2026-07-03*

**What happened.** While building the self-hosting harness, the seed's derivation seam invoked the
Cadenza-authored compiler as `eval_compiler_call(compiler, "compile", arg: Value) -> Result<Value,
…>` — it wrapped the program's binary AST in the seed's dynamic `Value` (a `Value::Bytes`), applied
the compiler function, and then matched the returned `Value` back down to bytes. The compiler's
`compile` was thereby presented to the outside world as a dynamic `Value -> Value` function, even
though what it *is* is a total transform from a program's binary AST to a component's bytes. The
harness exposed the mismatch sharply: the *compiled* compiler is a real WebAssembly component whose
entry is typed `list<u8> -> list<u8>`, so to compare the interpreted compiler against the compiled
one (the whole point of self-hosting), the interpreted side had to be invoked through the *same*
static type. Threading a `Value` through one side and `list<u8>` through the other made the two
incomparable at the type level and, worse, baked a dynamic-language assumption into the compiler's
contract. The seam was retyped to `eval_compile(compiler, program_bytes: &[u8]) -> Result<Vec<u8>,
…>`; the `Value::Bytes` wrapping and unwrapping became an *internal* detail of how this generation's
dynamic interpreter happens to run a statically typed function, not part of the interface.

**Why.** The seed is a dynamic interpreter (Core Principle VII's bootstrap carve-out), but the
language it is bootstrapping is statically typed, and the compiler's derivation interface is one of
the earliest and most load-bearing type signatures in the whole system: `compile : list<u8> ->
list<u8>`. If the seed's *dynamic* value representation leaks into that interface — if `compile` is
specified or invoked as "a function from a dynamic value to a dynamic value" — then every future
generation inherits the assumption that the language is dynamic at exactly the seam where static
types matter most, and adding strong typing later becomes a contract change rather than an internal
refinement. The specification pinned that the compiler emits component bytes as an ordinary
byte-sequence value and that a program is *supplied* as its binary AST, but it did not pin that the
seed must *invoke* the compiler through a byte-to-byte interface rather than through its dynamic
value type. That gap let the implementation present the seam dynamically without violating any
requirement — precisely the kind of under-determination that hardens into a wrong assumption.

**The requirement it drove.** Added to `spec/bootstrap.md` §"The Compiler Is Authored In Cadenza,
Not In The Seed" two requirements: the seed MUST invoke the Cadenza-authored compiler through an
interface typed as a byte sequence to a byte sequence (binary AST in, component bytes out), so the
derivation interface is statically typed and the seed's dynamic evaluation of it is not part of that
interface; and the seed MUST NOT require that interface to consume or produce a value of the seed's
dynamic value representation, so the interface carries no dynamic-typing assumption and a later
generation may type-check the same `compile : list<u8> -> list<u8>` without changing it. This keeps
the interpreted seam and the compiled component's exported entry the *same static type*, which is
what makes the self-hosting fixpoint a clean, like-for-like byte comparison.
