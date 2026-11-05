# A frozen contract names the widths of the byte-level form it pins

*2026-11-05*

**What happened.** Core Principle XIII of the constitution said that a normative statement must not
name a numeric width, AGENTS.md rule 2 said the same, and check 4 of commands/analyze.md reported each
fixed integer or float width in a normative file, except under spec/learnings/ and options/. Every file
under spec/contracts/ is normative, and two frozen contracts name widths in their requirements.
[ast-binary-format.md](../contracts/ast-binary-format.md) names the eight-byte container header, the
7-bit groups of a `varu64` and its ten-byte limit, the eight-byte big-endian exponent of a float body,
and the unsigned 32-bit bound on an index. [ast-typed-values.md](../contracts/ast-typed-values.md) names
the 64-bit and the 32-bit binary floating-point formats in its float rows and in the rule by which a
decoder rounds a float leaf to its target. The requirement gate extracts these sentences all the same,
since it doesn't check widths, but analyze's check 4 reports both contracts, so analyze couldn't end in
`ANALYZE: PASS` while either contract stated its bytes.

**Why.** The ban keeps a requirement independent of how a generation realizes it: the width of an
integer that a compiler uses is a technology choice, recorded at the declared-default location, and a
requirement stays true whatever choice a generation makes. A frozen contract is the other case. It pins
a byte-level form so that a third party can implement the bytes from the contract alone, and every
generation writes the same bytes, so the widths of those bytes are part of the form itself. A decoder
can't read a container header without knowing how many bytes it has, and the number 300 is written as
the integer decimal 300 when it is a 64-bit value and as the shortest decimal 3 x 10^2 when it is a
32-bit value, so the format of the value decides its bytes. Two other ways were considered and refused.
Moving the widths to `options/` doesn't fit, because a declared default is a choice a generation may
make another way, and these bytes are the same for every generation. Rewording the contracts without
the widths would change sentences that the requirement gate identifies by their quoted text, and a byte
layout stated without its sizes is harder to implement and to check than one that states them.

**The requirement it drove.**

- [constitution.md](../../constitution.md), Core Principle XIII, second sentence (Amendment 0.9.0): a
  numeric width that a frozen contract names as part of the byte-level form it pins is the one exception
  to the ban.
- [AGENTS.md](../../AGENTS.md) rule 2 states the same exception for authors.
- [commands/analyze.md](../../commands/analyze.md) check 4 doesn't report such a width, and still
  reports a width that a frozen contract names outside the form it pins, and every width in another
  normative file.
- The checklist of [templates/contract.md](../../templates/contract.md) and the conventions of
  [commands/specify.md](../../commands/specify.md) say the same for a new frozen contract.

The amendment doesn't change any requirement of a frozen contract or any byte of a tree, a program or a
component, and it doesn't weaken any of the minimum-process rules the constitution sets for a change to
the specification.
