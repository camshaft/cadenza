(def
  (f)
  #tuple((embedded #"json" (json-object (member "a" 1)))
    (embedded
      #"toml"
      (toml-document
        (toml-kv (toml-key-path (toml-key "x" " " " " "" "")) (toml-integer " " " " "1" 1))
        ""))))
