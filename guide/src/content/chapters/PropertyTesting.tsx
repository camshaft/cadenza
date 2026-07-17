import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function PropertyTesting() {
  return (
    <article>
      <H1>Testing &amp; properties</H1>
      <Lede>
        A test in Cadenza is just a function you mark <C>@test</C>. There's no separate testing DSL: the
        runner finds every <C>@test</C>, runs it, and reports a pass or a fail. The panels below run for
        real, right here — each shows ✓ or ✗ per test.
      </Lede>

      <H2>A test is a marked function</H2>
      <P>
        Mark a zero-argument function <C>@test</C> and the runner calls it. It <em>passes</em> unless it{" "}
        <em>traps</em> — so you assert by trapping on the bad case. A tiny assert helper (<C>assert</C>,{" "}
        <C>assert-eq</C>, <C>assert-ne</C>, each just an <C>if</C> that traps on failure) is in scope in
        these panels. Here two tests, both passing — press Run and you'll see two ✓:
      </P>
      <Runnable
        mode="test"
        source={`(@ test (def (two-plus-two-is-four)
  (assert-eq (+ 2 2) 4 "arithmetic is broken")))
(@ test (def (addition-commutes)
  (assert-eq (Int64.wrapping-add 3 7) (Int64.wrapping-add 7 3) "not commutative")))`}
      />
      <P>
        No <C>main</C>, no wrapper — the source <em>is</em> the test defs, and the runner discovers and runs
        them exactly the way <C>cdz test</C> does on your machine. A pass means the function returned
        without trapping; that's the whole contract.
      </P>

      <H2>A failing test shows why</H2>
      <P>
        When a test traps, the runner reports it as a failure with the trap message. Here one test passes
        and one deliberately fails — Run it and you'll see a ✓ and a ✗ with its reason:
      </P>
      <Runnable
        mode="test"
        expect="error"
        source={`(@ test (def (four-is-under-five)
  (assert (< 4 5) "4 should be under 5")))
(@ test (def (five-is-under-five)
  (assert (< 5 5) "5 is not under 5 — this test is meant to fail")))`}
      />
      <P>
        The second test's condition is false, so <C>assert</C> traps and the runner marks it ✗ with the
        message you gave. That message is the whole point of a good assert: when a test fails, it tells you{" "}
        <em>what</em> broke, not just that something did.
      </P>

      <H2>Tests are ordinary code</H2>
      <P>
        A <C>@test</C> is a normal function, so it can do anything a function can — build a list, call your
        own definitions, check a result. Here a test defines a helper and asserts a property of it:
      </P>
      <Runnable
        mode="test"
        source={`(def (double x) (* 2 x))
(@ test (def (double-then-halve-round-trips)
  (assert-eq (/ (double 21) 2) 21 "double then halve should return the original")))
(@ test (def (list-length-is-nonneg)
  (assert (>= (List.len (list 1 2 3)) 0) "length can't be negative")))`}
      />
      <P>
        There's nothing special about a test beyond the <C>@test</C> mark — it's a function the runner knows
        to call. That's the whole testing story: no framework, no assertion library baked into the language,
        just functions that trap when an expectation is violated.
      </P>

      <H2>Parameters make it generative</H2>
      <P>
        Give a <C>@test</C> <em>parameters</em> and it becomes a <em>property</em> test: the runner{" "}
        <em>generates</em> inputs for those parameters — the generator is synthesized from the parameter{" "}
        <em>types</em>, nothing to write — runs the test many times (100 by default), and passes only if
        every trial does. On failure it <em>shrinks</em> the counterexample to the smallest input that still
        fails and prints a seed to replay. A property is just a test that should hold for <em>all</em>{" "}
        inputs:
      </P>
      <Note>
        <C>{`@test def add_comm(a: Int64, b: Int64) =`}</C>
        <br />
        <C>{`  assert_eq(Int64.wrapping-add(a, b), Int64.wrapping-add(b, a), "not commutative")`}</C>
        <br />
        <C>cdz test</C> reports: <C>PASS add_comm (100 trials)</C>.
      </Note>
      <P>
        You wrote the predicate; the runner wrote the generator. Scalars generate directly; compound types
        (a <C>(List Int64)</C>, a record, a user sum) generate too — the compiler derives the generator from
        the type. And a deliberately-wrong property shows the shrink: <C>{`@test def small(n: Int64) = assert(n < 5, "too big")`}</C>{" "}
        fails and the runner reports <C>counterexample: small(5)</C> — the <em>smallest</em> failing input,
        not whatever large value it first stumbled on, with a seed to reproduce it.
      </P>
      <Note>
        The runnable panels above use <em>nullary</em> tests, which run live in the browser. Parameterized
        property tests run in the full <C>cdz test</C> runner (they generate and shrink over many trials);
        the in-browser panels show them as deferred, so the generative examples here are shown as{" "}
        <C>cdz test</C> transcripts rather than live runs.
      </Note>

      <H2>Proving a small domain, and tagging</H2>
      <P>
        Two refinements round out the surface. <C>@exhaustive</C> — on a test whose parameters are{" "}
        <em>bounded</em> (a <C>Bool</C>, a narrow integer) — runs <em>every</em> combination instead of
        sampling, so a pass is a <em>proof</em> over that domain: <C>{`@exhaustive def or_symmetric(a: Bool, b: Bool) = …`}</C>{" "}
        checks all four <C>Bool</C> pairs. And <C>@tag("slow")</C> labels a test so{" "}
        <C>cdz test --tag slow</C> runs just that subset. All three are ordinary annotations — no privileged
        testing layer, just functions the runner knows how to call.
      </P>

      <Why tenet="A test is a function; a property is a test with arguments">
        Cadenza doesn't add a property-testing DSL with its own generators and combinators. A test is a
        function marked <C>@test</C>; a <em>property</em> is that same idea with parameters, and the compiler
        synthesizes the generator from the types you already wrote — so there's nothing extra to learn. The
        runner drives generation, shrinking, and a recorded seed under the hood, but the surface you touch is
        just: write a function, assert what should hold, mark it — and, if it takes arguments, the runner
        tries to break it for you.
      </Why>

      <H2>Your turn</H2>
      <P>
        A test's core is the <em>predicate</em> it asserts. These exercises compute that predicate directly
        (returning <C>1</C> when it holds) — the exact check a <C>@test</C> body performs each time the
        runner calls it.
      </P>
      <Exercise
        id="property-testing:1"
        prompt={
          <>
            Doubling then halving should return the original — the property a round-trip test asserts. Fill
            the value so the check holds for <C>21</C>, giving <C>1</C>.
          </>
        }
        starter={`(def (round-trips n) (= (/ (* n 2) 2) ?))
(def (main) (if (round-trips 21) 1 0))`}
        solution={`(def (round-trips n) (= (/ (* n 2) 2) n))
(def (main) (if (round-trips 21) 1 0))`}
        expected="1"
        hint={
          <>
            Doubling then halving gets you back where you started, so <C>(/ (* n 2) 2)</C> equals <C>n</C>.
            Fill the blank with <C>n</C>; a <C>@test</C> would <C>assert-eq</C> these two.
          </>
        }
      />

      <Exercise
        id="property-testing:2"
        prompt={
          <>
            The length of a three-element list is exactly <C>3</C> — the property a length test asserts.
            Fill the comparison value so the check gives <C>1</C>.
          </>
        }
        starter={`(def (len-is xs n) (= (List.len xs) n))
(def (main) (if (len-is (list 10 20 30) ?) 1 0))`}
        solution={`(def (len-is xs n) (= (List.len xs) n))
(def (main) (if (len-is (list 10 20 30) 3) 1 0))`}
        expected="1"
        hint={
          <>
            The list has three elements, so its length is <C>3</C>. Fill <C>3</C> and the check holds — the
            same equality a <C>@test</C> would <C>assert-eq</C>.
          </>
        }
      />
    </article>
  );
}
