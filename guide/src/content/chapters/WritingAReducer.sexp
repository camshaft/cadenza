(chapter
  (slug "writing-a-reducer")
  (title "Writing a reducer")
  (pillar "platform")
  (section "Writing a reducer")
  (blurb "The capstone: author a real agent-harness reducer in Cadenza, a pure apply that folds events to effect-requests, with host state reached through effect-and-binding, compiled to a component the kernel loads.")
  (lede "The " (link (slug "platform-overview") " platform ") " runs an agent by folding its event log through a " (em "reducer") ", a pure function the kernel calls once per event. Until now that reducer was written in Rust; now you write it in Cadenza, compile it to a WebAssembly " (em "component") ", and the kernel drives it. This chapter builds one from the empty reducer up, and it's where the language's " (link (slug "effects") "effects") " and the kernel's fold model meet in a real program.")
  (h2 "The shape of a reducer")
  (p "A reducer is one function, " (c "apply") ". The kernel calls it with the " (em "content-type") " of the event (a family name and a version), the event's " (em "payload") " as optional bytes, and, when the event is a " (em "resume") " (an effect completing), the " (em "resumes") " bytes. It returns a list of " (em "effect-requests") ", the outbound work the kernel should schedule. That's the whole contract: events in, effect-requests out, no hidden state.")
  (note "apply(content-type, payload, resumes) → list of effect-requests " (br) " content-type = a record of a family name + a version; payload / resumes = optional bytes")
  (p "The effect-kinds and the effect-request are ordinary Cadenza types, a closed sum and a record, the same declarations you'd write for any data. Here they are, matching the kernel's interface:")
  (runnable
    (source (type EffectKind (Shell) (Http) (Model) (Now) (Timer) (Emit))

(type
  EffectRequest
  (Mk
    (Record
      (: kind EffectKind)
      (: target String)
      (: payload (Option Bytes))
      (: correlation (Option Bytes)))))

(def (main) ((. EffectKind Http)))))
  (h2 "The empty reducer")
  (p "The smallest reducer that works ignores every input and asks for nothing. It's total (it can't brick the agent) and it's the honest starting point: a fold that advances the log and requests no effects. Here " (c "apply") " returns the empty list for every event, and a " (c "main") " exercises it on a sample message and shows exactly what came back, the list of effect-requests itself:")
  (runnable
    (source (type EffectKind (Shell) (Http) (Model) (Now) (Timer) (Emit))

(type
  EffectRequest
  (Mk
    (Record
      (: kind EffectKind)
      (: target String)
      (: payload (Option Bytes))
      (: correlation (Option Bytes)))))

(def
  (apply
    (: ct (Record (: family String) (: version (UInt 32))))
    (: payload (Option Bytes))
    (: resumes (Option Bytes)))
  (: #list() (List EffectRequest)))

(def (main) (apply #record((= family "message") (= version 1)) (None) (None)))))
  (p "The result is " (cdz "#list()") ", the empty list: no effects, for any event. To turn this program into something the kernel can run, you compile it as a " (em "component") " bound to the reducer interface, naming that interface on the command line:")
  (note "cdz compile reducer.cdz --target wasm --component-name cadenza:agent-kernel/fold")
  (p "The " (c "--component-name") " flag is the whole difference between an ordinary program and a component the kernel can load: it exports " (c "apply") " through the named " (c "cadenza:agent-kernel/fold") " interface the kernel expects, so the kernel can marshal each event into the call and read the effect-requests back out. (That marshalling and the component export are the kernel's side of the boundary; the examples on this page run " (c "apply") "'s logic directly, the way you'd unit-test it, rather than through the component interface.)")
  (p "That reducer interface is one interface of a WIT " (em "world") ", the target the component compiles against: it " (em "exports") " the " (c "fold") " interface (this " (c "apply") ") and " (em "imports") " the host interfaces it uses, like the key-value store below. A module can name that world inline with a " (link (slug "modules") " " (c "world") " declaration ") " (its example is exactly a " (c "world Reducer") "), so the compile target is spelled out in the source instead of supplied separately.")
  (h2 "Emitting an effect")
  (p "A useful reducer asks for work. An effect-request names a " (em "kind") " (which capability), a " (em "target") ", an optional " (em "payload") ", and an optional " (em "correlation") " tag the kernel echoes back on the resume so you can match a completion to the request that caused it. This reducer returns a single " (c "Http") " request on every event; the " (c "main") " reaches into the returned list and shows the " (em "target") " the reducer asked the kernel to fetch:")
  (runnable
    (source (type EffectKind (Shell) (Http) (Model) (Now) (Timer) (Emit))

(type
  EffectRequest
  (Mk
    (Record
      (: kind EffectKind)
      (: target String)
      (: payload (Option Bytes))
      (: correlation (Option Bytes)))))

(def
  (apply
    (: ct (Record (: family String) (: version (UInt 32))))
    (: payload (Option Bytes))
    (: resumes (Option Bytes)))
  (:
    #list(((. EffectRequest Mk)
        #record((= kind ((. EffectKind Http)))
          (= target "https://ok.host/x")
          (= payload (None))
          (= correlation (Some ((. String to-bytes) "step-1"))))))
    (List EffectRequest)))

(def
  (main)
  (match
    ((. List at) (apply #record((= family "message") (= version 1)) (None) (None)) 0)
    ((Some (Mk r)) (. r target))
    ((None) "no effect")))))
  (p "The result is " (c "\"https://ok.host/x\"") ", the exact target the reducer chose. A returned effect-request is " (em "declarative") ": the reducer doesn't perform the HTTP call, it hands the kernel a description of the work and returns. The kernel schedules it, and when it completes, calls " (c "apply") " again with the " (em "resumes") " field set to that same " (c "correlation") " tag, echoed back verbatim. The tag is the reducer's " (em "own") " token, not a kernel-assigned id, so a later " (c "apply") " whose " (c "resumes") " matches the token you chose is the completion of the request you made. That guest-chosen token is the only resume mechanism; a " (c "resumes") " of " (c "None") " means \"not a resume\", an inbound message, not the answer to an earlier request. That's how a pure fold reaches the outside world without ever blocking inside the reducer.")
  (h2 "Deciding by event")
  (p "Real reducers branch on what arrived. The content-type's " (c "family") " and the " (c "resumes") " field are enough to distinguish an inbound message from an effect completing. A common shape: on a resume, stop (the effect you asked for is done, don't cascade); on a fresh message, act; otherwise ignore. This reducer emits one " (c "Http") " request for a message and nothing for a resume or any other family. Here the " (c "main") " hands it a fresh " (em "message") ", so it takes the acting branch, and shows what the reducer decided to do, the target of the request it emits:")
  (runnable
    (source (type EffectKind (Shell) (Http) (Model) (Now) (Timer) (Emit))

(type
  EffectRequest
  (Mk
    (Record
      (: kind EffectKind)
      (: target String)
      (: payload (Option Bytes))
      (: correlation (Option Bytes)))))

(def
  (apply
    (: ct (Record (: family String) (: version (UInt 32))))
    (: payload (Option Bytes))
    (: resumes (Option Bytes)))
  (:
    (match
      resumes
      ((Some _) #list())
      ((None)
        (if
          (= (. ct family) "message")
          #list(((. EffectRequest Mk)
              #record((= kind ((. EffectKind Http)))
                (= target "https://ok.host/x")
                (= payload (None))
                (= correlation (None)))))
          #list())))
    (List EffectRequest)))

(def
  (decision ct payload resumes)
  (match
    ((. List at) (apply ct payload resumes) 0)
    ((Some (Mk r)) (. r target))
    ((None) "(no effect)")))

(def (main) (decision #record((= family "message") (= version 1)) (None) (None)))))
  (p "On a message the reducer dispatches to the acting branch and the result is " (c "\"https://ok.host/x\"") ", the target it emitted. Change " (c "main") " to hand " (c "apply") " a resume instead, " (c "decision(&#123; family = \"result\", version = 1 &#125;, None(), Some(String.to-bytes(\"tok\")))") ", and the first " (c "match") " arm fires and it renders " (c "\"(no effect)\"") ": a completion doesn't cascade. The reducer's whole behaviour is this one " (c "match") " on the event, which is exactly what makes it easy to test.")
  (h2 "Reading session state: an effect through a binding")
  (p "An effect-request is for work the kernel does " (em "later") ". But a reducer often needs something " (em "now") ", most commonly its own session's key-value store, to read a counter before deciding. That's a host capability, and Cadenza reaches it the same way it reaches any capability: an " (strong "effect") " declares the operations, and a " (strong "binding") " says which interface the host provides them through. The same construct that calls a host store would call a peer component, the program performs the effect and the binding routes it.")
  (p "You " (em "declare") " the interface as an effect, its operations and their types, then " (em "bind") " it to the host interface by name:")
  (note "effect Kv = | get : Bytes -&gt; Option(Bytes) | put : (Bytes, Bytes) -&gt; Unit " (br) " bind(Kv, \"cadenza:agent-kernel/kv\")")
  (p "One syntax note if you write this out: a multi-argument operation like " (c "put") " uses the arrow-form " (c "`->`(Bytes, Bytes, Unit)") " in real source, not " (c "(Bytes, Bytes) -> Unit") ". The tuple-looking form above reads clearly but means " (em "one") " tuple argument; the arrow-form is what makes " (c "put") " a genuine two-argument op (the fixture " (c "reducer_b3.cdz") " writes it that way).")
  (p "With that in place, the reducer " (em "performs") " the operations inside a " (c "host") " block, the scope where the effect may be used. On a message it reads the old counter, writes the new one, and returns an outbound effect-request, all in one expression whose value is that returned list:")
  (note "host Kv in ( " (br) " &nbsp;&nbsp;Kv.put(key, next-count(Kv.get(key))); " (br) " &nbsp;&nbsp;[ http-request() ] " (br) " )")
  (p "Keep the two reaches distinct, because this reducer uses both. A " (em "performed") " effect ( " (c "Kv.get") " / " (c "Kv.put") ") is a capability the reducer uses inline and waits on, state it needs during the fold. A " (em "returned") " effect-request (the " (c "Http") " in the list) is declarative work the kernel schedules after " (c "apply") " returns. One happens now, inside the call; the other happens later, outside it. The " (c "host … in") " block and the returned list are the two halves of that story.")
  (note "performed effect (Kv.get/put) → used inline, reducer waits, host answers now " (br) " returned effect-request (the Http in the list) → declarative, kernel schedules it after apply returns")
  (h2 "Testing a reducer")
  (p "Because " (c "apply") " is a pure function of its inputs, its behaviour is testable without a kernel at all. Cadenza's " (c "@test") " definitions run on the compiler directly: a test is a nullary " (c "def") " that returns " (c "unit") " to pass and traps to fail, and a plain compile ignores them, so tests live in the same file as the reducer without bloating the component it emits.")
  (note "@test def emits_nothing_on_resume() = " (br) " &nbsp;&nbsp;if List.len(apply(&#123; family = \"result\", version = 1 &#125;, None(), Some(...))) == 0 " (br) " &nbsp;&nbsp;then unit else trap(\"a resume must not cascade\")")
  (p "Run them with " (c "cdz test") " over the reducer's directory and each " (c "@test") " reports pass or fail. The branches that don't perform a host effect, the resume-stops and non-message-ignored invariants, test standalone exactly like the runnable examples above; the branch that performs " (c "Kv") " needs the kernel's host handler, so its behaviour is covered by the kernel's end-to-end test instead. The general rule: pure branches test on the compiler, host-effect branches test against the kernel.")
  (h2 "How the kernel drives it")
  (p "Put the pieces together and the loop is the whole platform in miniature. Every idea the earlier chapters introduced shows up in this one reducer. The " (link (slug "platform-overview") " overview ") " 's pure fold " (em "is") " " (c "apply") ": the kernel appends an event, calls " (c "apply") " with the content-type, payload, and any resume bytes, and reads back the effect-requests. The reducer holds nothing between calls, exactly the " (link (slug "platform-state") " events &amp; state ") " model, so its only memory is the session state it reads and writes through " (c "Kv") ". Each outbound reach is an effect through a binding, which is " (link (slug "platform-safety") " doing things safely ") " made concrete: the effect row is the reducer's permission list. And the schedule-then-resume rhythm is the " (link (slug "platform-execution") " execution model ") " , an append wakes the reducer, nothing polls, and the correlation tag threads a completion back to the request that caused it.")
  (p "That's a complete agent-harness reducer in Cadenza: a typed " (c "apply") " that folds events to effect-requests, host capabilities reached through effect-and-binding, and test coverage on the pure core, compiled to a component the kernel loads by interface name. The language you learned in the first pillar is exactly the language an agent runs on."))
