(fold xs zero (fn (acc x) (match x ((Some v) (+ acc v)) ((None _) acc))))
