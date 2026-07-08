# Decision — Execution Model

**The decision.** The concrete component format, runtime engine, determinism configuration, resource
metering, and host interface that realize the execution-model requirements the specification states
technology-neutrally (constitution IV, VI; host-interface-binding.md; determinism-and-fuel.md;
build-tool-interface.md).

**Requirements any choice must satisfy (from the spec — do not weaken):**
- Content-addressed binary component behind a versioned host interface (constitution VI).
- Imports mirror the manifest exactly; the compiler adds no undeclared capability (constitution IV;
  host-interface-binding.md §Capability Honesty).
- The compiler introduces no undeclared nondeterminism (constitution III; determinism-and-fuel.md).

Bounding a run's execution is **not** a requirement any choice must satisfy (constitution Principle V,
retired by Amendment 0.7.0); a choice's engine provides resource metering as an operational facility,
recorded as a declared default rather than required here.

## Choices

- [`wasm-component-model`](./wasm-component-model.md) — WebAssembly Component Model, an embeddable
  component-model runtime, fuel metering, and the `cadenza-host/1` interface. **The default.**

DEFAULT: wasm-component-model
