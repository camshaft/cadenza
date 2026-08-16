(case "rb3 read of an UNTERMINATED byte literal declines — the b-prefix joins the reader-totality family"
  (input  (read "b\"hi"))
  (declines))
