(def
  (demand-typed-leaf (: db Db) (: id Int64))
  (match
    (require-ty db id)
    (comment-after "memo HIT — no recompute" (((. Option Some) fact) #tuple(db fact)))
    (((. Option None) _)
      (comment
        "MISS — compute from source, fill, thread"
        (let ((ty (compute-leaf-type (db-tree db) id))) #tuple((fill-ty db id ty) ty))))))
