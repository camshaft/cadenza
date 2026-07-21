import { H1, Lede, H2, P, C, Note } from "../../components/Prose.tsx";
import { Runnable } from "../../components/Runnable.tsx";
import { Why } from "../../components/Why.tsx";
import { Link } from "react-router-dom";

export default function RecordedLog() {
  return (
    <article>
      <H1>History as a value</H1>
      <Lede>
        Property testing hunted for the inputs that break a function. This last difference is about a
        different kind of running altogether: a long-lived agent that acts on the world over time. The guide
        opened by saying Cadenza programs are written and read by AI agents, and here is the half that makes
        that literal. An agent's whole history is an ordinary <em>event log</em>, and running the agent is a
        fold over that log, so replaying it, forking it, and recovering from a crash are all just re-folding
        a value.
      </Lede>

      <H2>The log is the source of truth</H2>
      <P>
        Picture an account the agent drives. Every action it takes is appended to a log as an event, a{" "}
        <C>Deposit</C> or a <C>Withdraw</C>, and the current balance isn't stored anywhere separately: it's
        what you get by <em>replaying</em> the log from the start. Replaying is a fold, and since the prelude
        keeps lists lean (there's no <C>List.fold</C>, just <C>at</C>, <C>len</C>, and <C>push</C>), you
        write the fold as a plain recursion over the events:
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
        The balance is <C>75</C>, computed straight from the history: <C>100</C> in, <C>30</C> out,{" "}
        <C>5</C> in. There's no separate state to keep in sync with the log, because the log <em>is</em> the
        state. That single idea, the value is a fold over its history, is the whole chapter; replay, fork,
        and recovery are just three things you do with the fold.
      </P>

      <H2>Fork: branch the timeline</H2>
      <P>
        Because the history is an ordinary list, a <em>what if</em> is just replaying a <em>prefix</em> of
        it. Fold only the first two events and you get the balance as it stood before the last deposit, a
        divergent timeline you could carry forward differently without touching the original:
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
(def (take log n)
  (match log
    ((list) (list))
    ((list e .. rest) (if (= n 0) (list) (List.push (take rest (- n 1)) e)))))
(def (main)
  (replay (take (list (Deposit 100) (Withdraw 30) (Deposit 5)) 2) 0))`}
      />
      <P>
        Replaying the first two events gives <C>70</C>. Forking a running agent to explore an alternative is
        exactly this: copy a prefix of the log and fold it into a new timeline. Nothing is snapshotted and no
        process is cloned, because the branch point is just an index into a value.
      </P>

      <H2>Recover: resume from a cursor</H2>
      <P>
        If the agent stops partway, it doesn't have to redo everything. Record how far it got, a{" "}
        <em>cursor</em> into the log plus the balance at that point, and recovery replays only the events
        after the cursor. Here the cursor is at index two with a saved balance of <C>70</C>, so replay folds
        just the remaining event and arrives back at the same answer a full replay would:
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
(def (drop log n)
  (match log
    ((list) (list))
    ((list e .. rest) (if (= n 0) log (drop rest (- n 1))))))
(def (main)
  (replay (drop (list (Deposit 100) (Withdraw 30) (Deposit 5)) 2) 70))`}
      />
      <P>
        The result is <C>75</C> again, the same balance as a full replay, reached by folding only the tail
        after the cursor. Resuming is re-folding from a checkpoint, so an agent that crashes and restarts
        picks up exactly where it left off and never performs an action twice.
      </P>

      <H2>The real thing runs this over a durable log</H2>
      <P>
        The miniatures above are the model. The production tool is <C>cdz-agent</C>, and it runs this exact
        fold over a real, durable log on disk, where the events are genuine host actions the agent performed.
        The session below is a terminal transcript, not an editable panel, but every command maps to the
        model you just ran: seeding a log, appending a trigger event, folding it with real effects, then
        replaying that same history deterministically.
      </P>
      <Note>
        <C>$ cdz-agent bootstrap agent.log</C>
        <br />
        <C>$ cdz-agent emit agent.log trigger deposit</C>
        <br />
        <C>$ cdz-agent hosted agent.log 1</C>
        <br />
        {"    "}summed per-op result = 3; 2 performed + 0 denied
        <br />
        <C>$ cdz-agent replay agent.log 1</C>
        <br />
        {"    "}summed per-op result = 3; 2 op(s) replayed, 0 missing (faithful)
      </Note>
      <P>
        The miniature above is the model; <C>cdz-agent</C> runs it over a real durable log, answering each
        host action from what the log recorded so the replay reproduces the original result with nothing
        performed twice. Replay re-runs the same timeline; fork branches a new one from any point; recovery
        resumes from a cursor. All three are the fold you already wrote, applied to a value that happens to
        be a program's whole past.
      </P>

      <Why tenet="A program's history is an ordinary value">
        Most systems bolt persistence on: a database beside the program, a snapshot mechanism, a
        checkpoint format, each its own machinery. Cadenza doesn't. The history is a list of events, a value
        like any other, and running the program is a fold over it, so the tools you already have (a sum
        type, a <C>match</C>, a recursion) are the whole persistence story. Replay is a fold, fork is a fold
        of a prefix, recovery is a fold from a cursor. Because the log is data, an agent's past is something
        you can inspect, branch, and resume with no new concepts, which is what lets one generation of an
        agent hand its whole existence to the next.
      </Why>

      <P>
        That's what makes Cadenza different: exact numbers, units that erase, effects as values, code as
        data, properties the machine checks, and now a history you can fold. The ideas are the point, but
        they earn their keep by running. Next, see them at work in full programs, in{" "}
        <Link to="/example-apps" className="text-cadenza-300 underline-offset-2 hover:underline">
          Example applications
        </Link>
        .
      </P>
    </article>
  );
}
