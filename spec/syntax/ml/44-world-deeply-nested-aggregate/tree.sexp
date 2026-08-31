(world
  W
  (export
    i
    (member
      get
      (func
        (param key (string))
        (result
          ("option"
            ("result"
              ("record" (val ("list" (u8))) (tags ("list" (string))))
              ("variant" (NotFound) (Corrupt (string))))))))))
