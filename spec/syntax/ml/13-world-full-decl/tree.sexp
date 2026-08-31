(world
  Reducer
  (export fold (member apply (func (param event Bytes) (result Bytes))))
  (import
    kv
    (member get (func (param key String) (result Bytes)))
    (member put (func (param key String) (param value Bytes) (result ("unit"))))))
