# Decision — Type Mapping

**The decision.** The concrete Cadenza-to-host-interface type table that realizes the component-abi.md
frozen contract's requirement that each Cadenza type appearing in an exported or imported signature
has a single stable representation in the host interface's type system.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- Every exported type has a single stable boundary representation (component-abi.md).
- Generics do not cross the boundary; they are monomorphized first (component-abi.md).
- Aggregate layout is determined by the declared type alone (component-abi.md).

This is an ABI-level decision: changing an existing row alters bytes produced from unchanged source
and is a coordinated change under the constitution's Governance Floors; adding a row for a type that
had no boundary representation is additive.

## Choices

- [`component-model-types`](./component-model-types.md) — the Cadenza-to-WIT table (integers,
  float, big-integer, rational, char, string, list, record, variant, option, result, functions as
  resources). **The default.**

DEFAULT: component-model-types
