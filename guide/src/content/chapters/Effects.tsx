import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Effects() {
  return (
    <article>
      <H1>Effects &amp; handlers</H1>
      <Lede>
        A function can ask for something — a random choice, the current setting, a way to bail out — and
        leave <em>how</em> it's answered to whoever runs it. That's an effect.
      </Lede>

      <P>
        Most languages bake the answers in: a function that needs a random number calls the global
        random generator; one that needs to give up throws an exception. Cadenza splits the two apart. A
        function <em>performs</em> an operation — it names what it needs — and a <C>handle</C> around it
        decides what performing means. The performing code doesn't know or care who's listening. We'll
        build up from a handler right next to the performance to the real payoff: a function that performs
        an operation some <em>other</em> function, further out, decides how to answer.
      </P>

      <H2>Declaring and performing an operation</H2>
      <P>
        An <C>effect</C> declares a named operation and its type. Here <C>Ask</C> has one operation,{" "}
        <C>ask</C>, that produces an <C>Int64</C>. The body performs it with <C>(Ask.ask)</C>, and the
        enclosing <C>handle</C> answers every performance by <C>resume</C>-ing with a value:
      </P>
      <Runnable
        source={`(effect Ask (op ask (-> Unit Int64)))
(def (main)
  (handle Ask unit
    ((ask () s (resume 42 s)))
    (Ask.ask)))`}
      />
      <P>
        The body is just <C>(Ask.ask)</C>. There's no <C>42</C> in it — the value came entirely from the
        handler. Read the arm as: when <C>ask</C> is performed, <C>resume</C> the performing code with{" "}
        <C>42</C>. (The <C>s</C> is the handler's state; we'll get to it — for now it's along for the
        ride.)
      </P>

      <Why tenet="A function says what it needs, not how it's met">
        Splitting <em>performing</em> from <em>handling</em> is the same move as returning an{" "}
        <C>Option</C> instead of crashing: it puts a decision into the open where it can be seen and
        changed. Code that performs <C>Ask.ask</C> works against any handler — one that returns a
        constant, one that reads a config, one that records every call for a test. The alternative —
        reaching out to a global, or throwing an exception the caller can't see in the type — welds the
        answer to the question. An effect keeps them separable.
      </Why>

      <H2>The handler intercepts every performance</H2>
      <P>
        A handler isn't a one-time value — it answers <em>each</em> time the operation is performed. Here
        the body performs <C>Ask.ask</C> twice, and each one resumes with <C>20</C>:
      </P>
      <Runnable
        source={`(effect Ask (op ask (-> Unit Int64)))
(def (main)
  (handle Ask unit
    ((ask () s (resume 20 s)))
    (+ (Ask.ask) (Ask.ask))))`}
      />
      <P>Two performances, each answered with <C>20</C>, summed to <C>40</C>.</P>

      <H2>A handler with state</H2>
      <P>
        That <C>s</C> is the handler's own state, threaded from one performance to the next. A <C>handle</C>{" "}
        seeds it (here <C>0</C>), each arm receives it, and <C>resume</C> takes both the value to hand back{" "}
        <em>and</em> the next state. So a counter that hands out <C>0</C>, then <C>1</C>, then <C>2</C>… is
        a three-line handler:
      </P>
      <Runnable
        source={`(effect Counter (op next (-> Unit Int64)))
(def (main)
  (handle Counter 0
    ((next (u) s (resume s (+ s 1))))
    (+ (Counter.next) (* 10 (Counter.next)))))`}
      />
      <P>
        The first <C>next</C> resumes with the state <C>0</C> and bumps it to <C>1</C>; the second resumes
        with <C>1</C>. So the body computes <C>0 + 10 * 1</C> = <C>10</C>. The state never leaks out of the
        handler — the performing code just sees a sequence of numbers.
      </P>

      <H2>The performer and the handler can be far apart</H2>
      <P>
        So far the <C>handle</C> has wrapped the performance directly. But nothing says they have to sit
        in the same function — and this is where effects earn their keep. A function can perform an
        operation it never handles; the handler is supplied by whoever <em>calls</em> it. Here <C>gen</C>{" "}
        just performs <C>Bump.by</C> — it has no handler at all — and <C>main</C> decides what performing
        means, wrapped around its <em>call</em> to <C>gen</C>:
      </P>
      <Runnable
        source={`(effect Bump (op by (-> Int64 Int64)))
(def (gen) (Bump.by 41))
(def (main)
  (handle Bump unit
    ((by (n) s (resume (+ n 1) s)))
    (gen)))`}
      />
      <P>
        <C>gen</C> reads as an ordinary function, yet it reaches out to a handler installed one level up
        the call stack. Resolution follows the <em>calls</em>, not the source layout: the performance in{" "}
        <C>gen</C> searches outward along the chain that led to it until it finds a <C>Bump</C> handler —{" "}
        <C>main</C>'s. That's what lets a leaf function deep in a program name what it needs and let the
        edge of the program decide how.
      </P>

      <Why tenet="A function says what it needs, not how it's met">
        This is dependency injection without the plumbing. <C>gen</C> doesn't take the answer as a
        parameter, doesn't import a global, doesn't know a handler exists — it just performs. The same{" "}
        <C>gen</C>, unchanged, behaves differently under a different caller's handler: one caller resumes
        with <C>n + 1</C>, another could resume with <C>0</C> for a test, another could count the calls.
        The dependency is threaded implicitly along the call chain and resolved at the nearest handler —
        the useful half of a global variable with none of the "who set this?" mystery.
      </Why>

      <H2>A getter/setter, shared across functions</H2>
      <P>
        Give an effect <em>two</em> operations and let the handler's state be real mutable state — a{" "}
        <C>get</C> and a <C>set</C> — and you have a store that any function can read and write, without a
        single one of them holding the data. Here <C>deposit</C> and <C>balance</C> are ordinary helpers;
        neither owns the balance. The <C>State</C> handler in <C>main</C> is the only thing that does, and
        it threads it through <C>s</C>:
      </P>
      <Runnable
        source={`(effect State
  (op get (-> Unit Int64))
  (op set (-> Int64 Unit)))
(def (deposit (: n Int64))
  (State.set (+ (State.get) n)))
(def (balance) (State.get))
(def (main)
  (handle State 100
    ((get (u) s (resume s s))
     (set (v) s (resume unit v)))
    (do (deposit 20) (deposit 5) (balance))))`}
      />
      <P>
        The account starts at <C>100</C>; two deposits push it to <C>125</C>. Read the arms as the store's
        implementation: <C>get</C> resumes with the current state and leaves it unchanged; <C>set</C>{" "}
        resumes with <C>unit</C> and makes its argument the new state. <C>deposit</C> and <C>balance</C>{" "}
        talk to that store through <C>State.get</C> / <C>State.set</C> as if it were ambient — but it isn't
        ambient at all. Swap in a handler that logs every <C>set</C>, or seeds a different opening balance,
        and not one line of <C>deposit</C> changes.
      </P>

      <H2>A handler that doesn't resume: bailing out</H2>
      <P>
        A handler doesn't have to <C>resume</C>. If an arm just returns a value, the whole <C>handle</C>{" "}
        expression becomes that value and the rest of the body is abandoned — an early exit, no exceptions
        required. Here <C>Bail.bail</C> hands its argument straight out, so the <C>+ 100</C> never runs:
      </P>
      <Runnable
        source={`(effect Bail (op bail (-> Int64 Int64)))
(def (main)
  (handle Bail 0
    ((bail (n) s n))
    (+ (Bail.bail 7) 100)))`}
      />
      <P>
        The result is <C>7</C>, not <C>107</C>: performing <C>bail</C> jumped out of the addition entirely.
        This is how you'd write "stop and return this now" — the same shape as an exception, but it's just
        a handler choosing not to resume.
      </P>

      <Note>
        These handlers are <em>one-shot</em>: each performance resumes at most once, so they compile down to
        ordinary control flow — no captured continuations, no runtime machinery. Handling something the
        program can't discharge itself (real input, the clock) is delegated to the host at the program's
        edge, and shows up in its manifest — that's how effects stay honest about what a program actually
        does.
      </Note>

      <H2>Your turn</H2>
      <Exercise
        id="effects:1"
        prompt={<>Make the handler resume <C>ask</C> with <C>41</C>, so <C>(+ (Ask.ask) 1)</C> gives <C>42</C>.</>}
        starter={`(effect Ask (op ask (-> Unit Int64)))
(def (main)
  (handle Ask unit
    ((ask () s (resume ? s)))
    (+ (Ask.ask) 1)))`}
        solution={`(effect Ask (op ask (-> Unit Int64)))
(def (main)
  (handle Ask unit
    ((ask () s (resume 41 s)))
    (+ (Ask.ask) 1)))`}
        expected="42"
        hint={<>The handler decides the value <C>ask</C> produces — resume with <C>41</C>.</>}
      />

      <Exercise
        id="effects:2"
        prompt={<>Finish the <C>bail</C> arm so it hands its argument out, making the answer <C>5</C> (not <C>15</C>).</>}
        starter={`(effect Bail (op bail (-> Int64 Int64)))
(def (main)
  (handle Bail 0
    ((bail (n) s ?))
    (+ (Bail.bail 5) 10)))`}
        solution={`(effect Bail (op bail (-> Int64 Int64)))
(def (main)
  (handle Bail 0
    ((bail (n) s n))
    (+ (Bail.bail 5) 10)))`}
        expected="5"
        hint={<>Return <C>n</C> from the arm without <C>resume</C> — that bails out, skipping the <C>+ 10</C>.</>}
      />

      <Exercise
        id="effects:3"
        prompt={<>
          Now the state. This <C>next</C> hands each caller the current count, so summing three
          performances should give <C>0 + 1 + 2 = 3</C>. It only counts up if each arm resumes with the{" "}
          <em>next</em> state — fill in the second <C>resume</C> argument so the counter advances by one.
        </>}
        starter={`(effect Counter (op next (-> Unit Int64)))
(def (main)
  (handle Counter 0
    ((next (u) s (resume s ?)))
    (+ (Counter.next) (+ (Counter.next) (Counter.next)))))`}
        solution={`(effect Counter (op next (-> Unit Int64)))
(def (main)
  (handle Counter 0
    ((next (u) s (resume s (+ s 1))))
    (+ (Counter.next) (+ (Counter.next) (Counter.next)))))`}
        expected="3"
        hint={<>
          <C>resume</C> takes two things: the value handed back (here <C>s</C>, the current count) and
          the state the <em>next</em> performance will see. Advance it by one — <C>(+ s 1)</C>. (Leave it
          as plain <C>s</C> and every call sees <C>0</C>, summing to <C>0</C>.)
        </>}
      />
    </article>
  );
}
