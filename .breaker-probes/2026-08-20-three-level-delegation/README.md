# three-level-delegation — 3 nested handlers, inner arms delegate outward

## pynd4 — C.cc -> {B.bb, A.aa}; B.bb -> A.aa; A.aa base. All deep/tail.
Seeds A=n%3, B=5, C=0. Body C.cc + C.cc.
Traced (n=10, sA0=1): C.cc#1 = B.bb#1(5 + A.aa=10) + A.aa=20 = 35; C.cc#2 = B.bb#2(6 + A.aa=30) + A.aa=40 = 76; sum 111.
n=0: 71. Verified 111/71 x3 + opt-sweep 0-div. Extends pynd3 (2-level) to 3-level outward delegation;
A's single counter advances across performs originating from C, from B, and directly — all correctly threaded.
