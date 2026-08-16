(do
  (effect io (op send (-> UInt8 Int64)))
  (def (main (: n Int64))
    (host (io) (io.send 999)))
  (export main))
