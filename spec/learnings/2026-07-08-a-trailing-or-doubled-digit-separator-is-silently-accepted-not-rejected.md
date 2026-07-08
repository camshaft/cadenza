# A trailing or doubled digit separator is silently accepted, not rejected

*2026-07-08*

**What happened.** Adversarial probing of numeric literal lexing found the reader silently
accepting malformed digit separators. `1_` reads as the value `1`, `1__0` reads as `10`, `1_0_`
and `1_000_` and `12_` all read as their digits with the underscores dropped, and `0xFF_` reads
as 255. The digit-separator rule is "meaningful only between digits," but the reader strips every
`_` regardless of position, so a separator with no digit on one side is silently normalized away
rather than rejected.

**Why it is a break.** The corpus (01-literals.sexp) pins that a digit separator `_` "is only
meaningful BETWEEN digits" — it establishes this for the leading-underscore boundary (`_1` is an
identifier, not `1` with a stray separator) and restates it for the radix literals. "Between
digits" is a both-sides condition: a `_` needs a digit on its left AND its right. A trailing `_`
(`1_`) has none on the right; a doubled `__` has a `_` (not a digit) on one side. So `1_`, `1__0`,
`1_0_` are malformed literals. A digit-led token is a number (the number/identifier boundary the
corpus pins), and a malformed number is a well-formedness rejection (CDZ0201), the same class as
an out-of-range literal — never silently read as the digits with the separator removed.

**Root cause — the reader strips separators unconditionally.** The lexer removes every `_` from a
numeric token before parsing the digits, rather than requiring each `_` to sit between two digits.
So the position of the separator is never validated: leading `_` is handled specially (a `_`-led
token is classified as an identifier, which is why `_1` works), but trailing and interior-doubled
separators fall through to "strip and parse," which accepts them. The fix is to validate separator
placement during lexing — a `_` is well-formed only with a digit immediately before and after it —
and reject a token with a misplaced separator as a malformed literal (CDZ0201).

**Severity and scope.** This is a reader/front-door defect, not a miscompile in the trusted
compiler path (contracts/ast-encoding.md notes the reader is the seed toolchain's front door, not
the trusted path). The accepted value is benign (`1_` = 1 is what a lenient reader intends), so it
is lower-severity than a wrong-value miscompile. But it is squarely the same class the corpus
already guards — a digit-led token that is malformed must be rejected, not misclassified or
silently normalized — so it is worth pinning: the corpus carefully fixes the leading-underscore
and out-of-range boundaries but left the trailing/doubled-separator boundary unpinned, and the
reader accepts what the between-digits rule forbids.

**The lesson.** "Only between digits" is a two-sided constraint, and a reader that implements it as
"strip all separators then parse" satisfies neither side — it accepts a separator anywhere. The
positive case (`1_000_000`) passes under both the correct rule and the lenient strip-all
implementation, so the corpus's positive witness cannot distinguish them; only a *malformed*
witness (a separator NOT between two digits) exercises the constraint. A between-X rule needs a
negative case on each side to pin that X is required on both.

**Corpus case added.** `spec/semantics/01-literals.sexp` §"a trailing digit separator is a
malformed literal, not the digits with it dropped" — `1_` MUST reject CDZ0201, with a note covering
the doubled (`1__0`) and trailing-group (`1_000_`) siblings. Native seed reader; the behavior gate
catches it (expected reject CDZ0201, observed a running component).
