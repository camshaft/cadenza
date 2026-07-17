;; 2nd operator repro (concierge) of the SAME module-qualified-call bug as
;; mlrepro-module-qualified-call-browser-stackoverflow.ml — different browser symptom (OOB vs stack-overflow), same root.
;; NATIVE cdz check on CURRENT trunk c48d9dc4c: PASSES CLEAN (rc=0) — P1 resolve HEALED (top-level module Circle
;; IS referenceable for Circle.area). concierge saw native CDZ0101 on an EARLIER trunk (pre-resolve-fix).
;; P0 (live): browser/guide-wasm path crashes "Memory access out of bounds" instead of matching native — likely
;; STALE guide-wasm build (native resolve fix not rebuilt into the browser wasm compiler). Routed to v-guide-infra.
;; Conformance case (once browser fixed): this program + the Temp one run/diagnose IDENTICALLY native + browser.
module Circle {
  def pi = 3
  def area(r) = pi * (r * r)
  export { area }
}
def main() = Circle.area(10)
export { main }
