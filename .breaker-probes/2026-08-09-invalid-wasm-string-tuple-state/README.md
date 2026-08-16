# INVALID WASM: tuple(scalar,string) state + branch-suffix growth, 2 puts + 2 sizes (tick 1000, base b1745f321)

INVERTED differential: rust + rust-async COMPUTE correctly; wasm emits an INVALID COMPONENT
("cdz-run: invalid component: failed to compile: wasm[0]::function[14]") - a validation failure at load.

Bisection (all base b1745f321):
- slmin11 = MINIMAL: 2 puts + 2 sizes, put grows string by (if (= (% s 3) 0) "x" "yz") -> INVALID WASM
- slmin10: ONE put + 2 sizes, same branch -> OK
- slmin8:  2 puts + ONE size -> OK
- slmin9:  2 puts + 2 sizes, CONSTANT suffix (no branch) -> OK
- slmin4:  2 puts + 2 sizes but puts DON'T touch the string -> OK
- slmin5:  SCALAR string state (no tuple), growing puts, 2 sizes -> OK
- sl-min1/2/6: inner-handle variants all OK (inner handle immaterial)
Trigger = [tuple (scalar, string) state] x [arm rebuilds tuple with BRANCH-picked concat suffix] x [>=2 growth dispatches] x [>=2 reads of the grown value].
Suspect: the tail-resumptive fold's stack/local typing for the branch-merged rope value inside the rebuilt tuple - function[14] fails wasm validation.
