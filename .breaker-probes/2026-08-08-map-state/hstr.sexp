(do
  (effect io (op log (-> String Int64)))
  (def (main (: k Int64)) (host (io) (io.log "hi")))
  (export main))
