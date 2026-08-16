(case "rb4 read of a byte literal INSIDE a compound: (f b\"hi\") parses the Bytes leaf in place"
  (input  (match (read "(f b\"hi\")")
            ((Ast.List els)
              (match (List.at els 1)
                ((Option.Some (Ast.Bytes b)) (Bytes.len b))
                (_ -2)))
            (_ -1)))
  (output (: 2 Int64)))
