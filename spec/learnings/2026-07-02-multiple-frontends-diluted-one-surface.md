# Multiple front-ends diluted one surface

*2026-07-02*

**What happened.** Earlier Cadenza grew several front-end syntaxes that each parsed a different
surface language — a Cadenza surface, a Markdown surface, a SQL surface, a G-code surface — all
lowering into one shared abstract syntax tree for the same evaluator. Effort spread across surfaces
before any single surface, or the core beneath them, was solid, and the language's identity blurred
across the several front doors.

**Why.** Surface breadth was pursued before a canonical representation existed to project those
surfaces from. Without a single durable representation as the thing that *is* the program, each
surface was a parallel definition of the language rather than a view onto one, and there was no
principled place to add or drop a surface.

**The requirement it drove.** The [`code-shape.md`](../../defaults/code-shape.md) declared default:
one **homoiconic canonical representation** with **display decoupled from representation**. A program
*is* the representation; a display — conventional for humans, homoiconic for metaprogramming, or any
other — is a deterministic projection of it, and adding or removing a display touches no contract and
no capability requirement. Alternate surfaces re-enter only as displays over the one representation,
never as parallel definitions of the language. [Core Principle X](../../constitution.md) fixes that a
canonical form round-trips and that structure is manipulable through one structural interface.
