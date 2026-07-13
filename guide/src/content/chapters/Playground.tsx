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

      <P>
        Open the <PlaygroundLink>playground</PlaygroundLink> and paste in the last program you wrote in
        this guide — then reach for the REPL and start calling its pieces.
      </P>
    </article>
  );
}
