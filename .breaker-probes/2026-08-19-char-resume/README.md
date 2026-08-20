# char-resume — op resumes an (Option Char) computed from the state
## pych2 — letter() resumes (Char.from-int (+ 97 s)); body matches Some/None + Char.to-int. Model 98099/97098. PASS x3.
Char (inside Option) round-trips through the resume seam; code point advances per dispatch. (API: Char.from-int -> (Option Char); Char.to-int; Char literal #\a.) Promotable.
