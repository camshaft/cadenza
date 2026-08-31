// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Ch } from "../../components/ChapterLink.tsx";
import { Runnable } from "../../components/Runnable.tsx";

export default function WritingAReducer() {
  return (
    <article>
      <H1>Writing a reducer</H1>
      <Lede>The <Ch to="/platform-overview"> platform </Ch> runs an agent by folding its event log through a <em>reducer</em>, a pure function the kernel calls once per event. Until now that reducer was written in Rust; now you write it in Cadenza, compile it to a WebAssembly <em>component</em>, and the kernel drives it. This chapter builds one from the empty reducer up, and it's where the language's <Ch to="/effects">effects</Ch> and the kernel's fold model meet in a real program.</Lede>
      <H2>The shape of a reducer</H2>
      <P>A reducer is one function, <C>apply</C>. The kernel calls it with the <em>content-type</em> of the event (a family name and a version), the event's <em>payload</em> as optional bytes, and, when the event is a <em>resume</em> (an effect completing), the <em>resumes</em> bytes. It returns a list of <em>effect-requests</em>, the outbound work the kernel should schedule. That's the whole contract: events in, effect-requests out, no hidden state.</P>
      <Note>apply(content-type, payload, resumes) → list of effect-requests <br /> content-type = a record of a family name + a version; payload / resumes = optional bytes</Note>
      <P>The effect-kinds and the effect-request are ordinary Cadenza types, a closed sum and a record, the same declarations you'd write for any data. Here they are, matching the kernel's interface:</P>
      <Runnable
        source={`(type EffectKind (Shell) (Http) (Model) (Now) (Timer) (Emit))

(type
  EffectRequest
  (Mk
    (Record
      (: kind EffectKind)
      (: target String)
      (: payload (Option Bytes))
      (: correlation (Option Bytes)))))

(def (main) (EffectKind.Http))`}
      />
      <H2>The empty reducer</H2>
      <P>The smallest reducer that works ignores every input and asks for nothing. It's total (it can't brick the agent) and it's the honest starting point: a fold that advances the log and requests no effects. Here <C>apply</C> returns the empty list for every event, and a <C>main</C> exercises it on a sample message and shows exactly what came back, the list of effect-requests itself:</P>
      <Runnable
        source={`(type EffectKind (Shell) (Http) (Model) (Now) (Timer) (Emit))

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

(def (main) (apply #record((= family "message") (= version 1)) (None) (None)))`}
      />
      <P>The result is <C>[]</C>, the empty list: no effects, for any event. To turn this program into something the kernel can run, you compile it as a <em>component</em> bound to the reducer interface, naming that interface on the command line:</P>
      <Note>cdz compile reducer.cdz --target wasm --component-name cadenza:agent-kernel/fold</Note>
      <P>The <C>--component-name</C> flag is the whole difference between an ordinary program and a component the kernel can load: it exports <C>apply</C> through the named <C>cadenza:agent-kernel/fold</C> interface the kernel expects, so the kernel can marshal each event into the call and read the effect-requests back out. (That marshalling and the component export are the kernel's side of the boundary; the examples on this page run <C>apply</C>'s logic directly, the way you'd unit-test it, rather than through the component interface.)</P>
      <P>That reducer interface is one interface of a WIT <em>world</em>, the target the component compiles against: it <em>exports</em> the <C>fold</C> interface (this <C>apply</C>) and <em>imports</em> the host interfaces it uses, like the key-value store below. A module can name that world inline with a <Ch to="/modules"> <C>world</C> declaration </Ch> (its example is exactly a <C>world Reducer</C>), so the compile target is spelled out in the source instead of supplied separately.</P>
      <H2>Emitting an effect</H2>
      <P>A useful reducer asks for work. An effect-request names a <em>kind</em> (which capability), a <em>target</em>, an optional <em>payload</em>, and an optional <em>correlation</em> tag the kernel echoes back on the resume so you can match a completion to the request that caused it. This reducer returns a single <C>Http</C> request on every event; the <C>main</C> reaches into the returned list and shows the <em>target</em> the reducer asked the kernel to fetch:</P>
      <Runnable
        source={`(type EffectKind (Shell) (Http) (Model) (Now) (Timer) (Emit))

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
    #list((EffectRequest.Mk
        #record((= kind (EffectKind.Http))
          (= target "https://ok.host/x")
          (= payload (None))
          (= correlation (Some ((. String to-bytes) "step-1"))))))
    (List EffectRequest)))

(def
  (main)
  (match
    (List.at (apply #record((= family "message") (= version 1)) (None) (None)) 0)
    ((Some (Mk r)) r.target)
    ((None) "no effect")))`}
      />
      <P>The result is <C>"https://ok.host/x"</C>, the exact target the reducer chose. A returned effect-request is <em>declarative</em>: the reducer doesn't perform the HTTP call, it hands the kernel a description of the work and returns. The kernel schedules it, and when it completes, calls <C>apply</C> again with the <em>resumes</em> field set to that same <C>correlation</C> tag, echoed back verbatim. The tag is the reducer's <em>own</em> token, not a kernel-assigned id, so a later <C>apply</C> whose <C>resumes</C> matches the token you chose is the completion of the request you made. That guest-chosen token is the only resume mechanism; a <C>resumes</C> of <C>None</C> means "not a resume", an inbound message, not the answer to an earlier request. That's how a pure fold reaches the outside world without ever blocking inside the reducer.</P>
      <H2>Deciding by event</H2>
      <P>Real reducers branch on what arrived. The content-type's <C>family</C> and the <C>resumes</C> field are enough to distinguish an inbound message from an effect completing. A common shape: on a resume, stop (the effect you asked for is done, don't cascade); on a fresh message, act; otherwise ignore. This reducer emits one <C>Http</C> request for a message and nothing for a resume or any other family. Here the <C>main</C> hands it a fresh <em>message</em>, so it takes the acting branch, and shows what the reducer decided to do, the target of the request it emits:</P>
      <Runnable
        source={`(type EffectKind (Shell) (Http) (Model) (Now) (Timer) (Emit))

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
          (= ct.family "message")
          #list((EffectRequest.Mk
              #record((= kind (EffectKind.Http))
                (= target "https://ok.host/x")
                (= payload (None))
                (= correlation (None)))))
          #list())))
    (List EffectRequest)))

(def
  (decision ct payload resumes)
  (match (List.at (apply ct payload resumes) 0) ((Some (Mk r)) r.target) ((None) "(no effect)")))

(def (main) (decision #record((= family "message") (= version 1)) (None) (None)))`}
      />
      <P>On a message the reducer dispatches to the acting branch and the result is <C>"https://ok.host/x"</C>, the target it emitted. Change <C>main</C> to hand <C>apply</C> a resume instead, <C>decision(&#123; family = "result", version = 1 &#125;, None(), Some(String.to-bytes("tok")))</C>, and the first <C>match</C> arm fires and it renders <C>"(no effect)"</C>: a completion doesn't cascade. The reducer's whole behaviour is this one <C>match</C> on the event, which is exactly what makes it easy to test.</P>
      <H2>Reading session state: an effect through a binding</H2>
      <P>An effect-request is for work the kernel does <em>later</em>. But a reducer often needs something <em>now</em>, most commonly its own session's key-value store, to read a counter before deciding. That's a host capability, and Cadenza reaches it the same way it reaches any capability: an <strong>effect</strong> declares the operations, and a <strong>binding</strong> says which interface the host provides them through. The same construct that calls a host store would call a peer component, the program performs the effect and the binding routes it.</P>
      <P>You <em>declare</em> the interface as an effect, its operations and their types, then <em>bind</em> it to the host interface by name:</P>
      <Note>effect Kv = | get : Bytes -&gt; Option(Bytes) | put : (Bytes, Bytes) -&gt; Unit <br /> bind(Kv, "cadenza:agent-kernel/kv")</Note>
      <P>One syntax note if you write this out: a multi-argument operation like <C>put</C> uses the arrow-form <C>{"`->`(Bytes, Bytes, Unit)"}</C> in real source, not <C>{"(Bytes, Bytes) -> Unit"}</C>. The tuple-looking form above reads clearly but means <em>one</em> tuple argument; the arrow-form is what makes <C>put</C> a genuine two-argument op (the fixture <C>reducer_b3.cdz</C> writes it that way).</P>
      <P>With that in place, the reducer <em>performs</em> the operations inside a <C>host</C> block, the scope where the effect may be used. On a message it reads the old counter, writes the new one, and returns an outbound effect-request, all in one expression whose value is that returned list:</P>
      <Note>host Kv in ( <br /> &nbsp;&nbsp;Kv.put(key, next-count(Kv.get(key))); <br /> &nbsp;&nbsp;[ http-request() ] <br /> )</Note>
      <P>Keep the two reaches distinct, because this reducer uses both. A <em>performed</em> effect ( <C>Kv.get</C> / <C>Kv.put</C>) is a capability the reducer uses inline and waits on, state it needs during the fold. A <em>returned</em> effect-request (the <C>Http</C> in the list) is declarative work the kernel schedules after <C>apply</C> returns. One happens now, inside the call; the other happens later, outside it. The <C>host … in</C> block and the returned list are the two halves of that story.</P>
      <Note>performed effect (Kv.get/put) → used inline, reducer waits, host answers now <br /> returned effect-request (the Http in the list) → declarative, kernel schedules it after apply returns</Note>
      <H2>Testing a reducer</H2>
      <P>Because <C>apply</C> is a pure function of its inputs, its behaviour is testable without a kernel at all. Cadenza's <C>@test</C> definitions run on the compiler directly: a test is a nullary <C>def</C> that returns <C>unit</C> to pass and traps to fail, and a plain compile ignores them, so tests live in the same file as the reducer without bloating the component it emits.</P>
      <Note>@test def emits_nothing_on_resume() = <br /> &nbsp;&nbsp;if List.len(apply(&#123; family = "result", version = 1 &#125;, None(), Some(...))) == 0 <br /> &nbsp;&nbsp;then unit else trap("a resume must not cascade")</Note>
      <P>Run them with <C>cdz test</C> over the reducer's directory and each <C>@test</C> reports pass or fail. The branches that don't perform a host effect, the resume-stops and non-message-ignored invariants, test standalone exactly like the runnable examples above; the branch that performs <C>Kv</C> needs the kernel's host handler, so its behaviour is covered by the kernel's end-to-end test instead. The general rule: pure branches test on the compiler, host-effect branches test against the kernel.</P>
      <H2>How the kernel drives it</H2>
      <P>Put the pieces together and the loop is the whole platform in miniature. Every idea the earlier chapters introduced shows up in this one reducer. The <Ch to="/platform-overview"> overview </Ch> 's pure fold <em>is</em> <C>apply</C>: the kernel appends an event, calls <C>apply</C> with the content-type, payload, and any resume bytes, and reads back the effect-requests. The reducer holds nothing between calls, exactly the <Ch to="/platform-state"> events &amp; state </Ch> model, so its only memory is the session state it reads and writes through <C>Kv</C>. Each outbound reach is an effect through a binding, which is <Ch to="/platform-safety"> doing things safely </Ch> made concrete: the effect row is the reducer's permission list. And the schedule-then-resume rhythm is the <Ch to="/platform-execution"> execution model </Ch> , an append wakes the reducer, nothing polls, and the correlation tag threads a completion back to the request that caused it.</P>
      <P>That's a complete agent-harness reducer in Cadenza: a typed <C>apply</C> that folds events to effect-requests, host capabilities reached through effect-and-binding, and test coverage on the pure core, compiled to a component the kernel loads by interface name. The language you learned in the first pillar is exactly the language an agent runs on.</P>
    </article>
  );
}
