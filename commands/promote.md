# Command — promote

**Purpose.** Accept a candidate generation of the compiler as the current one, and
record the promotion decision and its provenance. A generation is accepted only if
both gates pass and its end-to-end derivation is real; this command records which
generation, which component hash, and which gate reports justify the decision.

**Agent-agnostic.** Neutral prompt body.

## Preconditions

- The candidate passed **both** gates against the full config
  (`.duvet/config.toml`): the requirement gate (100% of load-bearing MUST/SHALL
  covered, zero broken citations) and the behavior gate (every
  `spec/semantics/*.sexp` case reproduces its recorded output). Both are required
  by `conformance-gate.md` §"The Gate Is The Promotion Bar".
- The candidate's coverage is honest per `conformance-gate.md` §"A Citation
  Discharges Its Own Requirement": no vacuous, shared, or stub citations, and no
  behavior requirement discharged by shape rather than execution
  (`conformance-gate.md` §"A Behavior Requirement Is Covered Only By Execution").
- The end-to-end derivation is real, not modeled: the generation was actually
  derived by the previous generation and the derived component was run
  (`self-hosting-and-bootstrap.md` §"A Regeneration Is Derived, Gated, And Run").

## Procedure

1. Assemble the promotion record: the generation identifier, the content hash of
   the derived component being promoted, the gate report references (both gates),
   and the toolchain identity that produced it — the compiler-component hash plus
   the host-toolchain hash (`reproducible-derivation.md` §"Derivation Is A Function
   Of Source And Toolchain"; `options/toolchain/`).
2. Re-demonstrate the end-to-end path on the real derived-and-run component before
   accepting it (`self-hosting-and-bootstrap.md` §"Every Generation Re-Demonstrates
   The End-To-End Path"):
   - its imports mirror its declared capability manifest;
   - re-deriving the same source with the same toolchain reproduces a
     byte-identical component;
   - its compiled output agrees with the recorded corpus semantics (the oracle) on
     every executable-semantics case, and the two compiler implementations agree with
     each other
     (`self-hosting-and-bootstrap.md` §"A Compiled Program Agrees With The Recorded
     Semantics").
3. Record the accepted generation as the current toolchain identity, recorded
   alongside the produced component so the next `regen` derives from it, and
   confirm the whole thing is reconstructable: which generation, which hash, which
   gate reports, against which content-addressed specification snapshot
   (`conformance-gate.md` §"A Gate Run Judges Against A Content-Addressed
   Snapshot").

## Guardrails

- A promotion MUST NOT rest on vacuous coverage or a modeled derivation: a
  generation demonstrated only by emitting the artifacts a derivation would
  produce, without a component that was actually derived and run, is not
  promotable (`spec/bootstrap.md` §"A Modeled Derivation Is Not An Ignition";
  `self-hosting-and-bootstrap.md` §"A Regeneration Is Derived, Gated, And Run").
- A generation whose compiled output disagrees with the recorded corpus semantics on
  any executable-semantics case MUST NOT be promoted
  (`self-hosting-and-bootstrap.md` §"A Compiled Program Agrees With The Recorded Semantics").
- A change that would alter a frozen contract or weaken a governance floor is not
  a promotion; it MUST carry the explicit human approval the constitution's
  Governance Floors require — a promotion record alone does not substitute for it.
