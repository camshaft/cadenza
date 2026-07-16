import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function PropertyTesting() {
  return (
    <article>
      <H1>Testing &amp; properties</H1>
      <Lede>
        A test in Cadenza is just a function you mark <C>@test</C>. There's no separate "property" concept
        to learn: give that test <em>parameters</em> and it becomes generative — the runner makes up inputs,
        runs it many times, and shrinks any failure to the smallest case.
      </Lede>

      <H2>A test is a marked function</H2>
      <P>
        Mark a zero-argument function <C>@test</C> and <C>cdz test</C> runs it. It passes unless it{" "}
        <em>traps</em> — so you assert with an <C>if</C> that traps on the bad branch:
      </P>
      <Note>
        <C>{`@test def two_plus_two() = if 2 + 2 == 4 then unit else trap("wrong")`}</C>
        <br />
        <C>cdz test</C> reports: <C>PASS two_plus_two</C>
      </Note>
      <P>
        That's an ordinary unit test. These are <em>declarations</em> the test runner finds — the guide's
        Run button, by contrast, evaluates a <C>main</C> and shows its value, so throughout this chapter the{" "}
        <em>runnable</em> panels show the <em>predicate</em> being computed, and the <C>cdz test</C>{" "}
        transcripts (in the grey boxes) show what the runner prints. The predicate is the real content; the
        annotation just hands it to the runner.
      </P>

      <H2>Parameters make it generative</H2>
      <P>
        Give a <C>@test</C> parameters and it turns into a <em>property</em> test: the runner{" "}
        <em>generates</em> values for those parameters — the generator is synthesized from the parameter{" "}
        <em>types</em>, nothing to write — runs the test many times (100 by default), and only passes if
        every trial does. A property is just a predicate that should hold for <em>all</em> inputs. Here's
        one — addition is commutative — computed live on a concrete pair:
      </P>
      <Runnable
        source={`(def (add-comm a b)
  (= (Int64.wrapping-add a b) (Int64.wrapping-add b a)))
(def (main) (if (add-comm 3 7) 1 0))`}
      />
      <P>
        Run gives <C>1</C> — the predicate holds for <C>3</C> and <C>7</C>. Mark that same predicate a{" "}
        <C>@test</C> with parameters and <C>cdz test</C> supplies the <C>a</C> and <C>b</C> for you, a
        hundred times over:
      </P>
      <Note>
        <C>{`@test def add_comm(a: Int64, b: Int64) =`}</C>
        <br />
        <C>{`  if Int64.wrapping-add(a, b) == Int64.wrapping-add(b, a) then unit else trap("not commutative")`}</C>
        <br />
        <C>cdz test</C> reports: <C>PASS add_comm (100 trials)</C>
      </Note>
      <P>
        You wrote the predicate; the runner wrote the generator. Scalars (<C>Int64</C>, <C>Bool</C>,{" "}
        <C>Float64</C>, and the rest) generate directly. Compound types generate too — a{" "}
        <C>(List Int64)</C> parameter is filled with generated lists, a record with generated fields, and so
        on — the compiler derives the generator from the type. Here a list property, computed on a concrete
        list, holds:
      </P>
      <Runnable
        source={`(def (len-nonneg xs) (>= (List.len xs) 0))
(def (main) (if (len-nonneg (list 1 2 3)) 1 0))`}
      />
      <Note>
        <C>{`@test def len_nonneg(xs: List(Int64)) = if List.len(xs) >= 0 then unit else trap("neg")`}</C>
        <br />
        <C>cdz test</C> reports: <C>PASS len_nonneg-gen (100 trials)</C> — the <C>-gen</C> marks the
        synthesized list generator.
      </Note>

      <H2>Failure shrinks to the smallest case</H2>
      <P>
        When a property fails, a giant random counterexample isn't much help — so the runner{" "}
        <em>shrinks</em> it, searching for the smallest input that still fails, and prints the seed to
        replay. Take a deliberately-wrong property, "every number is under 5". It holds at <C>4</C>:
      </P>
      <Runnable
        source={`(def (small n) (< n 5))
(def (main) (if (small 4) 1 0))`}
      />
      <P>
        — and fails at <C>5</C>:
      </P>
      <Runnable
        source={`(def (small n) (< n 5))
(def (main) (if (small 5) 1 0))`}
      />
      <P>
        Run the first and you get <C>1</C>, the second <C>0</C>. As a <C>@test</C>, the runner finds a
        failing input and shrinks it to exactly the boundary — <C>5</C>, the <em>smallest</em> number that
        breaks it, not whatever large value it first stumbled on:
      </P>
      <Note>
        <C>{`@test def small(n: Int64) = if n < 5 then unit else trap("too big")`}</C>
        <br />
        <C>cdz test</C> reports:
        <br />
        <C>FAIL small</C>
        <br />
        <C>{`  counterexample: small(5)   (seed 0; replay with --seed 0)`}</C>
      </Note>
      <P>
        The recorded seed makes the failure reproducible — a counterexample found in CI replays exactly on
        your machine, which is what makes it actionable.
      </P>

      <H2>Proving over a whole small domain, and tagging</H2>
      <P>
        Two refinements finish the surface. <C>@exhaustive</C> — on a test whose parameters are{" "}
        <em>bounded</em> (a <C>Bool</C>, a small integer like <C>UInt8</C>) — runs <em>every</em>{" "}
        combination instead of sampling, so a pass is a <em>proof</em> over that domain, not just evidence:
      </P>
      <Note>
        <C>{`@exhaustive def eq_symmetric(a: Bool, b: Bool) = if (a == b) == (b == a) then unit else trap("no")`}</C>
        <br />
        <C>cdz test</C> reports: <C>PASS eq_symmetric (exhaustive, 4 cases)</C> — all four <C>Bool</C> pairs.
      </Note>
      <P>
        And <C>@tag("…")</C> labels a test so you can run a subset — <C>cdz test --tag slow</C> runs only
        the tests you tagged <C>"slow"</C>. All three are ordinary annotations; there's no privileged
        testing layer, just functions the runner knows how to call.
      </P>

      <Why tenet="A test is a function; a property is a test with arguments">
        Cadenza doesn't add a property-testing DSL with its own generators and combinators. A test is a
        function marked <C>@test</C>; a <em>property</em> is that same idea with parameters, and the
        compiler synthesizes the generator from the types you already wrote — so there's nothing extra to
        learn or maintain. Under the hood the runner drives generation, shrinking, and a recorded seed (a
        real generative engine), but the surface you touch is just: write a predicate, mark it, and — if it
        takes arguments — the runner tries to break it for you.
      </Why>

      <H2>Your turn</H2>
      <P>
        These exercises work the <em>predicate</em> — the part you write. In a real test file you'd mark it{" "}
        <C>@test</C> and let <C>cdz test</C> generate the inputs; here you apply it to a concrete input and
        Run, the same computation the runner performs each trial.
      </P>
      <Exercise
        id="property-testing:1"
        prompt={
          <>
            Doubling then halving should return the original — a round-trip property. Fill the check so{" "}
            <C>round-trip</C> confirms it for <C>21</C>, giving <C>1</C>.
          </>
        }
        starter={`(def (round-trip n) (= (/ (* n 2) 2) ?))
(def (main) (if (round-trip 21) 1 0))`}
        solution={`(def (round-trip n) (= (/ (* n 2) 2) n))
(def (main) (if (round-trip 21) 1 0))`}
        expected="1"
        hint={
          <>
            The property is that doubling then halving gets you back where you started — so{" "}
            <C>(/ (* n 2) 2)</C> should equal <C>n</C>. Fill the blank with <C>n</C>.
          </>
        }
      />

      <Exercise
        id="property-testing:2"
        prompt={
          <>
            Find the boundary. The property <C>(&lt; n 3)</C> holds for small <C>n</C> and fails once{" "}
            <C>n</C> is big enough — fill the input that is the <em>smallest</em> failing case (what a
            shrinker would report), so the check gives <C>0</C>.
          </>
        }
        starter={`(def (under-3 n) (< n 3))
(def (main) (if (under-3 ?) 1 0))`}
        solution={`(def (under-3 n) (< n 3))
(def (main) (if (under-3 3) 1 0))`}
        expected="0"
        hint={
          <>
            <C>(&lt; n 3)</C> is true for <C>0, 1, 2</C> and false from <C>3</C> up, so the smallest failing
            input is <C>3</C> — and <C>(under-3 3)</C> is <C>false</C>, giving <C>0</C>.
          </>
        }
      />
    </article>
  );
}
