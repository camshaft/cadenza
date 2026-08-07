import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Link } from "react-router-dom";

/// "Cadenza the Platform" pillar, section B — events & state. Concept-level (kernel early-stage, content
/// light per operator). Sources, all DECIDED sections of v-agent-harness's design/agent-harness-kernel.md:
/// §4 (session-attached KV, reducer stateless between events, snapshots-free via persistent map) and §4b
/// (storage tiers + the immutable-by-hash vs. mutable-current-view bridge rule). Deliberately does NOT
/// claim the cross-version replay contract (§16b-B is an OPEN gap audit) — determinism here is stated at
/// the already-established "fold over recorded events" level. Follows PlatformOverview in the pillar.
export default function PlatformState() {
  return (
    <article>
      <H1>Events &amp; state</H1>
      <Lede>
        The{" "}
        <Link to="/platform-overview" className="text-cadenza-300 underline-offset-2 hover:underline">
          overview
        </Link>{" "}
        said an agent's state is a fold over its event log. That raises a fair question: if the state is
        <em> derived</em> from the log every time, isn't recomputing it from scratch on every event
        hopelessly slow, and where does a running agent actually keep what it knows? The platform's answer
        is a small, careful state model, and it's what keeps the pure-fold idea practical.
      </Lede>

      <H2>The fold, concretely</H2>
      <P>
        Before the practicalities, it's worth seeing the fold as a real program, because the whole platform
        is built on it and it's smaller than you'd expect. Picture an agent driving an account. Every action
        it takes is appended to a log as an event, a <C>Deposit</C> or a <C>Withdraw</C>, and the current
        balance isn't stored separately: it's what you get by <em>replaying</em> the log from the start.
        Replaying is a fold, written as a plain recursion over the events:
      </P>
      <Runnable
        source={`(type Event (Deposit Int64) (Withdraw Int64))
(def (replay log acc)
  (match log
    ((list) acc)
    ((list e .. rest)
      (match e
        ((Deposit n) (replay rest (+ acc n)))
        ((Withdraw n) (replay rest (- acc n)))))))
(def (main)
  (replay (list (Deposit 100) (Withdraw 30) (Deposit 5)) 0))`}
      />
      <P>
        The balance is <C>75</C>, computed straight from the history, since the log <em>is</em> the state
        and there's nothing separate to keep in sync with it. And because that history is an ordinary list,
        the three moves the overview promised fall out as ordinary things you do with the fold. <em>Replay</em>{" "}
        is the fold above. <em>Fork</em> is folding a <em>prefix</em>: replay only the first two events and
        you get the balance as it stood before the last deposit, a divergent timeline you carry forward
        without touching the original. <em>Recover</em> is folding from a <em>cursor</em>: record how far the
        agent got plus the balance there, and on restart replay only the events after that point to arrive at
        the same answer, so a crashed agent resumes exactly where it left off and never performs an action
        twice. All three are the same fold, applied to a value that happens to be a program's whole past.
      </P>
      <Note>
        replay = fold the whole log · fork = fold a prefix · recover = fold from a saved cursor
        <br />
        no snapshot format, no checkpoint machinery, just three things you do with one recursion over a list
      </Note>
      <P>
        The miniature above is the model exactly. In production the same fold runs over a real, durable log
        on disk whose events aren't toy deposits but genuine host actions the agent performed, a message
        received, a file written, a tool invoked, and recovering or replaying the agent is this exact fold
        over that history. The rest of this chapter is how the platform keeps that fold <em>practical</em> at
        scale.
      </P>

      <H2>The reducer is stateless between events</H2>
      <P>
        A reducer holds nothing of its own between events. Each time one arrives, it runs against a{" "}
        <em>key-value view</em> the kernel hands it, reads what it needs, decides on its effects, and writes
        back. Then it's gone until the next event. There's no long-lived memory hidden inside the program,
        which is exactly what makes the whole thing portable: an agent is just its log plus that key-value
        state plus the hash of its reducer, three values you can pick up and run anywhere.
      </P>
      <Note>
        state an agent carries = (its event log) + (a key-value view) + (the hash of its reducer program)
        <br />
        all three are ordinary content-addressed values → ship them and any worker can run the next fold
      </Note>
      <P>
        That the state lives <em>outside</em> the reducer, in a store the kernel owns, is what lets the
        kernel snapshot and move an agent without the program ever knowing. The reducer can't tell whether
        it just woke fresh from a saved checkpoint or has been running all along; either way it reads the
        same key-value view.
      </P>

      <H2>Snapshots come for free</H2>
      <P>
        Here's where the platform reuses a piece of the language you've already met. The key-value store is
        a{" "}
        <Link to="/maps-sets" className="text-cadenza-300 underline-offset-2 hover:underline">
          persistent map
        </Link>
        , the same immutable, structurally-shared map from the collections chapter, where an update yields a
        new version that shares almost everything with the old one. So after folding each event, the kernel
        already holds a complete, valid version of the state, at no extra cost. A snapshot isn't a thing the
        reducer builds; it's just remembering which of those free versions to keep.
      </P>
      <P>
        Checkpointing turns from a compute problem into a retention choice: you have a valid checkpoint at
        every single event, and all you decide is how many to hold on to (every version while an agent is
        hot, a sparse few once it cools). The same immutability that makes Cadenza's collections cheap to
        copy makes an agent cheap to checkpoint.
      </P>

      <H2>Two ways to read, one rule</H2>
      <P>
        An agent doesn't live alone; it needs to read things beyond its own log, another agent's status, a
        shared note, a past snapshot. The platform allows exactly two kinds of read, split by one rule that
        protects the fold:
      </P>
      <P>
        <strong>Something immutable, addressed by its hash</strong>, a past event, a stored snapshot, a
        reducer's WebAssembly, can be read directly. The bytes behind a hash never change, so reading them
        is always safe and always gives the same answer: immutability <em>is</em> determinism, so no
        recorded event is needed.
      </P>
      <P>
        <strong>Something that changes, a "what's true right now" question</strong>, "is that agent still
        working?", "which notes about this topic are current?", must instead go through the kernel as a{" "}
        query, and its answer is folded into the log as an event. A live peek at mutable state would wreck
        replay, since the answer could differ next time; freezing the answer into the log the moment it's
        asked makes it as replayable as any other recorded fact. It's the same move the{" "}
        <Link to="/effects" className="text-cadenza-300 underline-offset-2 hover:underline">
          effects
        </Link>{" "}
        chapter made for reading the outside world: reach out once, record what came back, and every replay
        reads the record.
      </P>
      <Note>
        immutable, by hash (past event, snapshot, reducer wasm) → read directly, no event, replay-safe
        <br />
        mutable, "what's true now" (a peer's status, current notes) → query effect → answer frozen into the
        log
      </Note>
      <P>
        One rule, cleanly drawn, and the crown-jewel property survives contact with a whole system of
        agents reading each other: everything a fold depends on is either an unchanging value or a recorded
        answer, so an agent's history always replays to exactly the same place. The next sections build on
        this: what an agent is allowed to do, and how the kernel runs many of them at once.
      </P>
    </article>
  );
}
