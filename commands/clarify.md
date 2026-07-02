# Command — clarify

**Purpose.** Remove ambiguity from a specification by structured questioning, and turn each resolution
into a concrete requirement. Ambiguity is the enemy of a reproducible regeneration: an
under-determined spec lets two generations diverge.

**Agent-agnostic.** Neutral prompt body.

## Usage

`clarify <target>` where `<target>` names the capability or contract to clarify.

## Procedure

1. Read the target specification and identify points where a conforming generation could reasonably
   choose differently: unstated defaults, undefined edge cases, invariants that are implied but not
   stated.
2. Ask the fewest, highest-leverage questions needed to resolve them. Prefer questions whose answer
   changes what a regeneration would produce.
3. For each resolved ambiguity, write the resolution as a new RFC-2119 requirement — a single
   self-contained sentence under a stable heading — rather than as prose commentary. A clarification
   that adds no requirement changes no future generation.
4. For each resolved point that a conforming generation could still resolve more than one way, also
   add its **declared default**: a requirement stating the conforming choice to apply when the point is
   otherwise unresolved (per `spec/capabilities/build-modes.md` §"An Open Point Carries A Declared
   Default"). This is what lets an autonomous build proceed without halting. Where the standalone rule
   forbids naming an implementation choice in the spec, the declared default instead names the location
   outside the spec at which the choice is pinned: a decision directory under `options/` whose README
   carries a `DEFAULT: <choice>` line.
5. Run `analyze` to confirm the new requirements extract cleanly and traceability remains complete.

## Guardrails

- A clarification MUST land as an extractable requirement, not as a note.
- Do not weaken an existing requirement to resolve an ambiguity; add a new one that narrows the
  behavior.
- A clarification of an open point MUST be paired with a declared default, so that the attended path
  (halt-and-harden) and the autonomous path (apply-the-default) stay coherent — a resolved ambiguity
  with no default just moves the halt later.
