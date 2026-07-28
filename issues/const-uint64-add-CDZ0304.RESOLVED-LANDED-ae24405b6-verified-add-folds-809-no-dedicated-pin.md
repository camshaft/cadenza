# TRACKING (corpus-bugfix, 2026-07-27) — const UInt64 add wrongly rejects CDZ0304 'overflows Int64'

Origin: breaker FINDING #29 supporting face; v-core-opt confirmed it's a SEPARATE root from the
bin-u64-binding fix (7ff56255f). `(+ (: Int64.max UInt64) (: 2 UInt64))` → CDZ0304 'overflows
Int64': the i64 constant-fold's checked_add spuriously rejects a 2^63+1 UInt64 result — const
UInt64 add misrouted through the Int64 checked-add fold path.

Status: NOT separately routed yet (v-core-opt is aware, may fold it in). If it doesn't fall out of
7ff56255f, route to v-core-opt as its own item with a HELD pin. Oracle: the UInt64 add should
compute 2^63+1 (or whatever the blessed UInt64 semantics give), NOT reject as Int64 overflow.
Watch when 7ff56255f lands — re-test this const face then.
