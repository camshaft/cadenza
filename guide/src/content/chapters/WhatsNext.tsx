// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, H1, H2, Lede, P } from "../../components/Prose.tsx";
import { Ch } from "../../components/ChapterLink.tsx";
import { Runnable } from "../../components/Runnable.tsx";

export default function WhatsNext() {
  return (
    <article>
      <H1>Where to go next</H1>
      <Lede>You've met the core of Cadenza. Here's what you can now read, write, and reason about.</Lede>
      <H2>What you've learned</H2>
      <P>You can bind values and write functions (<Ch to="/basics">Values &amp; functions</Ch>), pipe and compose them (<Ch to="/functions">Composing functions</Ch>), branch and recurse (<Ch to="/control-flow">Control flow</Ch>), compare and order (<Ch to="/ordering">Comparison &amp; ordering</Ch>), and bundle data into tuples and records, reading, nesting, and functionally updating it (<Ch to="/data">Tuples &amp; records</Ch>, <Ch to="/records-tuples">Working with records &amp; tuples</Ch>). You decide by shape with <Ch to="/pattern-matching">pattern matching</Ch>, and you've met the collections, namely <Ch to="/lists">lists</Ch>, <Ch to="/maps-sets">maps &amp; sets</Ch>, and lazy <Ch to="/iterators">iterators &amp; ranges</Ch>, along with the text and binary types, <Ch to="/strings">strings</Ch> (down to the individual <C>Char</C>s that compose them), <Ch to="/bytes">bytes</Ch> (and <Ch to="/binary-matching">binary matching</Ch> to build and destructure them), and <Ch to="/symbols">symbols</Ch>. You handle <Ch to="/errors">absence and errors</Ch> with <C>Option</C>/<C>Result</C>, and you know how Cadenza's numbers behave, from checked <Ch to="/numbers">integers</Ch>, <Ch to="/sized-integers">sized integers</Ch>, <Ch to="/floats">floating-point numbers</Ch>, and <Ch to="/rationals">exact fractions</Ch>, none of them ever converting silently.</P>
      <P>And you've seen what makes Cadenza its own language: <Ch to="/effects">effects &amp; handlers</Ch>, where a handler decides what a performed operation means; <Ch to="/contracts">design by contract</Ch>, where <C>@requires</C> and <C>@ensures</C> turn a function's assumptions and promises into checks enforced at its boundary; <Ch to="/modules">modules</Ch> that are just records of their exports; <Ch to="/opaque-types">opaque types</Ch> that hide a representation behind a module boundary so an invariant can't be broken; <Ch to="/units">units of measure</Ch> that catch a dimensional mistake at compile time and then vanish before the program runs; <Ch to="/types-as-values">types as values</Ch> you can reflect, compare, and branch on at compile time; <Ch to="/ad-hoc-polymorphism">ad-hoc polymorphism</Ch> where one operator name dispatches on operand type and a generic function specializes per type, all resolved at compile time; <Ch to="/const-parameters">const parameters</Ch> that fix an argument at compile time, whether a constant, a type, or a dictionary, inlined into a specialized copy and then erased; <Ch to="/metaprogramming">metaprogramming</Ch> where a program is an ordinary AST value you can quote, take apart, build, and eval; <Ch to="/property-testing">testing</Ch>, where <C>@test</C> marks a function as a test and parameters make it generative, so the runner synthesizes the inputs and shrinks any failure, no testing DSL required.</P>
      <P>More importantly, you've seen the ideas underneath, the <Ch to="/philosophy">tenets</Ch> that explain <em>why</em> the language makes the choices it does. Determinism and capability-safety holding for every program. Declining rather than miscompiling. One program, many syntaxes. Refusing the ambiguous conversion. These aren't trivia; they're the through-line, and they'll make the rest of Cadenza feel predictable.</P>
      <H2>One last program</H2>
      <P>A little of everything: a recursive helper, a comparison, and a record, computing the larger coordinate of a point, doubled.</P>
      <Runnable
        source={`(def (max a b) (if (> a b) a b))

(def (main) (let ((p #record((= x 3) (= y 8)))) (* 2 (max p.x p.y))))`}
      />
      <P>Edit it, changing the point or the factor, or swapping <C>max</C> for <C>min</C>, and Run. Then flip the syntax toggle in the header and watch the very same program re-appear in the other surface.</P>
      <H2>Keep exploring</H2>
      <P>The best way forward is to keep an example open and change it. Every code block in this guide is a scratchpad, so break it, fix it, and press Run. That loop, with the compiler answering instantly, is the fastest way to build a feel for the language.</P>
      <P>When you outgrow a single code block, move to the <Ch to="/playground">playground</Ch>: a full editor with a REPL for calling your functions one at a time, a peek at the WebAssembly and Rust your code compiles to, and a Share button that packs a whole program into a link.</P>
      <P>And to see the language doing real work, browse the <Ch to="/example-apps">example applications</Ch>: the calculator, CAD preview, notebook, and playground, each a differentiator you learned running as a full program.</P>
      <P>Finally, if you want to see where the language leads, there's a second pillar to this guide: <Ch to="/platform-overview">Cadenza the Platform</Ch>. It's the agent kernel Cadenza was built to run on, where an agent's whole state is a pure fold over an event log and the language is what flows through it, so the effects, sum types, and total functions you just met become the way a real agent records its history and replays it. It's a concept tour rather than more language to learn, so read it when you're curious how these ideas add up to a platform.</P>
    </article>
  );
}
