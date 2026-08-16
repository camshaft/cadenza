# try-with-RUNTIME-operand flip candidates (re-triaged tick 1002)

NOT a regression: `?`/try lowers only CONSTANT operands (staged decline: "the boundary break for a
runtime operand is the next brick"). Constant-operand try computes and is pinned (14-effects 12883/3151).
Everything here DECLINES today and flips to pins when the runtime-operand brick lands:
- ao1 (Ast in Option through try), aomin1/3/4 (scalar/def/let minimal ladder)
- tb1 (try-unwrapped List.at with trie-lookup index) — was misdated 2026-08-15, renamed dir
Watch: any rcdzc commit touching the try lowering / "next brick" message.
