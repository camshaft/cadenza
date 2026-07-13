import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Exercise } from "../../components/Exercise.tsx";
import { Why } from "../../components/Why.tsx";

export default function Modules() {
  return (
    <article>
      <H1>Modules</H1>
      <Lede>
        As a program grows, related definitions want a home. A module groups them under a name and
        controls what's visible from outside — Cadenza's unit of scoping.
      </Lede>

      <P>
        A <C>module</C> gathers definitions and binds a name for them. From outside, you reach a
        definition by qualifying it with the module's name — <C>Math.square</C> — the same dotted access
        you already use to reach a record's field, because a module <em>is</em> a record of what it
        exports:
      </P>
      <Runnable
        source={`(def (main)
  (do
    (module Math
      (def (square x) (* x x)))
    (Math.square 5)))`}
      />
      <P>
        <C>Math</C> names the group; <C>Math.square</C> reaches its <C>square</C>. The definition lives in
        the module's namespace, not loose in the surrounding scope.
      </P>

      <H2>Modules hold values too</H2>
      <P>
        A module isn't only functions — a value definition is a field like any other. A little{" "}
        <C>Config</C> module makes a tidy namespace for constants:
      </P>
      <Runnable
        source={`(def (main)
  (do
    (module Config
      (def limit 100))
    (Config.limit)))`}
      />

      <H2>Several definitions, one namespace</H2>
      <P>
        The point is grouping: put the related pieces together and call them through the one name.
      </P>
      <Runnable
        source={`(def (main)
  (do
    (module Geo
      (def (square x) (* x x))
      (def (double x) (* x 2)))
    (+ (Geo.square 3) (Geo.double 5))))`}
      />
      <P>
        <C>Geo.square 3</C> is 9, <C>Geo.double 5</C> is 10 — summed to <C>19</C>. Two definitions, reached
        through one qualified name.
      </P>

      <Why tenet="A module is a record of its exports">
        Cadenza doesn't add a separate "module system" bolted onto the language — a module is just a{" "}
        <em>value</em>, a record whose fields are its exported definitions, bound to a name. That's why
        you reach into it with the same <C>.</C> you use for any record: there's one idea of "a named
        thing with fields", not two. Grouping and namespacing fall out of a feature the language already
        has, rather than a new construct with its own rules.
      </Why>

      <Note>
        This is scoping <em>within</em> a program. A larger project also splits across files, where one
        module <C>import</C>s the names another <C>export</C>s — the same grouping idea, scaled up to the
        whole package.
      </Note>

      <H2>Your turn</H2>
      <Exercise
        id="modules:1"
        prompt={<>Call <C>Math.square</C> on <C>6</C> through the module, so the answer is <C>36</C>.</>}
        starter={`(def (main)
  (do
    (module Math
      (def (square x) (* x x)))
    (Math.square ?)))`}
        solution={`(def (main)
  (do
    (module Math
      (def (square x) (* x x)))
    (Math.square 6)))`}
        expected="36"
        hint={<>Qualify the call with the module name and pass <C>6</C>: <C>(Math.square 6)</C>.</>}
      />
    </article>
  );
}
