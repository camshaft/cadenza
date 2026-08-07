import { H1, Lede, H2, P, Note } from "../../components/Prose.tsx";
import { Link } from "react-router-dom";

/// "Cadenza the Platform" pillar, section D (last planned concept chapter) — the execution model.
/// Concept-level (kernel early-stage, content light per operator). Sources = DECIDED sections of
/// v-agent-harness's design/agent-harness-kernel.md: §3 (sessions), §9d (reactive execution — append
/// wakes the reducer, no polling, deadline per effect), §22a (async substrate → many sessions
/// multiplexed), §22c (async must NOT break replay-determinism — determinism lives in the LOG not the
/// scheduler). DELIBERATELY DEFERS the in-flight impl: §22b async-blob-API, §22d/§22e gas/fuel mechanics
/// (v-agent-harness flagged these as still churning — do not pin their surface). Follows PlatformSafety.
export default function PlatformExecution() {
  return (
    <article>
      <H1>The execution model</H1>
      <Lede>
        One kernel runs many agents at once, and each is a fold that must replay identically forever. Those
        two facts seem to pull against each other: running things concurrently invites nondeterministic
        timing, yet replay is the platform's whole value. This last concept chapter is how they coexist,
        and why an agent here can never really "get stuck."
      </Lede>

      <H2>A session is the unit</H2>
      <P>
        Everything runs as a <em>session</em>: one agent's append-only log, its key-value state, its
        reducer, its capabilities. A session is deliberately small, a single agent doing one bounded task,
        because a session is the unit of replay, migration, and sandboxing. The kernel is a multiplexer: it
        hosts many independent sessions at once, and "deploy once" means one kernel quietly running a great
        many of them.
      </P>
      <P>
        Sessions don't share mutable state. The only way one touches another is the logged, authorized
        effect from{" "}
        <Link to="/platform-safety" className="text-cadenza-300 underline-offset-2 hover:underline">
          the last chapter
        </Link>
        , so the kernel can interleave them freely without one corrupting another. That isolation is what
        makes running them concurrently safe to begin with.
      </P>

      <H2>Nothing polls; an append wakes the reducer</H2>
      <P>
        The scheduling model is one sentence: <em>appending an event runs the reducer</em>. There's no
        polling loop anywhere. A reducer folds a single event and returns, so there's no long-running turn
        that can hang mid-stream, which means "stuck" isn't a state a session can even be in. A session is
        ever only waiting in one of two ways, each with a clean escape:
      </P>
      <Note>
        waiting on an outstanding effect (a slow model call, a hung command)
        <br />
        {"  "}→ every effect carries a deadline; no result in time → the kernel injects a timeout event → the reducer wakes to recover
        <br />
        idle, waiting on input
        <br />
        {"  "}→ any message is an append, and an append wakes the reducer; idle costs nothing and revives instantly
      </Note>
      <P>
        This is worth dwelling on, because it's the fix for a very real pain. Today's agents "get stuck"
        because each is a single long-running turn that can wedge partway through, and nothing outside can
        nudge it without an elaborate watchdog. In the fold model there's no partway to wedge in: the
        reducer has already returned, and the session is simply waiting on a named thing that either has a
        deadline or wakes on a message. Recovery is an injected event, not a kill-and-restart. (This guide's
        own fleet still polls on a timer only because it predates a kernel that can deliver-and-wake.)
      </P>

      <H2>Concurrency without losing determinism</H2>
      <P>
        Running many sessions at once means the runtime interleaves them, pausing one to let another
        progress. That timing is genuinely nondeterministic. So how does an agent still replay to exactly
        the same place? Because <strong>determinism lives in the log, not in the scheduler</strong>.
      </P>
      <P>
        A session folds its events in the order the log records, and that order is a recorded fact, not a
        product of who-ran-when. When two effects are in flight, either may finish first in wall-clock
        time, but the kernel writes each result into the log as it lands, freezing that order. Replay reads
        the frozen order; it never re-runs or re-races the effects. The scheduler is free to interleave,
        pause, and meter however it likes, because none of that is a fold input, only the recorded events
        are.
      </P>
      <P>
        It's the same discipline from{" "}
        <Link to="/platform-state" className="text-cadenza-300 underline-offset-2 hover:underline">
          Events &amp; state
        </Link>
        , now carried all the way up to a concurrent runtime: nondeterminism is allowed at the edges, as
        long as its outcome is recorded, so the fold in the middle stays pure. That single rule, held from a
        one-line fold up to a kernel multiplexing many agents, is what the whole platform is built to
        protect.
      </P>

      <H2>Where this leaves you</H2>
      <P>
        Four ideas carry the platform: a kernel that knows nothing and runs a{" "}
        <Link to="/platform-overview" className="text-cadenza-300 underline-offset-2 hover:underline">
          fold over an event log
        </Link>
        ; <Link to="/platform-state" className="text-cadenza-300 underline-offset-2 hover:underline">
          state
        </Link>{" "}
        that's a derived projection with free snapshots; effects as the{" "}
        <Link to="/platform-safety" className="text-cadenza-300 underline-offset-2 hover:underline">
          one gate
        </Link>{" "}
        where capability and safety live; and an execution model that runs many agents concurrently while
        keeping every one of them perfectly replayable. The kernel is early, and this section will grow with
        it, eventually into something you can run and inspect in the browser the way you can the language.
        For now, you've seen the shape of the thing: the same idea that made a value out of a program's
        history, scaled into a runtime for agents built in Cadenza.
      </P>
    </article>
  );
}
