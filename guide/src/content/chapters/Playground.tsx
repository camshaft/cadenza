import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Link } from "react-router-dom";

/// A router-aware link that respects the Pages sub-path basename.
function PlaygroundLink({ children }: { children: React.ReactNode }) {
  return (
    <Link to="/playground" className="font-medium text-cadenza-300 underline-offset-2 hover:underline">
      {children}
    </Link>
  );
}

function CalculatorLink({ children }: { children: React.ReactNode }) {
  return (
    <Link to="/calculator" className="font-medium text-cadenza-300 underline-offset-2 hover:underline">
      {children}
    </Link>
  );
}

/// A router-aware internal link to another chapter (respects the Pages sub-path basename).
function Ch({ to, children }: { to: string; children: React.ReactNode }) {
  return (
    <Link to={to} className="text-cadenza-300 underline-offset-2 hover:underline">
      {children}
    </Link>
  );
}

export default function Playground() {
  return (
    <article>
      <H1>The playground</H1>
      <Lede>
        Every example in this guide is a scratchpad — but the <PlaygroundLink>playground</PlaygroundLink>{" "}
        is the full workshop: a real editor, a REPL, and a window into what your code compiles to.
      </Lede>

      <P>
        The same compiler that runs the guide's inline examples powers a standalone editor. Nothing runs
        on a server — the compiler is itself WebAssembly, so your code is compiled and executed entirely
        in your browser. Here's what you can do there beyond pressing Run.
      </P>

      <H2>A REPL over your own module</H2>
      <P>
        Pressing Run evaluates one thing: your module's <C>main</C>. But a module usually defines{" "}
        <em>many</em> functions, and often you want to poke at them one at a time — call{" "}
        <C>double</C> on 21, then on 100, then try <C>compose</C> on the result. That's what the{" "}
        <strong>REPL</strong> tab is for.
      </P>
      <P>
        Type an expression — in whichever syntax you're using — and press Enter. It's evaluated against
        every definition your buffer declares, and the value comes back rendered just like a run. Say
        your editor holds:
      </P>
      <Note>
        <C>{`(def (double n) (* n 2))`}</C> and <C>{`(def (add a b) (+ a b))`}</C>
      </Note>
      <P>
        Then in the REPL you can enter <C>(double 21)</C> and get <C>42</C>, or nest calls freely —{" "}
        <C>(double (add 3 4))</C> gives <C>14</C>. Results aren't limited to numbers: an expression that
        builds a list or a tuple shows the whole value, the same canonical form a run produces. Press
        the up arrow to recall a previous expression and tweak it.
      </P>

      <Note>
        The REPL calls your functions by compiling a tiny program that invokes them and running it — so a
        REPL answer is exactly what the compiled program computes, never a separate interpreter that
        might disagree. It's the real thing, every time.
      </Note>

      <H2>See what it compiles to</H2>
      <P>
        The <strong>Compiled</strong> tab shows the output of the backend. <strong>WAT</strong> is the
        WebAssembly text of the core module your program becomes — the actual instructions that run.
        <strong>Rust</strong> (and <strong>Rust (async)</strong>) show the same program lowered to Rust
        source instead, because Cadenza is target-neutral above its backend: one typed program, more than
        one place it can go.
      </P>
      <P>
        It's worth a look even if you never write WebAssembly by hand — seeing <C>(+ 2 3)</C> become a
        single constant, or a recursive function become a loop, makes the compiler feel less like a black
        box.
      </P>

      <H2>The editor helps as you type</H2>
      <P>
        The playground's editor is a small IDE. Mistakes are underlined <em>as you type</em>, with the
        same diagnostics the compiler would report — no need to run first. Hover a name to see its
        inferred type. The <strong>Diagnostics</strong> tab lists every problem, and clicking one jumps
        to it in the source. The <strong>AST</strong> tab shows the raw tree the compiler sees — the one
        structure both syntaxes are just views of.
      </P>
      <P>
        Even the colours come from the compiler. Rather than guessing a token's role from its spelling,
        the editor asks the compiler what each name <em>means</em> — so a type, a constructor, a function,
        a local, and an <em>unbound</em> name (a typo the compiler can't resolve) each get their own
        colour. The highlighting is the same understanding the type checker has, not a separate guess.
      </P>

      <H2>Share what you made</H2>
      <P>
        Press <strong>Share</strong> and the whole program is packed into the page's URL — no account, no
        server, no saved file. Send the link and the recipient opens your exact program, in the syntax
        you wrote it. It's the fastest way to ask a question or show something off.
      </P>

      <H2>A calculator, too</H2>
      <P>
        There's a lighter-weight sibling for quick sums: the{" "}
        <CalculatorLink>calculator</CalculatorLink>. It's a focused prompt over the same language, tuned for
        arithmetic you'd otherwise reach for a desk calculator to do — except it's <em>exact</em>. Fractions
        stay fractions (<C>1 / 3 + 1 / 3 + 1 / 3</C> is <C>1</C>, not <C>0.9999…</C>), dimensioned quantities
        carry their units, and big integers don't overflow. Recall the previous result with <C>ans</C> and
        keep going.
      </P>
      <P>
        It's the <strong>Exact fractions</strong> and <strong>Units of measure</strong> chapters made
        interactive — reach for it when you want a number now, and the playground when you want a program.
      </P>

      <H2>More to explore</H2>
      <P>
        Two more surfaces run the same in-browser compiler, each showing off a different corner of the
        language. The <Ch to="/cad">CAD preview</Ch> renders a solid modelled in Cadenza — a 3D shape built
        from exact rational coordinates and real units (the <strong>exact CAD</strong> worked example in{" "}
        <Ch to="/units">Units of measure</Ch> made visible), which you can orbit and inspect. And the{" "}
        <Ch to="/notebook">notebook</Ch> is a live document: markdown prose interleaved with runnable code
        cells and interactive widgets — drag a slider and every dependent cell recomputes, so a
        compound-interest model or a data table stays parametric. Both are ordinary Cadenza under the hood;
        they just render its results as geometry, tables, and charts instead of a single value.
      </P>

      <H2>Beyond the browser: the <C>cdz</C> toolchain</H2>
      <P>
        Everything here runs the real compiler in your browser — but the same compiler ships as a single
        command-line tool, <C>cdz</C>, for working on your own machine. One binary carries the whole loop:
        <C>cdz new</C> scaffolds a project (a <C>Project.cdz</C> manifest, a buildable entry file, and a
        passing starter <C>@test</C>, so <C>cdz test</C> is green from the first command);{" "}
        <C>cdz compile</C> lowers a single program to a WebAssembly component and <C>cdz build</C> compiles a
        whole project into one component; <C>cdz run</C> executes a compiled one, <C>cdz check</C> reports
        diagnostics without building, and <C>cdz test</C> runs a module's tests while <C>cdz fmt</C> reprints
        source in canonical form. For the tightest loop of all, <C>cdz watch</C> re-runs any of those on every
        save — the same instant-feedback cycle you've had in this guide, at your terminal. There's even a{" "}
        <C>cdz calc</C> — the same exact-arithmetic calculator you met above.
      </P>
      <P>
        And your editor can speak to that compiler directly. <C>cdz lsp</C> is a Language Server, so any
        editor that speaks the protocol gets the <em>same</em> compiler-backed help the playground has —
        not a text-based guess, but the type checker's actual understanding. In VS Code (one-step setup
        with <C>cargo xtask install-lsp</C>) you get, live as you type: diagnostics, hover types, semantic
        highlighting by role, go-to-definition and find-references, same-symbol highlighting (rest the caret
        on a name and every other use of it tints, the declaration included — shadowing respected, so a local
        never lights up an unrelated top-level), name completion from what's actually in scope, one-click
        quick-fixes from the compiler's suggested repairs, and an outline of a file's definitions.
      </P>
      <P>
        One of those is unique to Cadenza's compiler. Above every generic definition the compiler{" "}
        <em>specialized</em>, the editor shows a <em>CodeLens</em> naming its concrete instances — a label
        like <C>2 instances: [n: Int64, x: Int64] · [n: Int64, x: String]</C> above a generic <C>loopn</C>{" "}
        used at both types. Cadenza monomorphizes generics (see{" "}
        <Ch to="/ad-hoc-polymorphism">Ad-hoc polymorphism</Ch>), and this makes that normally-invisible
        specialization visible right in the source — it's the in-editor face of the same query{" "}
        <C>cdz instantiations</C> answers on the command line. A plain, non-generic definition gets no
        lens, because nothing was specialized.
      </P>
      <Note>
        It's the same principle as the playground's editor, carried to your own tools: the colours, the
        completions, the fixes all come from the compiler that will build your code — so what the editor
        tells you and what the compiler does never drift apart.
      </Note>

      <P>
        Open the <PlaygroundLink>playground</PlaygroundLink> and paste in the last program you wrote in
        this guide — then reach for the REPL and start calling its pieces.
      </P>
    </article>
  );
}
