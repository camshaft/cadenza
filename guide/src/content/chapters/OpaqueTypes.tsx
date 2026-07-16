import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function OpaqueTypes() {
  return (
    <article>
      <H1>Opaque types</H1>
      <Lede>
        A module can export a type's <em>name</em> while keeping its <em>constructor</em> private. The
        result is an <em>opaque</em> type (also called an abstract data type): code elsewhere can hold and
        pass its values and call the module's functions on them, but can't build or take one apart. That's
        how an invariant becomes unbreakable.
      </Lede>

      <P>
        Exporting a type and exporting its <em>constructors</em> are two independent decisions. A bare{" "}
        <C>(export Color)</C> publishes only the type's <em>handle</em> — enough to name it, but not to
        construct it. Adding <C>(export Color.*)</C> (or naming a specific variant) publishes the
        constructors too, making the type <em>concrete</em>. So opacity is the <em>default</em> of
        exporting a type; concreteness is opt-in. Withholding the constructor is what makes a type opaque.
      </P>

      <H2>The smart constructor</H2>
      <P>
        Why hold a constructor back? Because then the <em>only</em> way to make a value is to go through a
        function the module <em>does</em> export — a "smart constructor" that can enforce an invariant every
        value must satisfy. Establish the invariant once, there, and no caller can ever produce a value that
        skips it. Here a <C>Counter</C> that must never be negative: <C>mk</C> clamps, and <C>value</C>{" "}
        reads it back out:
      </P>
      <Runnable
        source={`(type Counter (MkCounter Int64))
(def (mk (: n Int64)) (if (< n 0) (Counter.MkCounter 0) (Counter.MkCounter n)))
(def (value (: c Counter)) (match c ((Counter.MkCounter v) v)))
(def (main) (value (mk 5)))`}
      />
      <P>
        That reads <C>5</C>. The point is what happens with a bad input: hand <C>mk</C> a negative and the
        invariant holds — it clamps to <C>0</C> rather than storing nonsense:
      </P>
      <Runnable
        source={`(type Counter (MkCounter Int64))
(def (mk (: n Int64)) (if (< n 0) (Counter.MkCounter 0) (Counter.MkCounter n)))
(def (value (: c Counter)) (match c ((Counter.MkCounter v) v)))
(def (main) (value (mk -3)))`}
      />
      <P>
        Every operation you offer routes through <C>mk</C>, so the invariant is preserved by construction —
        a <C>bump</C> that increments can't produce a negative because it, too, goes through the smart
        constructor:
      </P>
      <Runnable
        source={`(type Counter (MkCounter Int64))
(def (mk (: n Int64)) (if (< n 0) (Counter.MkCounter 0) (Counter.MkCounter n)))
(def (value (: c Counter)) (match c ((Counter.MkCounter v) v)))
(def (bump (: c Counter)) (mk (+ (value c) 1)))
(def (main) (value (bump (mk 5))))`}
      />
      <P>
        These run in one file, where the constructor <C>MkCounter</C> is visible — so you can see the whole
        mechanism at once. The <em>enforcement</em>, though, is a property of the module boundary, which is
        where the interesting part lives.
      </P>

      <H2>The boundary is what enforces it</H2>
      <P>
        When <C>Counter</C> lives in its own module and exports only its handle plus <C>mk</C> and{" "}
        <C>value</C>, another module that imports it may name <C>Counter</C>, hold a <C>Counter</C>, pass one
        around, and call <C>mk</C>/<C>value</C>/<C>bump</C> on it — but it may <em>not</em> reach the
        constructor. An attempt to build one directly is a compile error:
      </P>
      <Note>
        <C>{`// in module "counter": (export Counter)  — the handle only, MkCounter stays private`}</C>
        <br />
        <C>{`// in another module: (Counter.MkCounter 999)`}</C>
        <br />
        <C>cdz</C> reports: <C>CDZ0214</C> — the constructor <C>MkCounter</C> is withheld; a{" "}
        <C>Counter</C> can be built only through the module's exported functions.
      </Note>
      <P>
        The same wall stops an importer from <em>taking a value apart</em>: it can't match on the private
        constructor, strip it, or structurally compare two values. Everything a <C>Counter</C> can do flows
        through the doors the module opened — which is exactly why the invariant can't be dodged. (The guide
        runs a single module at a time, so the runnable examples above show the mechanism from{" "}
        <em>inside</em>; the <C>CDZ0214</C> rejection is what the compiler prints when the very same
        construction is attempted from <em>outside</em>.)
      </P>

      <Why tenet="Hide the representation, and the invariant can't be broken">
        Data hiding here isn't a convention or a naming trick — it's checked by the type system. Because a
        type's handle and its constructors are exported independently, a module can publish a fully usable
        type whose <em>representation</em> is genuinely unreachable: the only values that exist are the ones
        its own functions made. So an invariant established in a smart constructor — non-negative, sorted,
        validated, normalized — is guaranteed for <em>every</em> value of that type, everywhere, with no
        trust in callers required. That's the foundation the more ambitious uses build on: a parser that
        only yields well-formed trees, a units library whose quantities can't be forged, a proof kernel
        whose theorems can only come from its inference rules.
      </Why>

      <H2>Your turn</H2>
      <Exercise
        id="opaque-types:1"
        prompt={
          <>
            The smart constructor enforces the invariant. This <C>mk</C> clamps negatives to <C>0</C>; fill
            the input so that reading the result back gives <C>0</C> — a value the invariant had to correct.
          </>
        }
        starter={`(type Counter (MkCounter Int64))
(def (mk (: n Int64)) (if (< n 0) (Counter.MkCounter 0) (Counter.MkCounter n)))
(def (value (: c Counter)) (match c ((Counter.MkCounter v) v)))
(def (main) (value (mk ?)))`}
        solution={`(type Counter (MkCounter Int64))
(def (mk (: n Int64)) (if (< n 0) (Counter.MkCounter 0) (Counter.MkCounter n)))
(def (value (: c Counter)) (match c ((Counter.MkCounter v) v)))
(def (main) (value (mk -8)))`}
        expected="0"
        hint={
          <>
            Any negative works — <C>mk</C> replaces it with <C>0</C>. For example <C>-8</C> clamps to{" "}
            <C>0</C>, so <C>value</C> reads <C>0</C>. A non-negative input would come back unchanged.
          </>
        }
      />

      <Exercise
        id="opaque-types:2"
        prompt={
          <>
            Because <C>bump</C> routes through <C>mk</C>, the invariant survives every step. Start from{" "}
            <C>(mk 0)</C> and fill how many times to <C>bump</C> so the result reads <C>3</C>.
          </>
        }
        starter={`(type Counter (MkCounter Int64))
(def (mk (: n Int64)) (if (< n 0) (Counter.MkCounter 0) (Counter.MkCounter n)))
(def (value (: c Counter)) (match c ((Counter.MkCounter v) v)))
(def (bump (: c Counter)) (mk (+ (value c) 1)))
(def (main) (value (bump (bump (bump (mk ?))))))`}
        solution={`(type Counter (MkCounter Int64))
(def (mk (: n Int64)) (if (< n 0) (Counter.MkCounter 0) (Counter.MkCounter n)))
(def (value (: c Counter)) (match c ((Counter.MkCounter v) v)))
(def (bump (: c Counter)) (mk (+ (value c) 1)))
(def (main) (value (bump (bump (bump (mk 0))))))`}
        expected="3"
        hint={
          <>
            Three <C>bump</C>s add <C>3</C>, so starting from <C>(mk 0)</C> gives <C>3</C>. Every
            intermediate value still went through <C>mk</C>, so the never-negative invariant held the whole
            way.
          </>
        }
      />
    </article>
  );
}
