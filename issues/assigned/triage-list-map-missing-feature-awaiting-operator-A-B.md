# TRIAGE: List.map is a MISSING FEATURE (not a fold-bug) — awaiting operator A/B design ruling
List.map / higher-order list ops don't exist in the prelude (CDZ0201 'List has no member map', identical
const+runtime — verified fresh). NOT a compile-time-fold gap, NOT a sound-decline of an existing op.
Concierge routed to OPERATOR with strong (A) lean:
  (A) add List.map/filter/fold to prelude (rcdzc-core + v-iterators reuse pull-iterator map/filter machinery)
      → corpus-bugfix adds a WORKING-List.map corpus case once impl lands.
  (B) mapping via iterators only → v-diagnostics adds an iterator-surface fix-hint to the CDZ0201
      → corpus-bugfix pins the HINTED decline.
PIN HELD until the operator rules (which pin depends on A/B). v-metaprogramming unblocked meanwhile.
