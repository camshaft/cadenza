// A faithful message -> step echo reducer: fold a delivered message and emit exactly one request on the
// same contract carrying the same payload (the event round-trips), keeping the session running. Structural
// records inline; Outcome is the one nominal sum. No host imports.
type Outcome =
  | Continue
  | Close(Record(schema: Bytes, reason: Bytes))

def on-message(
  msg: Record(contract: Bytes,
              sender: Record(reducer: Bytes, host: Bytes),
              payload: Bytes,
              token: Bytes)
) -> Record(requests: List(Record(contract: Bytes,
                                  payload: Bytes,
                                  token: Bytes,
                                  deadlineNanos: Option(UInt64))),
            outcome: Outcome) =
  { requests = List.push(([] : List(Record(contract: Bytes,
                                           payload: Bytes,
                                           token: Bytes,
                                           deadlineNanos: Option(UInt64)))),
                         { contract = msg.contract,
                           payload = msg.payload,
                           token = msg.token,
                           deadlineNanos = None }),
    outcome = Outcome.Continue }

export { on-message }
