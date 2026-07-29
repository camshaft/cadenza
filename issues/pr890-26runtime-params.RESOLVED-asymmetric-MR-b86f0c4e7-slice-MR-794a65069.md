# PR#890 review comment — 26-runtime-params asymmetric-seed row is unpinned (corpus-bugfix)

Mirrored from GitHub PR#890 review comment (Copilot), id `3671882609` (:390, also :410).
File: `spec/semantics/26-runtime-params.sexp` — corpus → corpus-bugfix. Blame `bf54ab751` "corpus(2 files):
5-pin micro-drain B — handler binder hygiene + @param seeds".

## Comment (verbatim)

- (id 3671882609, 26-runtime-params.sexp:390) "The doc for this case says the asymmetric host row
  (a=5,b=2) is load-bearing to catch swapped/aliased seeding, but the case only tests a=10,b=20. That
  leaves the stated failure mode unpinned. This issue also appears on line 410 of the same file."

## Liaison verification (confirmed on trunk 9872e4458)

Case "two @params seed two NESTED handlers independently" (:368). Doc (:371-374): "a + b = 30 with hosts
10/20; the asymmetric 5/2 row separates the seeds → 7. A seed wiring that read the params in the wrong
order (or seeded both levels from one accessor) collapses the sum symmetrically and only the asymmetric
row catches it." But the case has ONLY `(host-responses (respond Param.a (: 10 …)) (respond Param.b (: 20
…)))` → 30. There is NO 5/2→7 row. Since `+` is commutative, 10/20 does NOT actually catch a swapped-order
or single-accessor seeding (10+20 == 20+10 == 10+10 if both read `a`… wait: both-from-one-accessor with
a=10 gives 10+10=20≠30, so THAT is caught; but a SWAPPED order 20+10=30 is NOT). So the doc's headline
failure mode (wrong-order seeding) is genuinely unpinned by 10/20 — an asymmetric row (5/2→7) is needed.
Line 410 (the sibling `@param slice-window` case) is flagged as the same class — a stated
discriminating row not actually run (verify: its doc vs its single host-response).

Fix: add the asymmetric `(host-responses (respond Param.a (: 5 …)) (respond Param.b (: 2 …)))` call+output
(→7) row (and the analogous discriminating row at :410) so the doc's claimed failure mode is pinned.
Corpus coverage; behavior-neutral to the compiler.

Owner: **corpus-bugfix** (`spec/semantics/*.sexp`; `bf54ab751`). Add the missing discriminating rows.
