; breaker WIT battery W (2026-08-26): uncovered WIT-shape probes against the general WIT emission.
; Imposed worlds; expectations written for full-generality (operator mandate: any WIT def supported).

(case "w01 string param and result cross the export boundary"
  (wit-world (world w (export iface (member f (func (param s ("string")) (result ("string")))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: s String)) (String.concat s "!")) (export f)))
  (call f (: "hi" String))
  (output (: "hi!" String)))

(case "w02 char param and result cross the export boundary"
  (wit-world (world w (export iface (member f (func (param c ("char")) (result ("char")))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: c Char)) c) (export f)))
  (call f (: #\a Char))
  (output (: #\a Char)))

(case "w03 narrow scalars s8 and u16 cross the export boundary and sum to s64"
  (wit-world (world w (export iface (member f (func (param a (s8)) (param b (u16)) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: a Int8) (: b UInt16)) (+ (Int64.of a) (Int64.of b))) (export f)))
  (call f (: -3 Int8) (: 500 UInt16))
  (output (: 497 Int64)))

(case "w04 f32 param widens to f64 result across the export boundary"
  (wit-world (world w (export iface (member f (func (param x (f32)) (result (f64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: x Float32)) (Float64.of x)) (export f)))
  (call f (: 1.5 Float32))
  (output (: 1.5 Float64)))

(case "w05 enum field in a record result crosses the export boundary"
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result ("record" (mode ("enum" fast slow)))))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Mode (Fast) (Slow))
             (def (f (: m (Record (: x Int64)))) (record (= mode (if (= (. m x) 0) Mode.Fast Mode.Slow))))
             (export f)))
  (call f (: (record (= x 0)) (Record (: x Int64))))
  (output (: (record (= mode (fast unit))) (record (mode mode)))))

(case "w06 flags field in a record result crosses the export boundary"
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result ("record" (perms ("flags" read write)))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: x Int64)))) (record (= perms (record (= read true) (= write false)))))
             (export f)))
  (call f (: (record (= x 0)) (Record (: x Int64))))
  (output (: (record (= perms (record (= read true) (= write false)))) (record (perms (record (read Bool) (write Bool)))))))

(case "w07 top-level tuple result crosses the export boundary"
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result ("tuple" (s64) ("list" (u8)))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: x Int64)))) (tuple (. m x) b"\x07")) (export f)))
  (call f (: (record (= x 9)) (Record (: x Int64))))
  (output (: (tuple 9 (7)) (Tuple Int64 (List UInt8)))))

(case "w08 option<option<s64>> field crosses the export boundary"
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result ("record" (d ("option" ("option" (s64)))))))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: x Int64)))) (record (= d (Option.Some (Option.Some (. m x)))))) (export f)))
  (call f (: (record (= x 5)) (Record (: x Int64))))
  (output (: (record (= d (Some (Some 5)))) (record (d (Option (Option Int64)))))))

(case "w09 top-level result<s64,string> export result crosses the boundary on the ok arm"
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result ("result" (s64) ("string"))))))))
  (component-name "cadenza:demo/iface")
  (input (do (type R (Ok Int64) (Err String))
             (def (f (: m (Record (: x Int64)))) (if (> (. m x) 0) (R.Ok (. m x)) (R.Err "neg")))
             (export f)))
  (call f (: (record (= x 4)) (Record (: x Int64))))
  (output (: (ok 4) r)))

(case "w10 enum host-import RESULT threads into the export result"
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member mode (func (result ("enum" fast slow)))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Mode (Fast) (Slow))
             (effect hosti (op mode (-> Unit Mode)))
             (def (f (: m (Record (: x Int64))))
               (host (hosti) (match (hosti.mode unit) ((Mode.Fast) 1) ((Mode.Slow) 2))))
             (export f)))
  (call f (: (record (= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.mode (: Mode.Fast Mode)))
  (host-calls (call cadenza:demo/hosti.mode))
  (output (: 1 Int64)))

(case "w11 enum host-import PARAM is lowered and delivered"
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member set (func (param v ("enum" fast slow)) (result ("unit")))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Mode (Fast) (Slow))
             (effect hosti (op set (-> Mode Unit)))
             (def (f (: m (Record (: x Int64))))
               (host (hosti) (do (hosti.set Mode.Fast) 7)))
             (export f)))
  (call f (: (record (= x 0)) (Record (: x Int64))))
  (host-calls (call cadenza:demo/hosti.set))
  (output (: 7 Int64)))

(case "w12 TWO distinct host-import interfaces both delegate from one guest"
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result (s64)))))
               (import cadenza:demo/clock (member now (func (result (u64)))))
               (import cadenza:demo/sink (member push (func (param vals ("list" (s64))) (result ("unit")))))))
  (component-name "cadenza:demo/iface")
  (input (do (effect clock (op now (-> Unit UInt64)))
             (effect sink (op push (-> (List Int64) Unit)))
             (def (f (: m (Record (: x Int64))))
               (host (clock sink) (do (sink.push (list 1 2)) (Int64.of (clock.now unit)))))
             (export f)))
  (call f (: (record (= x 0)) (Record (: x Int64))))
  (host-responses (respond clock.now (: 42 UInt64)))
  (host-calls (call cadenza:demo/sink.push) (call cadenza:demo/clock.now))
  (output (: 42 Int64)))
