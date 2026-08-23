(module reducer
  (world reducer-world
    (export guest
      (member on-message
        (func (param msg ("record" (contract ("list" (u8))) (payload ("list" (u8))) (token ("list" (u8)))))
              (result ("list" (u8)))))))
  (def (on-message (: msg (Record (contract Bytes) (payload Bytes) (token Bytes))))
    (. msg payload))
  (export on-message))
