// 2nd operator repro (concierge) of the SAME module-qualified-call bug as
// mlrepro-module-qualified-call-browser-stackoverflow.ml — different browser symptom (OOB vs stack-overflow), same root.
// v-inference ROOT-CAUSED (MR incoming): NOT resolve.rs recursion — an inference re-entry cycle
// (type_of of the member-lambda's unannotated param → expected_arrow_for_lambda re-reads the same param's type_of,
// cycling to the 1024 descent limit → ~1024 frames that fit the 64MB native thread but overflow the browser worker stack).
// Fix = arrow_lambdas_in_progress re-entry guard (depth 1000+ → ~5). Circle → 300; shipped as v-inference's regression test.
// (Earlier ;; comments here were a MY error — ML comments are // not ;; ; the parser read comment words as unit tokens.)
module Circle {
  def pi = 3
  def area(r) = pi * (r * r)
  export { area }
}
def main() = Circle.area(10)
export { main }
