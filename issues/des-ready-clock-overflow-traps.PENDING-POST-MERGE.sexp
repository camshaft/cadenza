(case "at overflow"
  (input (do
    (type Duration (Duration UInt64))
    (type Instant  (Instant  UInt64))
    (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
    (def (dur-ns  (: d Duration)) (match d ((Duration.Duration n) n)))
    (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
    (def (main (: t UInt64) (: d UInt64)) (inst-ns (at (Instant.Instant t) (Duration.Duration d))))
    (export main)))
  (call main (: 18446744073709551610 UInt64) (: 10 UInt64)) (trap "integer overflow")
  (call main (: 1000000000 UInt64) (: 2000000000 UInt64)) (output (: 3000000000 UInt64)))
