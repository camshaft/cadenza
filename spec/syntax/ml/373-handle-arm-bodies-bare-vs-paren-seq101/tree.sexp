(def
  (main db0)
  (handle
    DbState
    db0
    ((get-tcol () db (resume (types-col db) db))
      (get-ty (id) db (resume (require-ty db id) db))
      (set-ty (pair) db (let (((tuple id t) pair)) (resume unit (fill-ty db id t))))
      (get-resolved (id) db (resume (require-resolved db id) db)))
    (run-program db0)))
