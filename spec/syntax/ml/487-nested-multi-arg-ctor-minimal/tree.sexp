(do
  (type Vec3r (V3r Rational Rational Rational))

  (type Solidr (Cuber Vec3r))

  (def (r (: n Int64)) ((. Rational of) n 1))

  (def (main) ((. Solidr Cuber) (V3r (r 4) (r 4) (r 4)))))
