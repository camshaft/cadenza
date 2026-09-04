// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, Cadenza, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { AppLink, Ch } from "../../components/ChapterLink.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Effects() {
  return (
    <article>
      <H1>Effects & handlers</H1>
      <Lede>An effect lets a function ask for something like a random choice or the current setting or a way to bail out while leaving how that request is answered to whoever runs it.</Lede>
      <P>Most languages bake the answer into the function itself, so one that needs a random number calls the global generator and one that needs to give up throws an exception. Cadenza splits those two apart, because a function performs an operation by naming what it needs while a <C>handle</C> around it decides what performing that operation means, and the performing code never knows or cares who is listening. We'll build up from a handler sitting right next to the performance toward the case that matters, where a function performs an operation that some other function further out decides how to answer.</P>
      <H2>Declaring and performing an operation</H2>
      <P>An <C>effect</C> declares a named operation and its type. Here <C>Ask</C> has one operation, <C>ask</C>, that produces an <C>Int64</C>. The body performs it with <Cadenza ast="Y2R6YXN0AAEDGgoDQXNrCgNhc2sFAAAAAQACAQMAAQIBAQME" kind="expr">(Ask.ask)</Cadenza>, and the enclosing <C>handle</C> answers every performance by <C>resume</C>-ing with a value:</P>
      <Runnable
        source={`(effect Ask (op ask (-> Unit Int64)))

(def (main) (handle Ask unit ((ask () s (resume 42 s))) (Ask.ask)))`}
      />
      <P>The body is just <Cadenza ast="Y2R6YXN0AAEDGgoDQXNrCgNhc2sFAAAAAQACAQMAAQIBAQME" kind="expr">(Ask.ask)</Cadenza>. There's no <C>42</C> in it; the value came entirely from the handler. Read the arm as: when <C>ask</C> is performed, <C>resume</C> the performing code with <C>42</C>. (The <C>s</C> is the handler's state; we'll get to it, for now it's along for the ride.)</P>
      <Why tenet="A function says what it needs, not how it's met">Splitting <em>performing</em> from <em>handling</em> is the same move as returning an <C>Option</C> instead of crashing: it puts a decision into the open where it can be seen and changed. Code that performs <C>Ask.ask</C> works against any handler: one that returns a constant, one that reads a config, one that records every call for a test. The alternative, reaching out to a global or throwing an exception the caller can't see in the type, welds the answer to the question. An effect keeps them separable.</Why>
      <H2>The handler intercepts every performance</H2>
      <P>A handler isn't a one-time value; it answers <em>each</em> time the operation is performed. Here the body performs <C>Ask.ask</C> twice, and each one resumes with <C>20</C>:</P>
      <Runnable
        source={`(effect Ask (op ask (-> Unit Int64)))

(def (main) (handle Ask unit ((ask () s (resume 20 s))) (+ (Ask.ask) (Ask.ask))))`}
        id="two-asks"
      />
      <P>Two performances, each answered with <C>20</C>, summed to <C>40</C>.</P>
      <H2>A handler with state</H2>
      <P>That <C>s</C> is the handler's own state, threaded from one performance to the next. A <C>handle</C> seeds it (here <C>0</C>), each arm receives it, and <C>resume</C> takes both the value to hand back <em>and</em> the next state. So a counter that hands out <C>0</C>, then <C>1</C>, then <C>2</C>… is a three-line handler:</P>
      <Runnable
        source={`(effect Counter (op next (-> Unit Int64)))

(def
  (main)
  (handle Counter 0 ((next (u) s (resume s (+ s 1)))) (+ (Counter.next) (* 10 (Counter.next)))))`}
        id="counter"
      />
      <P>The first <C>next</C> resumes with the state <C>0</C> and bumps it to <C>1</C>; the second resumes with <C>1</C>. So the body computes <C>0 + 10 * 1</C> = <C>10</C>. The state never leaks out of the handler; the performing code just sees a sequence of numbers.</P>
      <H2>The performer and the handler can be far apart</H2>
      <P>So far the <C>handle</C> has wrapped the performance directly. But nothing says they have to sit in the same function, and this is where effects earn their keep. A function can perform an operation it never handles; the handler is supplied by whoever <em>calls</em> it. Here <C>gen</C> just performs <C>Bump.by</C> (it has no handler at all) and <C>main</C> decides what performing means, wrapped around its <em>call</em> to <C>gen</C>:</P>
      <Runnable
        source={`(effect Bump (op by (-> Int64 Int64)))

(def (gen) (Bump.by 41))

(def (main) (handle Bump unit ((by (n) s (resume (+ n 1) s))) (gen)))`}
      />
      <P><C>gen</C> reads as an ordinary function, yet it reaches out to a handler installed one level up the call stack. Resolution follows the <em>calls</em>, not the source layout: the performance in <C>gen</C> searches outward along the chain that led to it until it finds a <C>Bump</C> handler, <C>main</C>'s. That's what lets a leaf function deep in a program name what it needs and let the edge of the program decide how.</P>
      <Why tenet="A function says what it needs, not how it's met">This is dependency injection without the plumbing. <C>gen</C> doesn't take the answer as a parameter, doesn't import a global, doesn't know a handler exists; it just performs. The same <C>gen</C>, unchanged, behaves differently under a different caller's handler: one caller resumes with <C>n + 1</C>, another could resume with <C>0</C> for a test, another could count the calls. The dependency is threaded implicitly along the call chain and resolved at the nearest handler, the useful half of a global variable with none of the "who set this?" mystery.</Why>
      <H2>A getter/setter, shared across functions</H2>
      <P>Give an effect <em>two</em> operations and let the handler's state be real mutable state (a <C>get</C> and a <C>set</C>) and you have a store that any function can read and write, without a single one of them holding the data. Here <C>deposit</C> and <C>balance</C> are ordinary helpers; neither owns the balance. The <C>State</C> handler in <C>main</C> is the only thing that does, and it threads it through <C>s</C>:</P>
      <Runnable
        source={`(effect State (op get (-> Unit Int64)) (op set (-> Int64 Unit)))

(def (deposit (: n Int64)) (State.set (+ (State.get) n)))

(def (balance) (State.get))

(def
  (main)
  (handle
    State
    100
    ((get (u) s (resume s s)) (set (v) s (resume unit v)))
    (do (deposit 20) (deposit 5) (balance))))`}
        id="state"
      />
      <P>The account starts at <C>100</C>; two deposits push it to <C>125</C>. Read the arms as the store's implementation: <C>get</C> resumes with the current state and leaves it unchanged; <C>set</C> resumes with <C>unit</C> and makes its argument the new state. <C>deposit</C> and <C>balance</C> talk to that store through <C>State.get</C> / <C>State.set</C> as if it were ambient, but it isn't ambient at all. Swap in a handler that logs every <C>set</C>, or seeds a different opening balance, and not one line of <C>deposit</C> changes.</P>
      <H2>Doing work after resume</H2>
      <P>Every handler so far has <C>resume</C>d as its <em>last</em> act: it hands a value back and steps aside, and whatever the performing code computes flows straight out. But an arm can do work <em>after</em> the resumption returns. <C>resume</C> is an ordinary expression, so its result, the value the rest of the body eventually produces, is something the arm can keep computing with:</P>
      <Runnable
        source={`(effect Amb (op flip (-> Unit Int64)))

(def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (Amb.flip)))`}
        id="amb-tail"
      />
      <P>The arm resumes the performer with <C>10</C>, and here the body <em>is</em> that performance, so it reduces to <C>10</C>; then the arm's own <C>+ 1</C> wraps that result, and the answer is <C>11</C>. The <C>resume</C> plugs a value into the hole where <C>Amb.flip</C> was, and the arm gets to act on what comes back.</P>
      <P>That hole can have work around it too. If the body is <Cadenza ast="Y2R6YXN0AAEFCgErAAFkGgoDQW1iCgRmbGlwCAAAAAEAAgADAAQBAwIDBAEBBQEDAAEGBw==" kind="expr">(+ 100 (Amb.flip))</Cadenza>, the resumption re-runs <em>the whole rest of the body</em> with <C>10</C> in the hole, giving <C>110</C>, and only then does the arm's <C>+ 1</C> apply:</P>
      <Runnable
        source={`(effect Amb (op flip (-> Unit Int64)))

(def (main) (handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (+ 100 (Amb.flip))))`}
        id="amb-around"
      />
      <P>The result is <C>111</C>: <C>100 + 10</C> from re-reducing the body, then <C>+ 1</C> from the arm on the way out. This is what lets a handler <em>post-process</em> or <em>aggregate</em> a whole computation, logging a total, accumulating, transforming a result, rather than only feeding a value in. And it composes with state: each performance resumes with the advanced state, and the arm's surrounding work wraps every re-reduction:</P>
      <Runnable
        source={`(effect St (op tick (-> Unit Int64)))

(def (main) (handle St 0 ((tick (u) s (+ 100 (resume s (+ s 1))))) (+ (St.tick) (St.tick))))`}
        id="st-tick"
      />
      <P>The first <C>tick</C> reads the initial state <C>0</C> and the second reads the advanced <C>1</C>; each is wrapped by the arm's <C>+ 100</C>, so they add up to <C>201</C>. Tail resume answers and steps aside; non-tail resume answers, then acts on what came back.</P>
      <H2>A handler that doesn't resume: bailing out</H2>
      <P>A handler can also <em>not</em> resume at all. If an arm just returns a value, the whole <C>handle</C> expression becomes that value and the rest of the body is abandoned: an early exit, no exceptions required. Here <C>Bail.bail</C> hands its argument straight out, so the <C>+ 100</C> never runs:</P>
      <Runnable
        source={`(effect Bail (op bail (-> Int64 Int64)))

(def (main) (handle Bail 0 ((bail (n) s n)) (+ (Bail.bail 7) 100)))`}
        id="bail"
      />
      <P>The result is <C>7</C>, not <C>107</C>: performing <C>bail</C> jumped out of the addition entirely. This is how you'd write "stop and return this now": the same shape as an exception, but it's just a handler choosing not to resume.</P>
      <P>The bail works even right beside another effect, not just a plain value. Here the body adds a resumptive <Cadenza ast="Y2R6YXN0AAEDGgoBRQoFb3RoZXIFAAAAAQACAQMAAQIBAQME" kind="expr">(E.other)</Cadenza> and an abortive <Cadenza ast="Y2R6YXN0AAEEGgoBRQoEYmFpbAABBQYAAAABAAIBAwABAgADAQIDBAU=" kind="expr">(E.bail 5)</Cadenza>, and performing <C>bail</C> still exits the whole <C>handle</C>:</P>
      <Runnable
        source={`(effect E (op other (-> Int64)) (op bail (-> Int64 Int64)))

(def (main) (handle E 0 ((other () s (resume s (+ s 1))) (bail (v) s v)) (+ (E.other) (E.bail 5))))`}
        id="e-bail"
      />
      <P>The result is <C>5</C>. <Cadenza ast="Y2R6YXN0AAEDGgoBRQoFb3RoZXIFAAAAAQACAQMAAQIBAQME" kind="expr">(E.other)</Cadenza> ran first and drew the seed <C>0</C>, then <Cadenza ast="Y2R6YXN0AAEEGgoBRQoEYmFpbAABBQYAAAABAAIBAwABAgADAQIDBAU=" kind="expr">(E.bail 5)</Cadenza> bailed, so the pending <C>+</C> never finishes. The effect <Cadenza ast="Y2R6YXN0AAEDGgoBRQoFb3RoZXIFAAAAAQACAQMAAQIBAQME" kind="expr">(E.other)</Cadenza> already performed isn't undone, though: a bail-out abandons the work still <em>pending</em>, not what already happened. And the bail need not sit at the top of the body; it exits from wherever it runs, even from inside a conditional like <Cadenza ast="Y2R6YXN0AAEHCgJpZgoBYxoKAUUKBGJhaWwAAQUAAAoAAAABAAIAAwAEAQMCAwQABQECBQYABgEEAAEHCAk=" kind="expr">(if c (E.bail 5) 0)</Cadenza> that only some runs reach.</P>
      <Note>These handlers are <em>one-shot</em>: each performance resumes at most once, so they compile down to ordinary control flow: no captured continuations, no runtime machinery. Handling something the program can't discharge itself (real input, the clock) is delegated to the host at the program's edge, and shows up in its manifest. That's how effects stay honest about what a program actually does.</Note>
      <H2>A guard must be side-effect-free</H2>
      <P>There's one place a perform is <em>not</em> allowed: a <Ch to="/pattern-matching"> match-arm </Ch> <em>guard</em>. A guard is a boolean decision the pattern engine may evaluate <em>speculatively or repeatedly</em>, or skip entirely when an earlier arm wins, so it has no well-defined "run exactly once, in this order" the way an arm body does. Performing an effect there would mean an effect with no defined schedule, so the compiler rejects it outright. Here a guard performs <C>Ask.ask</C>, and the program declines:</P>
      <Runnable
        source={`(effect Ask (op ask (-> Unit Int64)))

(def
  (main)
  (handle Ask unit ((ask () s (resume 5 s))) (match 3 ((guard x (< x (Ask.ask))) 1) (_ 0))))`}
        expect="error"
      />
      <P>The error is <C>CDZ0407</C>, and its message names the fix: <C>a guard must be side-effect-free — an effect is performed in this guard, which the pattern engine may evaluate speculatively or repeatedly; lift it to a `let` evaluated once before the `match` and guard on the bound value</C>. That is exactly the repair: perform the effect <em>once</em>, before the <C>match</C>, bind the result, and let the guard read the bound value, which is pure. Same logic, now with a defined evaluation:</P>
      <Runnable
        source={`(effect Ask (op ask (-> Unit Int64)))

(def
  (main)
  (handle
    Ask
    unit
    ((ask () s (resume 5 s)))
    (let ((limit (Ask.ask))) (match 3 ((guard x (< x limit)) 1) (_ 0)))))`}
        id="guard-fix"
      />
      <P>Now the perform happens exactly once (<C>limit</C> is <C>5</C>), the guard compares against the bound value, and since <C>3 &lt; 5</C> the first arm fires and the result is <C>1</C>. The rule is narrow: an effect is welcome in a scrutinee, in an arm body, anywhere with a defined order, just not in the guard condition itself, where "how many times, in what order" isn't a question the pattern engine can answer.</P>
      <H2>Why this matters: mock now, real later</H2>
      <P>This is what makes effects useful in real programs. Because the performer doesn't know who answers, the <em>same</em> code runs against a test mock or a real external service just by choosing a different handler. Picture a step of an agent loop: <C>turn</C> performs <C>Model.converse</C>, a call to a language model. It names what it needs; it doesn't reach out itself. Run it under a <em>mock</em> handler that echoes the query back, and under a <em>different</em> handler that answers differently, and you get two behaviours from the same <C>turn</C>:</P>
      <Runnable
        source={`(effect Model (op converse (-> Int64 Int64)))

(def (turn) (Model.converse 5))

(def
  (main)
  (+
    (handle Model 0 ((converse (q) s (resume q s))) (turn))
    (handle Model 0 ((converse (q) s (resume (* q 10) s))) (turn))))`}
        id="model"
      />
      <P>The first handler resumes with the query unchanged (<C>5</C>), the second with ten times it (<C>50</C>), so the sum is <C>55</C>, from one unchanged <C>turn</C>. In a real program the mock is your unit test and the "different" handler is the one wired to the actual model at the edge; the loop's logic never mentions either. That's effects as an I/O <em>boundary</em>: the body performs, and swapping the handler swaps the whole outside world, with no dependency injection, no mocking framework, just a different <C>handle</C>.</P>
      <P>And the whole <em>loop</em> is just this. A real agent runs turns until it's done: each turn asks the model (<C>Model.converse</C>) and dispatches a tool (<C>Tools.dispatch</C>), accumulating a result, bounded by a fuel counter so it terminates. Every one of those is a performed operation; the loop itself is ordinary recursion, and the handlers supply the outside world. Here a three-step run against mock handlers accumulates <C>3 + 2 + 1 = 6</C>:</P>
      <Runnable
        source={`(effect Model (op converse (-> Int64 Int64)))

(effect Tools (op dispatch (-> Int64 Int64)) (op done (-> Int64 Int64)))

(def
  (run (: fuel Int64) (: acc Int64))
  (if
    (= fuel 0)
    (Tools.done acc)
    (if (= (Model.converse fuel) 0) (Tools.done acc) (run (- fuel 1) (+ acc (Tools.dispatch fuel))))))

(def
  (main)
  (handle
    Model
    0
    ((converse (q) s (resume q s)))
    (handle Tools 0 ((dispatch (a) s (resume a s)) (done (a) s (resume a s))) (run 3 0))))`}
      />
      <P>The <C>run</C> loop is pure Cadenza: no HTTP, no SDK, no knowledge of what a model or a tool <em>is</em>. Point the two handlers at a real language model and a real tool dispatcher (the program does this at its edge, where the effects show up in its manifest) and the identical loop drives a live agent, so the logic you test with mocks is byte-for-byte the logic that runs in production, and the handler is the only thing that changed.</P>
      <P>The <AppLink to="/notebook"> notebook </AppLink> turns this idea into a live document: drag a slider and every dependent cell recomputes, the surrounding context deciding what a value means, made interactive.</P>
      <H2>Your turn</H2>
      <Exercise
        id="effects:1"
        prompt={<>Make the handler resume <C>ask</C> with <C>41</C>, so <Cadenza ast="Y2R6YXN0AAEFCgErGgoDQXNrCgNhc2sAAQEIAAAAAQACAAMBAwECAwEBBAAEAQMABQYH" kind="expr">(+ (Ask.ask) 1)</Cadenza> gives <C>42</C>.</>}
        starter={`(effect Ask (op ask (-> Unit Int64)))

(def (main) (handle Ask unit ((ask () s (resume ? s))) (+ (Ask.ask) 1)))`}
        solution={`(effect Ask (op ask (-> Unit Int64)))

(def (main) (handle Ask unit ((ask () s (resume 41 s))) (+ (Ask.ask) 1)))`}
        expected="42"
        hint={<>The handler decides the value <C>ask</C> produces: resume with <C>41</C>.</>}
      />
      <Exercise
        id="effects:2"
        prompt={<>Finish the <C>bail</C> arm so it hands its argument out, making the answer <C>5</C> (not <C>15</C>).</>}
        starter={`(effect Bail (op bail (-> Int64 Int64)))

(def (main) (handle Bail 0 ((bail (n) s ?)) (+ (Bail.bail 5) 10)))`}
        solution={`(effect Bail (op bail (-> Int64 Int64)))

(def (main) (handle Bail 0 ((bail (n) s n)) (+ (Bail.bail 5) 10)))`}
        expected="5"
        hint={<>Return <C>n</C> from the arm without <C>resume</C>: that bails out, skipping the <C>+ 10</C>.</>}
      />
      <Exercise
        id="effects:3"
        prompt={<>Now the state. This <C>next</C> hands each caller the current count, so summing three performances should give <C>0 + 1 + 2 = 3</C>. It only counts up if each arm resumes with the <em>next</em> state, so fill in the second <C>resume</C> argument to make the counter advance by one.</>}
        starter={`(effect Counter (op next (-> Unit Int64)))

(def
  (main)
  (handle
    Counter
    0
    ((next (u) s (resume s ?)))
    (+ (Counter.next) (+ (Counter.next) (Counter.next)))))`}
        solution={`(effect Counter (op next (-> Unit Int64)))

(def
  (main)
  (handle
    Counter
    0
    ((next (u) s (resume s (+ s 1))))
    (+ (Counter.next) (+ (Counter.next) (Counter.next)))))`}
        expected="3"
        hint={<><C>resume</C> takes two things: the value handed back (here <C>s</C>, the current count) and the state the <em>next</em> performance will see. Advance it by one with <Cadenza ast="Y2R6YXN0AAEDCgErCgFzAAEBBAAAAAEAAgEDAAECAw==" kind="expr">(+ s 1)</Cadenza>. (Leave it as plain <C>s</C> and every call sees <C>0</C>, summing to <C>0</C>.)</>}
      />
      <P>An effect splits <em>what</em> a function needs from <em>how</em> that need is met: the caller supplies the handler. The next chapter constrains the other end: not what a function needs, but what it <em>promises</em>. With <Ch to="/contracts"> design by contract </Ch> you state a function's assumptions and guarantees as checks the compiler enforces at its boundary.</P>
    </article>
  );
}
