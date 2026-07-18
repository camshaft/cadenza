import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Why } from "../../components/Why.tsx";
import { Link } from "react-router-dom";

/// A router-aware internal link to another chapter (respects the Pages sub-path basename).
function Ch({ to, children }: { to: string; children: React.ReactNode }) {
  return (
    <Link to={to} className="text-cadenza-300 underline-offset-2 hover:underline">
      {children}
    </Link>
  );
}

export default function CdzToolchain() {
  return (
    <article>
      <H1>The cdz toolchain</H1>
      <Lede>
        Every example in this guide has run the real compiler in your browser. That same compiler is one
        binary on your <C>PATH</C>, called <C>cdz</C>, and it carries the whole loop from a fresh project
        to a running program.
      </Lede>

      <P>
        There is no separate toolchain to assemble, because <C>cdz</C> both compiles and runs Cadenza and
        also provides a project workflow and a set of code-understanding queries. You met the pieces
        interactively already, so this chapter is about reaching for them at a terminal.
      </P>

      <H2>Compile and run, in one binary</H2>
      <P>
        The smallest loop is a single file. <C>cdz compile</C> lowers a program to a WebAssembly component
        and <C>cdz run</C> executes one, and because a compiled component is just a value you can pipe the
        one straight into the other.
      </P>
      <Note>
        <C>{`$ cdz compile foo.cdz -o - | cdz run -`}</C>
        <br />
        <C>{`$ cdz run foo.wasm --call add --arg 2 --arg 40   # prints 42`}</C>
      </Note>
      <P>
        A run is bounded so a mistake fails loudly rather than hanging: it stops at a wall-clock deadline
        (30 seconds by default) and traps, which you can raise or lower with{" "}
        <C>CDZ_RUN_TIMEOUT_SECS</C> or disable with <C>=0</C> when you're stepping through a debugger. The
        exit code tells a script what happened, since an operational failure or a trap exits <C>1</C> while
        a usage mistake exits <C>2</C>.
      </P>

      <H2>A project, the cargo way</H2>
      <P>
        Past a single file you work in a project, and the commands mirror the ones you'd reach for in
        Cargo. <C>cdz new</C> scaffolds one with a <C>Project.cdz</C> manifest, an entry file, and a
        passing starter <C>@test</C>, so the project is green from the very first command.
      </P>
      <Note>
        <C>{`$ cdz new my-app`}</C>
        <br />
        <C>{`$ cd my-app`}</C>
        <br />
        <C>{`$ cdz build     # compile the whole project into one component`}</C>
        <br />
        <C>{`$ cdz run       # run it`}</C>
        <br />
        <C>{`$ cdz test      # run the project's tests`}</C>
      </Note>
      <P>
        The manifest is itself Cadenza: <C>Project.cdz</C> starts with a <C>def name</C>, a{" "}
        <C>def entry</C>, and a <C>def tests</C>, and it grows a <C>def deps</C> once you take on a
        dependency. You add and drop path dependencies with <C>cdz add ../mathlib</C> and{" "}
        <C>cdz remove ../mathlib</C> rather than hand-editing the manifest,
        inspect the dependency graph with <C>cdz tree</C>, and re-run any command on every save with{" "}
        <C>cdz watch</C> for the same instant feedback you've had in this guide. When something looks off,{" "}
        <C>cdz doctor</C> checks the setup.
      </P>

      <H2>Designed for agents as much as for people</H2>
      <P>
        The <Ch to="/welcome">very first chapter</Ch> said Cadenza is written and read by AI agents as much
        as by humans, and the toolchain is where that shows. Alongside the build loop, <C>cdz</C> answers
        one-shot, structured questions about your code, so an agent (or you) can understand and modify a
        program without opening an editor at all.
      </P>
      <Note>
        <C>{`$ cdz type add foo.cdz        # the inferred type of a definition`}</C>
        <br />
        <C>{`$ cdz check foo.cdz           # diagnostics, without building`}</C>
        <br />
        <C>{`$ cdz uses / def / scope      # where a name is used, defined, in scope`}</C>
        <br />
        <C>{`$ cdz symbols / doc           # a file's definitions, and their docs`}</C>
        <br />
        <C>{`$ cdz fix foo.cdz             # apply the compiler's verified repairs`}</C>
      </Note>
      <P>
        These are the same facts the editor shows on hover and as squiggles, offered one query at a time
        so a script or an agent can consume them directly. For an editor that speaks the Language Server
        Protocol, <C>cdz lsp</C> serves all of it live over stdio, and <C>cdz calc</C> opens the same
        exact-arithmetic calculator you met earlier as a REPL. The point is that a human and an agent reach
        for the same tool and get the same compiler-backed answers, never a separate approximation that
        might drift.
      </P>

      <Why tenet="One compiler, everywhere you work">
        The browser guide, the command line, the editor, and an agent's query all run the one compiler,
        which is why what you saw in the playground is exactly what <C>cdz</C> does at your terminal.
        Because a single tool answers both "build this" and "tell me about this," there is no second
        implementation to disagree with the first, and the understanding an agent acts on is the same
        understanding that compiles the code.
      </Why>

      <P>
        That's the whole toolchain: one binary from a fresh <C>cdz new</C> to a running program, with the
        compiler's knowledge available as ordinary queries along the way. To recap the language itself and
        find where to go from here, head to <Ch to="/whats-next">Where to go next</Ch>.
      </P>
    </article>
  );
}
