; Deliver a message to the reducer-echo guest and confirm it ECHOES: it emits a request on the SAME
; contract with the SAME payload (its on_message copies contract+payload straight back out, §3). This
; exercises the delivery path (§4/§9) end to end through a real wasm component under bach.
;
; v2a assertion (render-level): the rendered observation log shows reducer-echo receiving the delivered
; message and EMITTING a request whose payload preview is the echoed payload. The exact token==own-id +
; byte-exact contract/payload assertion needs the full-fidelity STRUCTURED log (v-platform-itest #3007) —
; the human render is lossy (contract = 10-char prefix, a non-UTF-8 token renders as "<N bytes>").
;
; `(= program "reducer-echo")` is resolved to the nix wasm store path by the harness-run framework. The
; contract is any valid 33-byte id (tag byte + 32 digest) — reducer-echo routes nothing on it, it just
; echoes. The payload is short UTF-8 so its preview is greppable.
("record"
  (= system "$system")
  (= blobs ("list"
    ("record" (= name "$system") (= bytes b"itest:no-system-reducer"))
    ("record" (= name "reducer-echo") (= program "reducer-echo"))))
  (= spawns ("list"
    ("record" (= name "reducer-echo") (= blob "reducer-echo"))))
  (= deliver ("list"
    ("record"
      (= target "reducer-echo")
      (= message ("record"
        (= contract b"\x01ABCDEFGHIJKLMNOPQRSTUVWXYZ012345")
        (= payload b"ECHOPAYLOAD")))))))
