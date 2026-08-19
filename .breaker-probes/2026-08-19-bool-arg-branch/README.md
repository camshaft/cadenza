# bool-arg-branch — op with a Bool arg; arm branches the whole resume on it
## pybf1 — flag(b): true→(* s 10)/+1, false→(+ s 100)/*2. Two dispatches true+false. Model 1102/101. PASS x3.
Bool-typed op arg drives an if selecting both answer and next-state; both branches fire. Promotable.
