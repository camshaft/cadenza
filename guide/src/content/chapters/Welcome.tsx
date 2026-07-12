import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";

export default function Welcome() {
  return (
    <article>
      <H1>Welcome to Cadenza</H1>
      <Lede>
        A programming language designed to be written and read by AI agents, read by humans, verified
        for its properties, and compiled to sandboxed WebAssembly components.
      </Lede>

      <P>
        This guide is interactive. Every example below is <strong>live</strong>: you can edit the code
        and press <C>▶ Run</C> to compile and execute it — right here in your browser. There is no
        server. The Cadenza compiler itself has been compiled to WebAssembly, so when you press Run,
        your browser compiles your program to a WebAssembly component and runs it, all locally.
      </P>

      <Runnable title="Your first program" source={`(+ 2 3)`} />

      <P>
        Try changing <C>2</C> and <C>3</C> to other numbers, then Run again. The result is shown with
        its type — Cadenza infers that this is an <C>Int64</C>.
      </P>

      <H2>One language, two syntaxes</H2>
      <P>
        A Cadenza program is stored as a single canonical structure — code as data. What you read is a{" "}
        <em>projection</em> of that structure, and more than one projection exists. Use the{" "}
        <strong>syntax toggle in the header</strong> to switch every example on the page between the{" "}
        <em>conventional</em> surface (an ML/Rust-family syntax) and the <em>s-expression</em> surface
        (the direct code-as-data form). Switching never changes the program — it is the same structure,
        printed differently.
      </P>

      <Note>
        Flip the header toggle now and watch this example re-render. Both forms compile to the exact
        same component.
      </Note>

      <Runnable title="The same program, either way" source={`((fn (x) (+ x 1)) 5)`} />

      <H2>How to use this guide</H2>
      <P>
        Work through the chapters in order using the sidebar, or jump around. Each chapter has runnable
        examples you are encouraged to edit — you will learn faster by changing things and seeing what
        happens than by only reading.
      </P>
    </article>
  );
}
