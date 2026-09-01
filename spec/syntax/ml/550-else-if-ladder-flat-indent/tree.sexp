(def
  (classify x)
  (if
    (first-threshold-check-predicate x)
    (first-branch-result-value x)
    (if
      (second-threshold-check-predicate x)
      (second-branch-result-value x)
      (if
        (third-threshold-check-predicate x)
        (third-branch-result-value x)
        (final-fallback-branch-result-value x)))))
