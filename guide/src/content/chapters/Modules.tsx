import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Modules() {
  return (
    <article>
      <H1>Modules</H1>
      <Lede>
        As a program grows, related definitions want a home. A module groups them under a name, and
        because a module is just a record of what it defines, you reach inside it the way you reach into
        any record.
      </Lede>

      <P>
        A <C>module</C> gathers definitions and binds a name for them. It stands at the top level as a
        sibling of your other definitions, not something tucked inside a function, and it <C>export</C>s
        the pieces callers may use. Say we have a temperature conversion; it belongs with other
        temperature code, under a <C>Temp</C> name, reached by qualifying it: <C>Temp.c-to-f</C>.
      </P>
      <Runnable
        source={`(do
  (module Temp
    (def (c-to-f c) (+ (/ (* c 9) 5) 32))
    (export c-to-f))
  (def (main) (Temp.c-to-f 100))
  (export main))`}
      />
      <P>
        100°C is 212°F. <C>Temp</C> names the group and exports <C>c-to-f</C>; <C>main</C> reaches the
        conversion with the qualified name <C>Temp.c-to-f</C>. The definition lives in the module's
        namespace rather than loose in the surrounding scope, the same dotted access you already use for
        a record's field.
      </P>

      <H2>A module keeps its own pieces together</H2>
      <P>
        The real value shows once a module has more than one piece. Here <C>Circle</C> holds a constant{" "}
        <C>pi</C> and an <C>area</C> that uses it. The caller only deals with <C>Circle.area</C>, since the{" "}
        <C>pi</C> is an internal detail the module manages for itself:
      </P>
      <Runnable
        source={`(do
  (module Circle
    (def pi 3)
    (def (area r) (* pi (* r r)))
    (export area))
  (def (main) (Circle.area 10))
  (export main))`}
      />
      <P>
        <C>area 10</C> is <C>3 × 10 × 10</C> = <C>300</C>. The function reads <C>pi</C> directly, because
        inside the module they're siblings; from outside you just call <C>area</C> and don't think about
        how it's computed.
      </P>

      <H2>Composing across modules</H2>
      <P>
        Two modules, each with its own job, combine cleanly, since a qualified name says exactly which
        piece you mean, so there's never a question of whose <C>f</C> is whose:
      </P>
      <Runnable
        source={`(do
  (module Inc (def (f x) (+ x 1)) (export f))
  (module Scale (def (g x) (* x 10)) (export g))
  (def (main) (Scale.g (Inc.f 4)))
  (export main))`}
      />
      <P>
        <C>Inc.f 4</C> is 5, then <C>Scale.g 5</C> is <C>50</C>. Swap the order to{" "}
        <C>(Inc.f (Scale.g 4))</C> and you'd get 41 instead, so the qualified names make the pipeline
        unambiguous either way.
      </P>

      <H2>Modules nest</H2>
      <P>
        A module can hold another module, so one file can carry a whole tree of scopes, much like a
        module tree in Rust. You reach through the layers with the same dotted access, one name per
        level. Here a <C>Geometry</C> module contains a <C>Square</C> module with an <C>area</C>:
      </P>
      <Runnable
        source={`(do
  (module Geometry
    (module Square
      (def (area s) (* s s))
      (export area))
    (export Square))
  (def (main) (Geometry.Square.area 5))
  (export main))`}
      />
      <P>
        <C>Geometry.Square.area 5</C> reads left to right, into <C>Geometry</C>, then <C>Square</C>,
        then <C>area</C>, and gives <C>25</C>. It's the same field access as a record inside a record;
        nesting modules is nothing new, because a module was a record all along.
      </P>

      <Why tenet="A module is a record of its exports">
        Cadenza doesn't bolt on a separate "module system" with its own rules, since a module is just a{" "}
        <em>value</em>, a record whose fields are its definitions, bound to a name. That's why you reach
        into it with the same <C>.</C> you use for any record: there's one idea of "a named thing with
        fields", not two. Grouping and namespacing fall out of a feature the language already has, so
        everything you know about records, how they're built, accessed, and passed around,
        already tells you how modules behave.
      </Why>

      <Note>
        This is scoping <em>within</em> one program. A larger project also splits across files, where a
        module <C>import</C>s the names another <C>export</C>s, the same grouping idea, scaled up to a
        whole package.
      </Note>

      <H2>Your turn</H2>
      <Exercise
        id="modules:1"
        prompt={
          <>
            Two modules, each with one job: <C>Money.cents</C> turns dollars into cents (×100), and{" "}
            <C>Tax.add</C> adds <C>5</C>. Compose them by feeding <C>2</C> dollars through both, qualifying
            the inner call with the right module name, so the answer is <C>205</C>.
          </>
        }
        starter={`(do
  (module Money (def (cents d) (* d 100)) (export cents))
  (module Tax   (def (add c) (+ c 5)) (export add))
  (def (main) (Tax.add (?.cents 2)))
  (export main))`}
        solution={`(do
  (module Money (def (cents d) (* d 100)) (export cents))
  (module Tax   (def (add c) (+ c 5)) (export add))
  (def (main) (Tax.add (Money.cents 2)))
  (export main))`}
        expected="205"
        hint={
          <>
            <C>cents</C> lives in <C>Money</C>, so the qualified name is <C>Money.cents</C>. Then{" "}
            <C>2 × 100 = 200</C>, and <C>Tax.add</C> makes it <C>205</C>. (Qualify it with <C>Tax</C> and the
            compiler declines, since <C>Tax</C> has no <C>cents</C>.)
          </>
        }
      />

      <Exercise
        id="modules:2"
        prompt={
          <>
            <C>f</C> lives inside <C>Double</C>, which lives inside <C>Mathy</C>. Write the qualified path
            to call it on <C>8</C>, so doubling gives <C>16</C>.
          </>
        }
        starter={`(do
  (module Mathy
    (module Double
      (def (f x) (* x 2))
      (export f))
    (export Double))
  (def (main) (?.f 8))
  (export main))`}
        solution={`(do
  (module Mathy
    (module Double
      (def (f x) (* x 2))
      (export f))
    (export Double))
  (def (main) (Mathy.Double.f 8))
  (export main))`}
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
