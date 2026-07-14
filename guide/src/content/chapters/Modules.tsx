import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Modules() {
  return (
    <article>
      <H1>Modules</H1>
      <Lede>
        As a program grows, related definitions want a home. A module groups them under a name — and,
        because a module is just a record of what it defines, you reach inside it the way you reach into
        any record.
      </Lede>

      <P>
        A <C>module</C> gathers definitions and binds a name for them. Say we have a temperature
        conversion — it belongs with other temperature code, under a <C>Temp</C> name, reached by
        qualifying it: <C>Temp.c-to-f</C>.
      </P>
      <Runnable
        source={`(def (main)
  (do
    (module Temp
      (def (c-to-f c) (+ (/ (* c 9) 5) 32)))
    (Temp.c-to-f 100)))`}
      />
      <P>
        100°C is 212°F. <C>Temp</C> names the group; <C>Temp.c-to-f</C> reaches the conversion inside it.
        The definition lives in the module's namespace rather than loose in the surrounding scope — the
        same dotted access you already use for a record's field.
      </P>

      <H2>A module keeps its own pieces together</H2>
      <P>
        The real value shows once a module has more than one piece. Here <C>Circle</C> holds a constant{" "}
        <C>pi</C> and an <C>area</C> that uses it. The caller only deals with <C>Circle.area</C> — the{" "}
        <C>pi</C> is an internal detail the module manages for itself:
      </P>
      <Runnable
        source={`(def (main)
  (do
    (module Circle
      (def pi 3)
      (def (area r) (* pi (* r r))))
    (Circle.area 10)))`}
      />
      <P>
        <C>area 10</C> is <C>3 × 10 × 10</C> = <C>300</C>. The function reads <C>pi</C> directly, because
        inside the module they're siblings; from outside you just call <C>area</C> and don't think about
        how it's computed.
      </P>

      <H2>Modules nest, and compose</H2>
      <P>
        A module can hold another module, so one file can carry a whole tree of scopes — much like a
        module tree in Rust. That's also how you keep several related modules together: here a <C>Calc</C>{" "}
        module groups an <C>Inc</C> and a <C>Scale</C>, each with its own job. You reach through the
        layers with the same dotted access, one name per level, and a qualified name says exactly which
        piece you mean — so there's never a question of whose <C>f</C> is whose:
      </P>
      <Runnable
        source={`(def (main)
  (do
    (module Calc
      (module Inc (def (f x) (+ x 1)))
      (module Scale (def (g x) (* x 10))))
    (Calc.Scale.g (Calc.Inc.f 4))))`}
      />
      <P>
        <C>Calc.Inc.f 4</C> is 5, then <C>Calc.Scale.g 5</C> is <C>50</C> — each name read left to right,
        into <C>Calc</C>, then the inner module, then the function. Swap the order —{" "}
        <C>(Calc.Inc.f (Calc.Scale.g 4))</C> — and you'd get 41 instead; the qualified names make the
        pipeline unambiguous either way. It's the same field access as a record inside a record: nesting
        modules is nothing new, because a module was a record all along.
      </P>

      <Why tenet="A module is a record of its exports">
        Cadenza doesn't bolt on a separate "module system" with its own rules — a module is just a{" "}
        <em>value</em>, a record whose fields are its definitions, bound to a name. That's why you reach
        into it with the same <C>.</C> you use for any record: there's one idea of "a named thing with
        fields", not two. Grouping and namespacing fall out of a feature the language already has. The
        payoff is that everything you know about records — how they're built, accessed, passed around —
        already tells you how modules behave.
      </Why>

      <Note>
        This is scoping <em>within</em> one program. A larger project also splits across files, where a
        module <C>import</C>s the names another <C>export</C>s — the same grouping idea, scaled up to a
        whole package.
      </Note>

      <H2>Your turn</H2>
      <Exercise
        id="modules:1"
        prompt={<>Finish <C>area</C> so <C>Circle.area 5</C> gives <C>75</C> — it should use the module's <C>pi</C>.</>}
        starter={`(def (main)
  (do
    (module Circle
      (def pi 3)
      (def (area r) (* pi ?)))
    (Circle.area 5)))`}
        solution={`(def (main)
  (do
    (module Circle
      (def pi 3)
      (def (area r) (* pi (* r r))))
    (Circle.area 5)))`}
        expected="75"
        hint={<>Area is <C>pi × r × r</C>. You need <C>r</C> squared: <C>(* r r)</C>.</>}
      />

      <Exercise
        id="modules:2"
        prompt={
          <>
            <C>f</C> lives inside <C>Double</C>, which lives inside <C>Mathy</C>. Write the qualified path
            to call it on <C>8</C> — doubling gives <C>16</C>.
          </>
        }
        starter={`(def (main)
  (do
    (module Mathy
      (module Double
        (def (f x) (* x 2))))
    (?.f 8)))`}
        solution={`(def (main)
  (do
    (module Mathy
      (module Double
        (def (f x) (* x 2))))
    (Mathy.Double.f 8)))`}
        expected="16"
        hint={
          <>
            Name each level from the outside in, separated by dots: <C>Mathy.Double.f</C>. Then{" "}
            <C>8 × 2 = 16</C>.
          </>
        }
      />
    </article>
  );
}
