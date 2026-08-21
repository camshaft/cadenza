; The deliver contract — the schema of the one built-in contract the kernel recognizes (design/cadenza-platform.md
; section 4). This file is the SOURCE OF TRUTH for the schema and is real Cadenza: `cargo xtask codegen` typechecks
; it with the compiler and generates `src/deliver_schema.rs` (the type declarations, as Cadenza-AST builder calls)
; from it. Because the generation is compiler-checked, the schema cannot drift into something that is not valid
; Cadenza. Re-run `cargo xtask codegen` after editing this file; `cargo xtask codegen --check` fails if the
; committed generated file is stale. The compiler is a codegen-time tool only — it never enters the shipped crate.
;
; The `def`s below are not part of the generated schema (codegen extracts only the `type` declarations). They are
; the CONFORMANCE PROOF: each builds a fully-literal envelope value and ascribes it against `Deliver-Envelope`, so
; the codegen typecheck fails if a value of the shape `Deliver::encode` marshals could not be a value of the schema
; type. This is what ties the runtime encoder to the schema — the operator's requirement that the built value
; type-ascribe against the schema and the compiler accept it.
(module deliver
  ; A delivery either failed or, if it targeted a request whose answer routes back, carries that answer.
  (type Error (Timeout) (MissingHandler))
  (type Result (Ok Bytes) (Err Error))

  ; The three kinds of event that may be injected into a reducer's log — one per entry point the target folds it
  ; through (on_message / on_response / on_notification). Every hash and opaque payload crosses as `Bytes`.
  (type Event
    (Message (Record (id Bytes) (reducer Bytes) (host Bytes) (payload Bytes) (token Bytes)))
    (Response (Record (id Bytes) (token Bytes) (result Result)))
    (Notification (Record (id Bytes) (payload Bytes))))

  ; The envelope (the contract's INPUT): deliver `event` into the log of the reducer named by `target`.
  (type Deliver-Envelope
    (Deliver (Record (target Bytes) (event Event))))

  ; The outcome (the contract's OUTPUT): delivered, or failed with a reason.
  (type Deliver-Outcome
    (Delivered)
    (Failed Bytes))

  ; --- conformance proofs: a fully-literal value of each event kind, ascribed against the schema type ---
  (def (message-value)
    (: (Deliver-Envelope.Deliver
          (record (target b"target")
                  (event (Event.Message
                           (record (id b"id") (reducer b"reducer") (host b"host")
                                   (payload b"payload") (token b"token"))))))
       Deliver-Envelope))

  (def (_response-ok-value)
    (: (Deliver-Envelope.Deliver
          (record (target b"target")
                  (event (Event.Response
                           (record (id b"id") (token b"token") (result (Result.Ok b"answer")))))))
       Deliver-Envelope))

  (def (_response-err-value)
    (: (Deliver-Envelope.Deliver
          (record (target b"target")
                  (event (Event.Response
                           (record (id b"id") (token b"token") (result (Result.Err (Error.Timeout))))))))
       Deliver-Envelope))

  (def (_notification-value)
    (: (Deliver-Envelope.Deliver
          (record (target b"target")
                  (event (Event.Notification (record (id b"id") (payload b""))))))
       Deliver-Envelope))

  (export message-value))
