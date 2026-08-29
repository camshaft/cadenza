// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (chapterModel.ts).
import { C, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Ch } from "../../components/ChapterLink.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Why } from "../../components/Why.tsx";
import { StatusLegend } from "../../components/StatusIcon.tsx";

export default function Welcome() {
  return (
    <article>
      <H1>Welcome to Cadenza</H1>
      <Lede>A programming language designed to be written and read by AI agents, read by humans, verified for its properties, and compiled to sandboxed WebAssembly components.</Lede>
      <Why tenet="Written for agents and humans">Cadenza's whole shape follows from one choice of audience: it is meant to be <em>written and read by AI agents</em> as much as by people. Once your author is a machine that manipulates structure and acts on your output, a cascade of decisions follows: code stored as data, diagnostics that carry verified fixes, one program shown in whichever syntax the reader prefers. As you go, watch how often a feature traces back to this.</Why>
      <P>This guide is interactive. Every example below is <strong>live</strong>: you can edit the code and press <C>▶ Run</C> to compile and execute it right here in your browser. There is no server. The Cadenza compiler itself has been compiled to WebAssembly, so when you press Run, your browser compiles your program to a WebAssembly component and runs it, all locally.</P>
      <Runnable
        source={`(+ 2 3)`}
        title="Your first program"
      />
      <P>Try changing <C>2</C> and <C>3</C> to other numbers, then Run again. The result is <C>5</C>, and though nothing here says so, Cadenza has inferred its type is <C>Int64</C>, the checked 64-bit integer you'll meet in <strong>The numeric model</strong>.</P>
      <H2>One language, two syntaxes</H2>
      <P>A Cadenza program is stored as a single canonical structure, code as data. What you read is a <em>projection</em> of that structure, and more than one projection exists. Use the <strong>syntax toggle in the header</strong> to switch every example on the page between the <em>conventional</em> surface (an ML/Rust-family syntax) and the <em>s-expression</em> surface (the direct code-as-data form). Switching never changes the program, since it is the same structure, printed differently.</P>
      <Note>Flip the header toggle now and watch this example re-render. Both forms compile to the exact same component.</Note>
      <Runnable
        source={`((fn (x) (+ x 1)) 5)`}
        title="The same program, either way"
      />
      <Why tenet="One representation, many syntaxes">A Cadenza program isn't <em>really</em> text, but a uniform data structure (code as data), and each syntax is just a lossless <em>projection</em> of it. That's why there's no single "the syntax" everyone must agree on: humans get a readable surface, agents get a literal-structure surface, and neither is privileged. It also means whitespace, line endings, and which syntax you chose can never change a program's identity, because the identity is the tree, not the text.</Why>
      <H2>How to use this guide</H2>
      <P>The sidebar has two parts. <strong>Cadenza the Language</strong> is the interactive tour you're starting now: work through its chapters in order, or jump around. <strong>Cadenza the Platform</strong> comes after it, a shorter concept tour of the agent kernel the language was built to run on; it's reading rather than exercises, and you can save it for when you're curious how the pieces add up to a system, or head there early from <Ch to="/platform-overview">its overview</Ch>. This first part is where you'll actually learn to write Cadenza.</P>
      <P>Each chapter has runnable examples you are encouraged to edit, and you will learn faster by changing things and seeing what happens than by only reading. Many chapters end with <strong>exercises</strong>: fill in the blank, press <C>Check</C>, and the guide grades your answer. A <C>Show solution</C> button is always there if you get stuck, but try not to lean on it too soon.</P>
      <P>When you run an example, its result is tagged so you always know what happened:</P>
      <StatusLegend />
      <P>That last one, the compiler <em>declining</em> a program, is worth dwelling on. Cadenza would rather refuse a program it can't compile correctly than emit something that silently misbehaves. Some examples in this guide are declined <em>on purpose</em>, to show you the language's guardrails.</P>
      <Why tenet="Decline, don't miscompile">Cadenza orders its possible outcomes by safety: a wrong value is worse than a crash, a crash is worse than an honest refusal, and a correct answer is best of all. So when the compiler can't be sure it would compile your program <em>correctly</em>, it declines rather than guessing. A refusal is never a dead end, either, since the goal is that every rejection carries a machine-checkable route back to a working program.</Why>
      <Why tenet="The source is the truth">A program's meaning lives in its source, never in the artifact it compiles to. The WebAssembly component is a rebuildable, content-addressed <em>function</em> of the source, so you fix a bug in the source and recompile, never by patching the binary. This is also why the same source always derives the same bytes: derivation is a pure function of the source and the toolchain.</Why>
      <P>Those last boxes are a preview: nearly every choice in Cadenza traces back to a handful of ideas. The next chapter, <em>Why Cadenza is the way it is</em>, lays them out, and you don't need them to write code, but they make everything that follows feel inevitable.</P>
    </article>
  );
}
