(case "ap3 control: print of runtime-SELECTED constant tree (branch picks which constant)"
  (input  (do
            (def (main (: n Int64))
              (String.scalar-len (print (if (> n 0) (Ast.Int 5N) (Ast.Name "xy")))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64)))
