(case "a local binding shadowing a builtin MODULE name makes member access read the binding"
  (doc    "A Built-In Module Is A Record Of Its Operations — and therefore an ordinary shadowable
           binding, exactly as the `list`/`tuple` constructor aliases are (:880/:915 pin those): `(let
           ((Map (record (len k)))) (. Map len))` binds `Map` to a plain record, so the member access
           reads the BINDING's `len` field (7), not the builtin module's `len` operation. Pins that
           module names get no special resolution in member-access head position — one name never
           resolves two ways (Binding Is Lexical), the module-name face of the alias-shadowing rule.")
  (input  (do
            (def (main (: k Int64))
              (let ((Map (record (len k))))
                (. Map len)))
            (export main)))
  (call   main (: 7 Int64)) (output (: 7 Int64)))

(case "the builtin module is intact outside the shadowing scope"
  (doc    "The scope-extent twin: `inner` shadows `Map` with a record (reads 7 via the binding), while
           `main` — OUTSIDE the shadow — still reaches the real builtin (`Map.len (Map.insert
           Map.empty 1 2)` = 1) → 8. Pins that shadowing a module name is scoped exactly like any
           binding: the builtin record is untouched beyond the `let`'s extent, and the same spelled
           name resolves to the binding inside and the module outside with no leakage either way.")
  (input  (do
            (def (inner (: k Int64)) (let ((Map (record (len k)))) (. Map len)))
            (def (main (: k Int64))
              (+ (inner k) (Map.len (Map.insert Map.empty 1 2))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 8 Int64)))
