# Spec gap: String and Bytes indexing lack a total-or-trap requirement of their own

*2026-07-05*

**What happened.** A `/loop` run added String out-of-bounds cases (`String.at "hi" 5`, a reversed or
past-the-end `String.slice`, a negative index) recording a **trap**, mirroring existing `Bytes.at`
out-of-bounds trap cases. But `collections-and-text.md` gives a dedicated total-or-trap requirement only
to **lists** — §"List Operations Are Total Or Trap": "An operation that indexes a list outside its
bounds MUST raise a trap of a defined kind rather than produce an unspecified value." There is no
equivalent §"String Operations Are Total Or Trap" or §"Bytes Operations Are Total Or Trap". The String
and Bytes trap cases lean on the *general* clause `core-semantics.md` §"Partial Operations Have A Defined
Outcome" — which requires a partial operation to "either evaluate to a value the executable semantics
defines **or** raise a trap of a defined kind." That general clause permits *either* a trap or a defined
value; it does not pin String/Bytes indexing to a trap the way the list requirement pins list indexing.

**Why.** The total-or-trap discipline was written as a per-type requirement (lists got theirs) rather
than as one requirement quantified over all indexable sequence types (list, string, bytes). So String
and Bytes inherited only the weaker general clause, under which a conforming compiler could legitimately
make `String.at "hi" 5` return a defined sentinel (e.g. an empty string, or an `Option`) instead of
trapping — a different observable behavior from the one the corpus now records. The corpus is pinning an
oracle (trap) that the specification permits but does not *require*, so a second conforming
implementation could disagree without violating any MUST.

**The requirement it drove.** *Deferred to a clarity pass* (this entry is the hand-off, per the
operator's request to document gaps for a clarity agent rather than resolve them inline). The resolution
should add a total-or-trap requirement covering String and Bytes indexing to `collections-and-text.md`
— either two new headings mirroring §"List Operations Are Total Or Trap" (§"String Operations Are Total
Or Trap", §"Bytes Operations Are Total Or Trap"), or a generalization of the existing list heading to
"a sequence type's indexing operation". This backs the String out-of-bounds cases in
`spec/semantics/13-strings.sexp` and the Bytes out-of-bounds cases in `spec/semantics/10-bytes.sexp`
with a MUST, so their recorded traps are required behavior rather than one permitted choice. Note the
*related but already-specified* case: `Bytes.of` on a value outside 0..=255 traps, which the general
partial-operations clause covers as "no result for those inputs"; the gap here is specifically the
out-of-*bounds index* choice (trap vs defined value) for String and Bytes reads.
