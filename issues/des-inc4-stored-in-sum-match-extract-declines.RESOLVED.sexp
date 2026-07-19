(do
  (type Instant (Instant UInt64))
  (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
  (type Box (Box (-> Unit Instant)))
  (def (unbox-apply (: b Box)) (match b ((Box.Box th) (th unit))))
  (effect Sim
    (op sleep (-> Instant Unit))
    (op now   (-> Unit Instant)))
  (def (main)
    (handle Sim (Instant.Instant 0)
      ( (now   (u) s (resume s s))
        (sleep (wake) s
          (unbox-apply (Box.Box (fn (_u) (resume unit wake))))) )
      (do (Sim.sleep (Instant.Instant 5000000000))
          (inst-ns (Sim.now)))))
  (export main))
