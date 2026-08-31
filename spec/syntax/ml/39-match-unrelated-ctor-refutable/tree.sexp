(do
  (type Wrapper (Wrap Int64))

  (def (f o) (match o ((Other x) x))))
