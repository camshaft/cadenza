import { H1, Lede, H2, P, C } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Link } from "react-router-dom";

/// A small internal link that respects the router basename (works under a Pages sub-path).
function Ch({ to, children }: { to: string; children: React.ReactNode }) {
  return (
    <Link to={to} className="text-cadenza-300 underline-offset-2 hover:underline">
      {children}
    </Link>
  );
}

export default function WhatsNext() {
  return (
    <article>
      <H1>Where to go next</H1>
      <Lede>You've met the core of Cadenza. Here's what you can now read, write, and reason about.</Lede>

      <H2>What you've learned</H2>
      <P>
        You can bind values and write functions (<Ch to="/basics">Values &amp; functions</Ch>), pipe
        and compose them (<Ch to="/functions">Composing functions</Ch>), branch and recurse{" "}
        (<Ch to="/control-flow">Control flow</Ch>), compare and order{" "}
        (<Ch to="/ordering">Comparison &amp; ordering</Ch>), and bundle data into tuples and records —
        reading, nesting, and functionally updating it (<Ch to="/data">Tuples &amp; records</Ch>,{" "}
        <Ch to="/records-tuples">Working with records &amp; tuples</Ch>). You decide by shape with{" "}
        <Ch to="/pattern-matching">pattern matching</Ch>, and you've met the collections —{" "}
        <Ch to="/lists">lists</Ch>, <Ch to="/maps-sets">maps &amp; sets</Ch> — and the text and binary
        types, <Ch to="/strings">strings</Ch>, <Ch to="/bytes">bytes</Ch>, and{" "}
        <Ch to="/symbols">symbols</Ch>. You handle{" "}
        <Ch to="/errors">absence and errors</Ch> with <C>Option</C>/<C>Result</C>, and you know how
        Cadenza's numbers behave — checked <Ch to="/numbers">integers</Ch>,{" "}
        <Ch to="/sized-integers">sized integers</Ch>,{" "}
        <Ch to="/floats">floating-point numbers</Ch>, and{" "}
        <Ch to="/rationals">exact fractions</Ch>, none of them ever converting silently.
      </P>
      <P>
        And you've seen what makes Cadenza its own language:{" "}
        <Ch to="/effects">effects &amp; handlers</Ch>, where a handler decides what a performed operation
        means; <Ch to="/modules">modules</Ch> that are just records of their exports; and{" "}
        <Ch to="/units">units of measure</Ch> that catch a dimensional mistake at compile time and then
        vanish before the program runs.
      </P>
      <P>
        More importantly, you've seen the ideas underneath — the{" "}
        <Ch to="/philosophy">tenets</Ch> that explain <em>why</em> the language makes the choices it
        does. Determinism and capability-safety as the floor. Declining rather than miscompiling. One
        program, many syntaxes. Refusing the ambiguous conversion. These aren't trivia; they're the
        through-line, and they'll make the rest of Cadenza feel predictable.
      </P>

      <H2>One last program</H2>
      <P>
        A little of everything: a recursive helper, a comparison, and a record — computing the larger
        coordinate of a point, doubled.
      </P>
      <Runnable
        source={`(def (max a b) (if (> a b) a b))
(def (main)
  (let ((p (record (x 3) (y 8))))
    (* 2 (max (. p x) (. p y)))))`}
      />
      <P>
        Edit it — change the point, the factor, swap <C>max</C> for <C>min</C> — and Run. Then flip the
        syntax toggle in the header and watch the very same program re-appear in the other surface.
      </P>

      <H2>Keep exploring</H2>
      <P>
        The best way forward is to keep an example open and change it. Every code block in this guide is
        a scratchpad — break it, fix it, and press Run. That loop, with the compiler answering
        instantly, is the fastest way to build a feel for the language.
      </P>
      <P>
        When you outgrow a single code block, move to the{" "}
        <Ch to="/playground">playground</Ch>: a full editor with a REPL for calling your functions one
        at a time, a peek at the WebAssembly and Rust your code compiles to, and a Share button that
        packs a whole program into a link.
      </P>
    </article>
  );
}
