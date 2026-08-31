// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Ch } from "../../components/ChapterLink.tsx";
import { Runnable } from "../../components/Runnable.tsx";

export default function PlatformExecution() {
  return (
    <article>
      <H1>The execution model</H1>
      <Lede>One kernel runs many agents at once, and each is a fold that must replay identically forever. Those two facts seem to pull against each other: running things concurrently invites nondeterministic timing, yet replay is the platform's whole value. This last concept chapter is how they coexist, and why an agent here can never really "get stuck."</Lede>
      <H2>A session is the unit</H2>
      <P>Everything runs as a <em>session</em>: one agent's append-only log, its key-value state, its reducer, its capabilities. A session is deliberately small, a single agent doing one bounded task, because a session is the unit of replay, migration, and sandboxing. The kernel is a multiplexer: it hosts many independent sessions at once, and "deploy once" means one kernel quietly running a great many of them.</P>
      <P>Sessions don't share mutable state. The only way one touches another is the logged, authorized effect from <Ch to="/platform-safety"> the last chapter </Ch> , so the kernel can interleave them freely without one corrupting another. That isolation is what makes running them concurrently safe to begin with.</P>
      <H2>Nothing polls; an append wakes the reducer</H2>
      <P>The scheduling model is one sentence: <em>appending an event runs the reducer</em>. There's no polling loop anywhere. A reducer folds a single event and returns, so there's no long-running turn that can hang mid-stream, which means "stuck" isn't a state a session can even be in. A session is ever only waiting in one of two ways, each with a clean escape:</P>
      <Note>waiting on an outstanding effect (a slow model call, a hung command) <br /> → every effect carries a deadline; no result in time → the kernel injects a timeout event → the reducer wakes to recover <br /> idle, waiting on input <br /> → any message is an append, and an append wakes the reducer; idle costs nothing and revives instantly</Note>
      <P>This is worth dwelling on, because it's the fix for a very real pain. Today's agents "get stuck" because each is a single long-running turn that can wedge partway through, and nothing outside can nudge it without an elaborate watchdog. In the fold model there's no partway to wedge in: the reducer has already returned, and the session is simply waiting on a named thing that either has a deadline or wakes on a message. Recovery is an injected event, not a kill-and-restart. (This guide's own fleet still polls on a timer only because it predates a kernel that can deliver-and-wake.)</P>
      <H2>Concurrency without losing determinism</H2>
      <P>Running many sessions at once means the runtime interleaves them, pausing one to let another progress. That timing is genuinely nondeterministic. So how does an agent still replay to exactly the same place? Because <strong>determinism lives in the log, not in the scheduler</strong>.</P>
      <P>A session folds its events in the order the log records, and that order is a recorded fact, not a product of who-ran-when. When two effects are in flight, either may finish first in wall-clock time, but the kernel writes each result into the log as it lands, freezing that order. Replay reads the frozen order; it never re-runs or re-races the effects. The scheduler is free to interleave, pause, and meter however it likes, because none of that is a fold input, only the recorded events are.</P>
      <P>It's the same discipline from <Ch to="/platform-state"> Events &amp; state </Ch> , now carried all the way up to a concurrent runtime: nondeterminism is allowed at the edges, as long as its outcome is recorded, so the fold in the middle stays pure. That single rule, held from a one-line fold up to a kernel multiplexing many agents, is what the whole platform is built to protect.</P>
      <H2>An agent is just a reducer</H2>
      <P>Everything so far has been about running <em>a reducer</em> over an event log. Here's where it leads, and it's the point of the whole platform: <strong>an AI agent is one of those reducers</strong>, with no new machinery at all. The turn-taking loop you'd expect to be the agent's special engine is, once again, just a fold over effects.</P>
      <P>Trace one turn. A message arrives as an event; the reducer folds it and, to think, <em>emits a model call as an effect</em>. The kernel authorizes and performs it, and folds the model's reply back in as the next event. If that reply asks to use tools, each tool call is itself an effect the reducer emits, the kernel runs them (through the very same authorization gate as any other effect), and their results fold back in. The reducer emits another model call with those results, and the cycle repeats until the model ends the turn. Inbound, model, tools, model, done: every step is an effect requested and an event folded back, so the "agent loop" is not a loop the runtime hardcodes, it's what the fold naturally does when the effects happen to be a model and its tools.</P>
      <Note>event in → reducer emits a model-call effect → fold reply → reply wants tools? emit each as an effect → fold results → emit model-call again → … → end-of-turn <br /> no bespoke agent runtime: the loop is itself the fold; the model and tools are ordinary authorized effects on the log</Note>
      <P>You can watch a whole turn happen as one fold, and it's worth splitting into two files so the boundary is explicit. <C>events</C> is the turn's history: the messages that arrived, as plain data, a task coming in, the model replying that it wants a tool, the tool's result, the model ending the turn. <C>reducer</C> is the behavior: the fold that consumes that history and decides what to do, here just accumulating a trace of each step. Fixtures stand in for the live model and tools, so the whole thing runs in your browser with no network, and the split is the same one a real agent lives by: its history is one thing, the program that folds it is another.</P>
      <Runnable
        files={[
          {
            name: "events",
            source: `(do
  (def
    turn
    #list(#record((= kind #"task") (= val "count files"))
      #record((= kind #"model") (= val "shell"))
      #record((= kind #"tool") (= val "3"))
      #record((= kind #"done") (= val "there are 3"))))

  (export turn))`,
            surface: "sexpr",
          },
          {
            name: "reducer",
            source: `(do
  (import "events" (turn))

  (def
    (step acc e)
    (let
      ((k e.kind) (v e.val))
      (if
        (= k #"task")
        (String.concat acc "asked-model; ")
        (if
          (= k #"model")
          (String.concat acc (String.concat "run-tool:" (String.concat v "; ")))
          (if
            (= k #"tool")
            (String.concat acc (String.concat "folded-result:" (String.concat v "; ")))
            (String.concat acc (String.concat "done:" v)))))))

  (def (run xs acc) (match xs (#list() acc) (#list(e (.. rest)) (run rest (step acc e)))))

  (def (main) (run turn ""))

  (export main))`,
            surface: "sexpr",
            entry: true,
          },
        ]}
        expected={`(: "asked-model; run-tool:shell; folded-result:3; done:there are 3" String)`}
        expect="value"
      />
      <P>Because the turn is nothing but recorded events, everything the platform promised comes along for free here too. The agent replays deterministically (the model's replies and tool results are facts on the log, not re-run), you can fork a conversation from any point, and a crash mid-turn resumes from the last recorded event without re-calling the model or re-running a tool. An agent's entire existence, its whole reasoning history, is an ordinary value you can inspect, branch, and hand to the next generation of itself.</P>
      <H2>When an agent fails, the failure is just another event</H2>
      <P>The hard part of running agents was never the happy path; it's what happens when one breaks: a tool errors, a fold traps, a whole sub-agent dies. On most platforms that's an exception unwinding a stack, or a process that vanishes and takes its state with it. Here a failure has nowhere to unwind <em>to</em>, because the agent is a fold: there's no stack in flight, only a log. So a failure becomes what everything else already is, an ordered event appended to the log. A fold that traps is recorded as a <C>FoldFailed</C> event, not a crash that loses the session, and it's recorded rather than fed straight back to the same reducer that just failed on it. The session is still a well-defined value; it simply has a failure in its history.</P>
      <P>Because a failure is an event, another agent can watch for it. An agent can <em>spawn</em> a child, and the child's identity is fixed the moment it's born: its id is a content hash of its starting state, so it can never be confused with another. The parent-child link is recorded on both sides. A child ends its life one of two ways, and the platform delivers a matching message to the parent, which its reducer folds like any other event so it lands on the parent's log carrying the child's outcome. When a child is <em>terminated</em> from outside, the parent receives a <C>child-exited</C> message, always a failure close, since a termination is not a completion. When a child <em>self-closes</em>, its own reducer returning a close outcome, the platform reaps it and delivers a <C>child-completed</C> message instead, carrying either a success with a payload or a failure with a reason. Both outcomes are the same structured value; the difference is only which signal fires, so a supervisor can tell an orderly finish from a forced kill. The parent's reducer folds whichever it gets and decides what to do next: re-spawn, retry, escalate, aggregate, or give up. That is a supervision tree, and unlike the in-memory restart tables of older systems it's durable and replayable: the whole parent-and-children engagement is one causally-linked history you can migrate, sandbox, or audit as a unit. (A root session with no parent that self-closes is simply reaped, with no signal to deliver.)</P>
      <Note>a fold that traps → recorded as a FoldFailed event (not a lost session), readable by a supervisor <br /> a child TERMINATED from outside → a child-exited message (always a failure close) on its parent's log <br /> a child that SELF-CLOSES → a child-completed message carrying its outcome (success payload or failure reason) <br /> the parent's reducer folds the outcome and chooses: re-spawn · retry · escalate · aggregate · give up</Note>
      <P>Notice the line between what the platform provides and what it doesn't. The platform makes every failure an ordered, foldable event a supervisor can see, an in-session trap becomes a <C>FoldFailed</C> event, a terminated child becomes a <C>child-exited</C> event and a self-closed one a <C>child-completed</C> event on its parent, so nothing fails silently into the void and nothing needs a human to jump-start it. But <em>what to do</em> about a failure, back off and retry a transient error, escalate a permanent one, restart from a clean checkpoint, is ordinary reducer logic the supervisor's author writes; the kernel hardcodes no strategy. So the platform is the self-heal <em>substrate</em>, a system where a supervisor <em>can</em> recover from failure without a manual restart, and the healing policy is just more Cadenza in the supervising reducer. Effect failures come typed for exactly this: an effect error carries whether it's retryable or permanent, so a supervisor can back off a transient model error and fail fast on one that will never succeed. Where this leads, still ahead, is a reusable library of supervisor strategies and worked multi-level trees (a planner agent spawning workers that spawn their own); the primitives underneath are all here now.</P>
      <H2>Where this leaves you</H2>
      <P>Four ideas carry the platform: a kernel that knows nothing and runs a <Ch to="/platform-overview"> fold over an event log </Ch> ; <Ch to="/platform-state"> state </Ch> that's a derived projection with free snapshots; effects as the <Ch to="/platform-safety"> one gate </Ch> where capability and safety live; and an execution model that runs many agents concurrently while keeping every one of them perfectly replayable. And what all four add up to: an agent itself is just a reducer folding model calls and tool calls as effects, so nothing about running an AI agent needs machinery beyond the fold, right down to its failures, which are just more events a supervisor folds and recovers from. The kernel is early, and this section will grow with it, eventually into something you can run and inspect in the browser the way you can the language. For now, you've seen the shape of the thing: the same idea that made a value out of a program's history, scaled into a runtime for agents built in Cadenza.</P>
      <P>One thing is left: writing a reducer yourself. The <Ch to="/writing-a-reducer"> next chapter </Ch> turns the model concrete, building a real agent-harness reducer in Cadenza, from the empty reducer up to one that reads state and requests effects, where the language you learned in the first pillar becomes the language an agent runs on.</P>
    </article>
  );
}
