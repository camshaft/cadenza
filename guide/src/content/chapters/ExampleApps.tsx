import { H1, Lede, H2, P, C } from "../../components/Prose.tsx";
import { Link } from "react-router-dom";

/// A router-aware link to an app route (respects the Pages sub-path basename).
function AppLink({ to, children }: { to: string; children: React.ReactNode }) {
  return (
    <Link to={to} className="font-medium text-cadenza-300 underline-offset-2 hover:underline">
      {children}
    </Link>
  );
}

/// A router-aware internal link to another chapter.
function Ch({ to, children }: { to: string; children: React.ReactNode }) {
  return (
    <Link to={to} className="text-cadenza-300 underline-offset-2 hover:underline">
      {children}
    </Link>
  );
}

export default function ExampleApps() {
  return (
    <article>
      <H1>Example applications</H1>
      <Lede>
        Everything you've learned isn't a toy language — it compiles and runs in this very browser. These
        are full, interactive applications built in Cadenza, each showing a different part of the language
        doing real work. Open any of them; they're the differentiators you just met, running.
      </Lede>

      <P>
        Each app below is a live page — click through and use it. This gallery is the curated index: what
        each one is, which Cadenza features it showcases, and where in the guide those features are taught.
        The set is growing, so expect more over time.
      </P>

      <H2>The playground</H2>
      <P>
        The <AppLink to="/playground">playground</AppLink> is the whole language in one editor: write a
        program, run it, and peek at the WebAssembly <em>and</em> Rust it compiles to — the same source
        targeting two backends, which is the point of a compiler that treats code as data. It ships a
        dropdown of worked example programs — an AST interpreter, a Collatz walk, function composition,
        a memoized Fibonacci, exact-rational arithmetic, set algebra, an RPN stack machine, a stateful{" "}
        <Ch to="/effects">effect handler</Ch>, and more — each a small showcase of recursion, sum types,
        closures, effects, or the collections. Two stand out: a Rule 110
        cellular automaton, a Turing-complete system in a handful of lines; and a{" "}
        <em>code-as-data</em> example that quotes a program into an AST, splices a computed value into it,
        and <C>eval</C>s the result — Cadenza building and running Cadenza from within itself. It's where{" "}
        <Ch to="/metaprogramming">metaprogramming</Ch> and the target-neutral value model become tangible.
        (The <Ch to="/using-the-playground">playground chapter</Ch> tours its features in depth.)
      </P>

      <H2>The calculator</H2>
      <P>
        The <AppLink to="/calculator">calculator</AppLink> is a focused prompt for exact arithmetic — the{" "}
        <Ch to="/rationals">exact fractions</Ch> and <Ch to="/units">units of measure</Ch> chapters made
        interactive. Fractions stay fractions (<C>1 / 3 + 1 / 3 + 1 / 3</C> is <C>1</C>, not{" "}
        <C>0.9999…</C>), dimensioned quantities carry their units, and big integers never overflow. Reach
        for it when you want a number now and don't want a floating-point surprise.
      </P>

      <H2>The CAD preview</H2>
      <P>
        The <AppLink to="/cad">CAD preview</AppLink> renders a 3D solid modelled in Cadenza — a shape built
        from <em>exact rational coordinates</em> in real <Ch to="/units">units</Ch> (the exact-CAD worked
        example from the Units chapter, made visible). Orbit and inspect it. The model stays exact where it
        matters; floating point appears only at the very end, in the mesh the renderer draws — exact where
        it counts, approximate only at the geometry kernel.
      </P>

      <H2>The notebook</H2>
      <P>
        The <AppLink to="/notebook">notebook</AppLink> is a live, parametric document: prose interleaved
        with runnable Cadenza code cells and interactive widgets. Drag a slider and every dependent cell
        recomputes — a compound-interest model or a data table stays live. It's the reactive, run-the-outside-world
        side of the language — the same "perform an operation, let the surrounding context decide what it
        means" idea you met in <Ch to="/effects">effects &amp; handlers</Ch>, turned into a document.
      </P>

      <P>
        All four run the identical in-browser compiler that powers this guide's inline examples — geometry,
        tables, charts, and values are just what Cadenza's results look like when an app renders them
        instead of printing a single number.
      </P>

      <P>
        That's the tour — the ideas, and the apps that prove them. To recap what you've learned and find
        where to go from here, head to <Ch to="/whats-next">Where to go next</Ch>.
      </P>
    </article>
  );
}
