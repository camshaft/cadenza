; W follow-ups: discriminate emit-bug vs harness-limitation for w01/w09/w10.

(case "w01b string RESULT only (record param) crosses the export boundary"
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result ("string")))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: m (Record (: x Int64)))) "hi!") (export f)))
  (call f (: (record (= x 0)) (Record (: x Int64))))
  (output (: "hi!" String)))

(case "w01c string PARAM only (s64 result) crosses the export boundary"
  (wit-world (world w (export iface (member f (func (param s ("string")) (result (s64)))))))
  (component-name "cadenza:demo/iface")
  (input (do (def (f (: s String)) (String.byte-len s)) (export f)))
  (call f (: "hello" String))
  (output (: 5 Int64)))

(case "w09b result<s64,string> as a record FIELD crosses the export boundary"
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result ("record" (r ("result" (s64) ("string"))))))))))
  (component-name "cadenza:demo/iface")
  (input (do (type R (Ok Int64) (Err String))
             (def (f (: m (Record (: x Int64)))) (record (= r (if (> (. m x) 0) (R.Ok (. m x)) (R.Err "neg")))))
             (export f)))
  (call f (: (record (= x 4)) (Record (: x Int64))))
  (output (: (record (= r (Ok 4))) (record (r r)))))

(case "w09c top-level result<s64,string> with a SCALAR param"
  (wit-world (world w (export iface (member f (func (param x (s64)) (result ("result" (s64) ("string"))))))))
  (component-name "cadenza:demo/iface")
  (input (do (type R (Ok Int64) (Err String))
             (def (f (: x Int64)) (if (> x 0) (R.Ok x) (R.Err "neg")))
             (export f)))
  (call f (: 4 Int64))
  (output (: (ok 4) r)))

(case "w10b enum host-import RESULT with variant-form respond clause"
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member mode (func (result ("enum" fast slow)))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Mode (Fast) (Slow))
             (effect hosti (op mode (-> Unit Mode)))
             (def (f (: m (Record (: x Int64))))
               (host (hosti) (match (hosti.mode unit) ((Mode.Fast) 1) ((Mode.Slow) 2))))
             (export f)))
  (call f (: (record (= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.mode (: (fast unit) mode)))
  (host-calls (call cadenza:demo/hosti.mode))
  (output (: 1 Int64)))

(case "w10c VARIANT-with-payload host-import result threads into the export result"
  (wit-world (world w (export iface (member f (func (param m ("record" (x (s64)))) (result (s64)))))
               (import cadenza:demo/hosti (member pick (func (result ("variant" (small (s64)) (big))))))))
  (component-name "cadenza:demo/iface")
  (input (do (type Pick (Small Int64) (Big))
             (effect hosti (op pick (-> Unit Pick)))
             (def (f (: m (Record (: x Int64))))
               (host (hosti) (match (hosti.pick unit) ((Pick.Small k) k) ((Pick.Big) 999))))
             (export f)))
  (call f (: (record (= x 0)) (Record (: x Int64))))
  (host-responses (respond hosti.pick (: (Pick.Small 5) Pick)))
  (host-calls (call cadenza:demo/hosti.pick))
  (output (: 5 Int64)))
