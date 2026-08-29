import { H1, Lede, H2, P, C } from "../../components/Prose.tsx";
import { Why } from "../../components/Why.tsx";

export default function Philosophy() {
  return (
    <article>
      <H1>Why Cadenza is the way it is</H1>
      <Lede>
        A short tour of the ideas underneath the language. You don't need these to write your first
        program, but they explain almost every decision you'll meet later.
      </Lede>

      <P>
        Most languages are shaped by their history. Cadenza is shaped by a single question:{" "}
        <em>what would a language look like if its author were as often a machine as a person, and if
        being able to trust a program mattered more than being able to write it quickly?</em> Nearly
        everything below falls out of taking that question seriously.
      </P>

      <H2>Trust is built in, not added afterward</H2>
      <P>
        Two properties hold for every Cadenza program; they aren't features you opt into, and no compiler
        flag can turn them off.
      </P>

      <Why tenet="Determinism by construction">
        A Cadenza program can't secretly depend on the clock, the weather, or a random number. It's not
        that those things are forbidden, but that any outside influence must be a <em>declared
        capability</em>, so a program's determinism is readable straight off its manifest. The compiler
        adds none of its own nondeterminism either. A happy side effect: because a run is a pure function
        of its inputs, you get perfect replay, the exact thing time-travel debuggers work so hard to
        fake.
      </Why>

      <Why tenet="Capability-safety holds for every program">
        A program declares every outside operation it might perform, and the component it compiles to
        imports exactly those and <em>nothing else</em>. There's no ambient authority to reach for,
        because the means to reach anything undeclared simply isn't present in the compiled component.
        So the manifest is both a description and an enforcement boundary: whoever runs a component can
        decide what to permit from the manifest alone.
      </Why>

      <H2>The source is what's real</H2>
      <Why tenet="The source is the truth; the component is a projection">
        A program's meaning lives in its source. The runnable component is a rebuildable,
        content-addressed <em>function</em> of that source, so a defect is fixed in the source and
        recompiled, never by patching the binary. The same source always derives the same bytes, on any
        conforming machine, and when an optimization would break that byte-for-byte reproduction,
        reproduction wins.
      </Why>

      <Why tenet="One meaning, checked against one oracle">
        Every construct's behavior is defined in exactly one runnable place, a conformance corpus of
        example programs with their expected results. Every compiler is checked against that corpus
        rather than becoming a second, independent definition of the language. It's the structural fix
        for a language whose meaning could otherwise drift between a document, an interpreter, and an
        implementation. In the same spirit, the <em>specification</em> is the durable artifact and the
        compiler is disposable, a regenerable projection of the spec.
      </Why>

      <H2>Refuse rather than guess</H2>
      <Why tenet="Decline, don't miscompile">
        Cadenza ranks outcomes by safety: a wrong answer is worse than a crash, a crash is worse than an
        honest refusal, and a correct answer is best. So a compiler that isn't sure it can compile your
        program <em>correctly</em> declines instead of emitting plausible-but-wrong code. There's a
        subtle asymmetry too: wrongly rejecting a good program is <em>worse</em> than failing to reject a
        bad one, because it denies a correct program its meaning, so a check it can't prove stays silent
        rather than rejecting.
      </Why>

      <P>
        You'll see these ideas show up again and again as concrete features, from safe indexing that returns
        an <C>Option</C>, to numbers that refuse to silently convert, to a <C>match</C> that insists on
        patterns. Each chapter flags the tenet at work in a <em>✦ Why it's this way</em> box. Keep an eye
        out for them.
      </P>

      <P>
        Enough <em>why</em>, so time to write some. The next chapter, <em>Values &amp; functions</em>, starts
        from the smallest pieces: literals, bindings, and functions the compiler types for you.
      </P>
    </article>
  );
}
