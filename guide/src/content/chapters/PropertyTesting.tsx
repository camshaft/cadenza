import { H1, Lede, H2, P, C } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function PropertyTesting() {
  return (
    <article>
      <H1>Property-based testing</H1>
      <Lede>
        Instead of checking one example at a time, state a <em>property</em> — something that should hold
        for <em>every</em> input — and let generated inputs try to break it. There's no new construct:
        a generator is a function, a property is a predicate, and both are ordinary Cadenza.
      </Lede>

      <H2>Generation is seeded and reproducible</H2>
      <P>
        A generator turns a <em>seed</em> into a value. The classic one is a linear congruential step —
        multiply, add, wrap. Because it's a pure function of its seed, the same seed always gives the same
        value, so a failing run is reproducible: record the seed and you can replay it exactly.
      </P>
      <Runnable
        source={`(let ((next (fn (s)
              (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005)
                                  1442695040888963407))))
  (= (next 42) (next 42)))`}
      />
      <P>
        Same seed, same draw — <C>true</C>. Reproducibility only earns its keep if <em>different</em> seeds
        explore <em>different</em> inputs, though; a generator that ignored its seed would be useless. Two
        distinct seeds give two distinct values, so <C>=</C> is <C>false</C>:
      </P>
      <Runnable
        source={`(let ((next (fn (s)
              (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005)
                                  1442695040888963407))))
  (= (next 1) (next 2)))`}
      />

      <H2>Bounding a generator to a range</H2>
      <P>
        A raw draw covers the whole <C>Int64</C> range. To generate in a smaller range — say a die roll in{" "}
        <C>0 .. 64</C> — mask the low bits with <C>&</C>. <C>(& v 63)</C> keeps the bottom six bits, so
        every draw lands in range no matter the seed:
      </P>
      <Runnable
        source={`(def (next (: s Int64))
  (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
(def (roll (: s Int64)) (& (next s) 63))
(def (main)
  (let ((v (roll 12345)))
    (if (>= v 0) (< v 64) false)))`}
      />
      <P>
        The drawn value satisfies its bound — <C>true</C>. That's a generator respecting a refinement: it
        produces only admissible values, which is what lets a property assume its inputs are well-formed.
      </P>

      <H2>A property, and whether it has teeth</H2>
      <P>
        A property is just a predicate that should hold for every input. A load-bearing one is{" "}
        <em>permutation invariance</em>: a commutative fold gives the same answer whatever order its inputs
        arrive in. Summing <C>(list 1 2 3)</C> and a shuffle <C>(list 3 1 2)</C> agrees:
      </P>
      <Runnable
        source={`(def (sum (: xs (List Int64)))
  (match xs
    ((list) 0)
    ((list h .. t) (+ h (sum t)))))
(def (main)
  (if (= (sum (list 1 2 3)) (sum (list 3 1 2))) 1 0))`}
      />
      <P>
        The two orderings agree, so the check gives <C>1</C>. A property is only meaningful if it can also{" "}
        <em>fail</em> — if it rejects the behaviors it's meant to catch. Order invariance has teeth: a
        computation that depends on order, like "keep the first argument", does <em>not</em> satisfy it —{" "}
        <C>(first-wins 3 7)</C> is <C>3</C> but <C>(first-wins 7 3)</C> is <C>7</C>, so the property is{" "}
        <C>false</C> (the check gives <C>0</C>):
      </P>
      <Runnable
        source={`(def (first-wins (: a Int64) (: b Int64)) a)
(def (main)
  (if (= (first-wins 3 7) (first-wins 7 3)) 1 0))`}
      />

      <H2>Shrinking to the smallest failure</H2>
      <P>
        When a property fails, a big random counterexample isn't much help — you want the <em>smallest</em>{" "}
        input that still fails. That's shrinking: scan candidates upward from <C>0</C> while the property
        holds, and stop at the first one that fails. For the property <C>x &lt; 4</C>, the search reports{" "}
        <C>4</C> — the minimal violating input:
      </P>
      <Runnable
        source={`(def (p (: x Int64)) (< x 4))
(def (search (: cand Int64) (: fuel Int64))
  (if (= fuel 0) cand
    (if (p cand) (search (+ cand 1) (- fuel 1)) cand)))
(def (main) (search 0 100))`}
      />
      <P>
        The <C>fuel</C> argument bounds the scan so it always terminates — even if the property never fails
        in range, the search runs out of fuel and returns rather than looping forever. A shrinker is total
        by construction.
      </P>

      <Why tenet="A property is ordinary code, not a testing DSL">
        Property testing here is a <em>pattern</em>, not a feature. A generator is a function from a seed;
        a property is a predicate; shrinking is a bounded search — all written in the same language you
        test. Nothing was added to support it, and nothing hides what's happening: you can read the
        generator, replay a seed, and step the shrinker. It's the same instinct as the rest of Cadenza —
        reach for the pieces already there rather than a special-purpose layer.
      </Why>

      <H2>Your turn</H2>
      <Exercise
        id="property-testing:1"
        prompt={
          <>
            Bound a draw to <C>0 .. 8</C> by masking the low three bits. Fill the mask so every draw lands
            in range — the check gives <C>1</C>.
          </>
        }
        starter={`(def (next (: s Int64))
  (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
(def (roll (: s Int64)) (& (next s) ?))
(def (main)
  (let ((v (roll 999)))
    (if (< v 8) 1 0)))`}
        solution={`(def (next (: s Int64))
  (Int64.wrapping-add (Int64.wrapping-mul s 6364136223846793005) 1442695040888963407))
(def (roll (: s Int64)) (& (next s) 7))
(def (main)
  (let ((v (roll 999)))
    (if (< v 8) 1 0)))`}
        expected="1"
        hint={
          <>
            To keep the low three bits, mask with <C>7</C> — that's <C>2³ − 1 = 0b111</C>, so the result is
            always <C>0 .. 8</C>. (A mask of <C>15</C> would allow up to 15 and the check could fail.)
          </>
        }
      />

      <Exercise
        id="property-testing:2"
        prompt={
          <>
            Shrink to the minimal failure of the property <C>x &lt; 3</C>. The search scans upward from{" "}
            <C>0</C> while the property holds; fill the base value it starts from so it reports the smallest
            failing input, <C>3</C>.
          </>
        }
        starter={`(def (p (: x Int64)) (< x 3))
(def (search (: cand Int64) (: fuel Int64))
  (if (= fuel 0) cand
    (if (p cand) (search (+ cand 1) (- fuel 1)) cand)))
(def (main) (search ? 100))`}
        solution={`(def (p (: x Int64)) (< x 3))
(def (search (: cand Int64) (: fuel Int64))
  (if (= fuel 0) cand
    (if (p cand) (search (+ cand 1) (- fuel 1)) cand)))
(def (main) (search 0 100))`}
        expected="3"
        hint={
          <>
            Shrinking looks for the <em>smallest</em> failing input, so the scan starts at <C>0</C> and
            counts up. With <C>p x = x &lt; 3</C> holding for 0, 1, 2, the first failure is <C>3</C>.
          </>
        }
      />
    </article>
  );
}
