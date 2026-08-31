// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (xtask-codegen-guide).
import { C, H1, H2, Lede, Note, P } from "../../components/Prose.tsx";
import { Ch } from "../../components/ChapterLink.tsx";
import { Runnable } from "../../components/Runnable.tsx";

export default function PlatformState() {
  return (
    <article>
      <H1>Events & state</H1>
      <Lede>The <Ch to="/platform-overview"> overview </Ch> said an agent's state is a fold over its event log. That raises a fair question: if the state is <em> derived</em> from the log every time, isn't recomputing it from scratch on every event hopelessly slow, and where does a running agent actually keep what it knows? The platform's answer is a small, careful state model, and it's what keeps the pure-fold idea practical.</Lede>
      <H2>The fold, concretely</H2>
      <P>Before the practicalities, it's worth seeing the fold as a real program, because the whole platform is built on it and it's smaller than you'd expect. Picture an agent driving an account. Every action it takes is appended to a log as an event, a <C>Deposit</C> or a <C>Withdraw</C>, and the current balance isn't stored separately: it's what you get by <em>replaying</em> the log from the start. Replaying is a fold, written as a plain recursion over the events:</P>
      <Runnable
        source={`(type Event (Deposit Int64) (Withdraw Int64))

(def
  (replay log acc)
  (match
    log
    (#list() acc)
    (#list(e (.. rest))
      (match e ((Deposit n) (replay rest (+ acc n))) ((Withdraw n) (replay rest (- acc n)))))))

(def (main) (replay #list((Deposit 100) (Withdraw 30) (Deposit 5)) 0))`}
      />
      <P>The balance is <C>75</C>, computed straight from the history, since the log <em>is</em> the state and there's nothing separate to keep in sync with it. And because that history is an ordinary list, the three moves the overview promised fall out as ordinary things you do with the fold. <em>Replay</em> is the fold above. <em>Fork</em> is folding a <em>prefix</em>: replay only the first two events and you get the balance as it stood before the last deposit, a divergent timeline you carry forward without touching the original. <em>Recover</em> is folding from a <em>cursor</em>: record how far the agent got plus the balance there, and on restart replay only the events after that point to arrive at the same answer, so a crashed agent resumes exactly where it left off and never performs an action twice. All three are the same fold, applied to a value that happens to be a program's whole past.</P>
      <Note>replay = fold the whole log · fork = fold a prefix · recover = fold from a saved cursor <br /> no snapshot format, no checkpoint machinery, just three things you do with one recursion over a list</Note>
      <P>The miniature above is the model exactly. In production the same fold runs over a real, durable log on disk whose events aren't toy deposits but genuine host actions the agent performed, a message received, a file written, a tool invoked, and recovering or replaying the agent is this exact fold over that history. The rest of this chapter is how the platform keeps that fold <em>practical</em> at scale.</P>
      <H2>The reducer is stateless between events</H2>
      <P>A reducer holds nothing of its own between events. Each time one arrives, it runs against a <em>key-value view</em> the kernel hands it, reads what it needs, decides on its effects, and writes back. Then it's gone until the next event. There's no long-lived memory hidden inside the program, which is exactly what makes the whole thing portable: an agent is just its log plus that key-value state plus the hash of its reducer, three values you can pick up and run anywhere.</P>
      <Note>state an agent carries = (its event log) + (a key-value view) + (the hash of its reducer program) <br /> all three are ordinary content-addressed values → ship them and any worker can run the next fold</Note>
      <P>That the state lives <em>outside</em> the reducer, in a store the kernel owns, is what lets the kernel snapshot and move an agent without the program ever knowing. The reducer can't tell whether it just woke fresh from a saved checkpoint or has been running all along; either way it reads the same key-value view.</P>
      <H2>Snapshots cost nothing extra</H2>
      <P>Here's where the platform reuses a piece of the language you've already met. The key-value store is a <Ch to="/maps-sets"> persistent map </Ch> , the same immutable, structurally-shared map from the collections chapter, where an update yields a new version that shares almost everything with the old one. So after folding each event, the kernel already holds a complete, valid version of the state, at no extra cost. A snapshot isn't a thing the reducer builds; it's just remembering which of those free versions to keep.</P>
      <P>Checkpointing turns from a compute problem into a retention choice: you have a valid checkpoint at every single event, and all you decide is how many to hold on to (every version while an agent is hot, a sparse few once it cools). The same immutability that makes Cadenza's collections cheap to copy makes an agent cheap to checkpoint.</P>
      <H2>Two ways to read, one rule</H2>
      <P>An agent doesn't live alone; it needs to read things beyond its own log, another agent's status, a shared note, a past snapshot. The platform allows exactly two kinds of read, split by one rule that protects the fold:</P>
      <P><strong>Something immutable, addressed by its hash</strong>, a past event, a stored snapshot, a reducer's WebAssembly, can be read directly. The bytes behind a hash never change, so reading them is always safe and always gives the same answer: immutability <em>is</em> determinism, so no recorded event is needed.</P>
      <P><strong>Something that changes, a "what's true right now" question</strong>, "is that agent still working?", "which notes about this topic are current?", must instead go through the kernel as a query, and its answer is folded into the log as an event. A live peek at mutable state would wreck replay, since the answer could differ next time; freezing the answer into the log the moment it's asked makes it as replayable as any other recorded fact. It's the same move the <Ch to="/effects"> effects </Ch> chapter made for reading the outside world: reach out once, record what came back, and every replay reads the record.</P>
      <Note>immutable, by hash (past event, snapshot, reducer wasm) → read directly, no event, replay-safe <br /> mutable, "what's true now" (a peer's status, current notes) → query effect → answer frozen into the log</Note>
      <P>One rule, cleanly drawn, and replayability survives contact with a whole system of agents reading each other: everything a fold depends on is either an unchanging value or a recorded answer, so an agent's history always replays to exactly the same place.</P>
      <H2>Finding another agent</H2>
      <P>Reading another agent's status assumes you can already <em>refer</em> to it. So how does one agent name another? The platform gives them a shared name service: an agent registers under a stable name, and another resolves that name to the target's session. A session's identity is a content hash of its starting state, so a name resolves to exactly one session with no ambiguity, and the name outlives any particular run. Resolve it again later and you find the same agent, or its successor if the name has since been repointed, never a stale address.</P>
      <P>A name isn't a free-for-all, and this is where addressing meets the safety model. Names are authority-scoped by their prefix: who may point a name at a session is gated by the same capability check as any other effect, so only a holder of authority over <C>system/</C> may set a <C>system/…</C> name. That's what makes a well-known name trustworthy. Resolving <C>system/compiler/latest</C> can't be silently hijacked to point at an impostor, because repointing it takes an authority almost no one holds. And resolution <em>freezes</em>: when an agent resolves a name, the answer it got is recorded into its own log, so a later repoint can't retroactively change what a resolver already acted on. It's the same rule as reading anything else that changes, reach out once, record the answer, and every replay reads the record, now applied to addresses.</P>
      <P>Naming generalizes from one session to a set. A <em>group</em> name holds a collection of live members: an agent adds itself or is added, it can leave, and resolving the group returns the whole current membership. That gives two things at once, a directory (who is in this group right now?) and multicast (send one message to every member). A supervisor with a pool of workers resolves the group and fans a message out to all of them; a worker joining or finishing changes the membership the next resolve sees. Because membership is itself folded from add and remove events on the group's log, it merges cleanly even when several members join at the same moment, with no lost writes, and the group is as replayable and auditable as any other agent state.</P>
      <Note>a name → one session (its identity is a content hash of its starting state), authority-scoped by prefix <br /> resolve freezes the answer into your log → a later repoint can't rewrite what you already acted on <br /> a group name → a live set of members you add/remove and multicast to: a directory and a broadcast at once</Note>
      <P>That closes the loop opened at the top of the chapter. An agent's own state is a fold over its log; reading the changing world is a query frozen into that log; and now even <em>who</em> the other agents are is just another name resolved and recorded. The next sections build on all of it: what an agent is allowed to do, and how the kernel runs many of them at once.</P>
    </article>
  );
}
