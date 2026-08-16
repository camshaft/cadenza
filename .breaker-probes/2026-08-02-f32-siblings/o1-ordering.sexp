(case "o1 f32 ordering: two same-f32-bits literals are not < each other (runtime)"
  (input  (do
            (def (main (: c Bool))
              (if (< (: (if c 0.3 9.9) Float32) (: 0.30000001192092896 Float32)) 1 0))
            (export main)))
  (call   main (: true Bool)) (output (: 0 Int64)))
