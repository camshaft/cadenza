# map-read-after-write — Map-state handler, insert-then-read across the resume seam
## pymr1 — put(k,v) threads Map.insert as next-state; later fetch(k) sees it; fetch(seed-key) sees seed. Model 6060/5050. PASS x3.
Heap read-after-write through the resume seam: the CHAMP Map threaded as handler state carries the
put's write to the subsequent fetch dispatch. Promotable pass-witness.
(API note: Map.empty / Map.insert / Map.lookup / Map.len — NOT Map.of/Map.get.)
