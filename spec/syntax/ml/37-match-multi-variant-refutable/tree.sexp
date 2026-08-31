(do
  (type Opt (Some Int64) None)

  (def (f o) (match o ((Some x) x))))
