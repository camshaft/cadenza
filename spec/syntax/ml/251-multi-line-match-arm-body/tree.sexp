(def
  (profile-half-extent (: p (Profile Rational)))
  (match
    p
    (((. Profile Rect) sz) (let ((((. Vec2 V2) w h) sz)) ((. Vec2 V2) (rhalf w) (rhalf h))))
    (((. Profile Circle) r) ((. Vec2 V2) r r))
    (((. Profile PathProfile) pth) (path-half-extent pth))))
