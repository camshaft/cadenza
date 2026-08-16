(case "ab6 text round-trip of an Ast.Bytes carrying NUL and 0xff (escaped spelling reads back raw)"
  (input  (= (read (print (Ast.Bytes b"\x00\xff"))) (Ast.Bytes b"\x00\xff")))
  (output (: true Bool)))
