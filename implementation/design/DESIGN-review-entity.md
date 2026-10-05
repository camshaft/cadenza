<!-- Approved board design of record: task-board Document #5 "Review entity" (v13), approved by
     cameron 2026-09-30. This tracked copy mirrors that document; the board document remains the
     living design of record and its revision history is in Appendix A8. Tracking epic: board #294;
     build increments: board #372-#377. -->

# Review entity

## Background

The fleet runs an automatic self-improvement loop today (epic #176, lane project #28): ephemeral observer sessions read an agent's transcript and file evidence-backed improvement proposals that board-pm triages into changes to tooling, knowledge, and process. Those observations are currently tracked as untyped board tasks. The board already tracks work as tasks and documents, documents already have a review flow (submit for review, request changes, approve), and a GitHub-to-board bridge (github-bridge, #136) already syncs GitHub items into the board with exactly-once ingest and external-author attribution.

Reviews are a large part of how the fleet produces correct work: a document review, a code review, a design review, and the self-improvement observation of an agent session are all cases of one activity — someone examines something and records what should change. The fleet wants a person pulled in only after a draft has already been critiqued, and it wants the volume of issues its reviews surface to fall over time. Both aims are served by making a review a first-class thing that agents can perform, measure, and learn from with the machinery that already exists.

## Problem Statement

The fleet has no single first-class entity for a review that works across every source and kind of review; drafts are not critiqued automatically before a person is involved; and a completed review does not automatically produce improvements that reduce the issues future reviews surface.

## Requirements, Goals, and Non-Goals

Goals:

1. A review is represented uniformly across its sources — an internal board document, a GitHub pull request, and an internal code-review tool change request — with no source-specific fields in the public interface.
2. A review is the single dedicated entity for every kind of review the fleet performs, including a self-improvement observation of an agent session or a completed task, so those observations are first-class records rather than untyped tasks.
3. The number of issues a review surfaces falls over time, read as a trend from each review's log, sliced by review kind and by the producing area or agent.
4. A fall in issues surfaced is genuine and not the result of less scrutiny: it is not accompanied by a rise in escaped defects, where an escaped defect is an issue found after approval.
5. A draft is critiqued for correctness, clarity, risk, and unconsidered alternatives before a person is asked to review it, so a person reviews only work that has already been critiqued and addressed.
6. Every improvement proposal a review produces lands in one curated, deduplicated, bounded queue, not in a separate system.
7. Adding review-driven and task-driven improvement creates no second improvement system for the fleet to run and triage.
8. A high-frequency event does not flood the improvement queue: an event of little value, such as a trivial task completion, produces no proposal.
9. Delivering the GitHub pull-request review needs no new GitHub data collection beyond what the fleet already gathers, so it can ship in a first increment.

Non-Goals:

10. This does not replace the existing board Document review flow; a document review builds on it.
11. This does not auto-sync internal code-review tool change requests in the first version; a change-request review is set through the interface until a bridge exists.
12. This does not require full GitHub pull-request state fidelity, such as draft and changes-requested, in the first version.
13. Adversarial review does not replace a person's review; it precedes it, and whether it blocks a person's review is an open decision.

## Solutions

### Solution A: One review entity for every review kind

A lightweight Review record carries a kind (document, code, design, agent-session, task), a source that is metadata only, one target reference read per source, a lifecycle, and a single generic event log. Every kind of examination the fleet does is a Review: a person or an adversarial reviewer reviews a document, a code change, or a design; an observer reviews an agent session or a completed task for self-improvement. Everything that happens to a review is an entry in its log, including a finding; an actionable finding is linked to a child task, which for a self-improvement observation is exactly the project #28 proposal the fleet files today. When a draft is submitted, one or more ephemeral adversarial reviewer agents are spawned to critique it from distinct angles and record findings before a person is involved. When an artifact review concludes, an observation review of the producing session reads the review's timeline and learns what would reduce such findings next time. The self-improvement observations that are untyped #28 tasks today become agent-session and task reviews, so the whole self-improvement loop is expressed in one entity. Both the adversarial reviewers and the observation reviewers are spawned by the existing observer spawn path, and every proposal still lands in the one project #28 queue. A GitHub pull request becomes a code review through github-bridge using its existing exactly-once ingest, and its concluded state is read from data the bridge already collects.

Pros:

- Meets Goals 1 and 2: one entity and one interface span documents, pull requests, change requests, agent sessions, and tasks, so self-improvement observations become first-class reviews.
- Meets Goal 5: a submitted draft is critiqued before a person sees it.
- Meets Goals 6 and 7: findings and proposals stay in the single project #28 queue, spawned by the one existing engine, with no parallel machinery.
- Meets Goals 3 and 4: the generic log records everything each review surfaced, so the trend and the escaped-defect counterbalance are read from the logs across reviews.
- Meets Goal 9: the pull-request concluded state comes from data github-bridge already collects.

Cons:

- Adds a new entity, a lifecycle, and an adversarial reviewer role, which is a one-time build cost the goals must repay.
- Unifying today's untyped self-improvement tasks into the entity touches the live project #28 convention and needs care so nothing in flight is lost (Goal 6).
- Running adversarial reviewers on every submitted draft has a cost that must be bounded.

### Solution B: Extend the document review flow

Reuse the Document entity's existing submit, request-changes, and approve states, add a generic log, adversarial-review spawning, and an observation trigger, and introduce no separate review entity.

Pros:

- Least new data model, reusing an existing lifecycle, which partly serves Goal 1 for documents.

Cons:

- Fails Goal 1: a Document cannot represent a GitHub pull request or a change request, so uniform source-agnostic reviews are impossible.
- Fails Goal 2: a Document cannot represent an agent-session or task observation, so self-improvement observations cannot be unified into it.

### Solution C: A separate self-improvement engine

Build the review entity and adversarial reviewers, but keep the self-improvement observations and their proposals in their own engine and queue rather than the existing project #28 lane.

Pros:

- Slightly fewer changes to the current project #28 convention in the short term.

Cons:

- Fails Goals 6 and 7: it creates a second engine and queue parallel to project #28, splitting triage and duplicating machinery.
- Fails Goal 2: self-improvement observations stay separate rather than becoming first-class reviews.

## Recommendation

Adopt Solution A. It is the only option that meets Goal 1, because a pull request and a change request cannot be modeled by the Document entity, which rules out Solution B, and the only option that meets Goals 6 and 7, because a separate engine and queue rules out Solution C. It meets Goal 2 by making a self-improvement observation a review of an agent session or a task, which is what the operator asked for and what board-pm co-owns for project #28. It meets Goal 5 by critiquing a draft before a person reviews it. Every event a review produces, findings included, is recorded in one generic log that Goal 3 reads and that the observation reviews take into account, so one record covers both how well the fleet critiques a draft and how well it learns to need fewer critiques. It meets Goal 9 by reading the pull-request concluded state from data github-bridge already collects. The data model, lifecycle, review and observation mechanics, adversarial review, GitHub phasing, guardrails, build increments, open decisions, and revision history are in the Appendix.

## Appendix

### A1. Data model

v-task-board owns the board data model, the MCP tools, the interface, and the events; this section lists the capabilities the review entity needs, not the routes.

- id.
- kind: one of document, code, design, agent-session, task.
- source: one of board-document, github-pull-request, internal-change-request, url, agent-session, task. Metadata only.
- target-ref: the pointer read per source — a board document id, an external identifier such as a pull-request url, an agent-session id, or a task id.
- status: the lifecycle state (A2).
- log: a single append-only, generic event log attached to the review. Every event is an entry with a timestamp and a type — submitted, revised to a new version, a finding raised, a finding resolved, a comment, a state change, an adversarial-review run, and the concluding decision. A finding is an entry of type finding, not a separate collection; an actionable finding entry is linked to a child task. Reading the log in order gives the full timeline of the review, which the observation reviewer takes into account when producing proposals, and from which any count or trend is derived rather than stored as a separate counter (operator preference, comments 48 and 49).
- vetted: whether adversarial review has run and its findings are addressed, for the person-review gate (A3b, D17).
- title, created-by, assignee (the reviewers), and timestamps.
- metadata: the producing agent id when a fleet agent authored the artifact, a predecessor review id for escaped-defect lineage, and tags.
- links: the existing attached-documents and attached-tasks relations, plus the child tasks linked from actionable finding entries.

A finding entry reuses the existing comment machinery, tagged as a finding; for a self-improvement observation (kind agent-session or task) the actionable finding entries are the improvement proposals the fleet files in project #28 today, each linked to its child task. For a GitHub pull request the conversation comments are already ingested by github-bridge and appear as log entries; the inline diff-review comments arrive in increment 2b (A6). github-bridge ingests GitHub items exactly-once via the board's external-link pattern (#270) and attributes external authors via external identities (#149); GitHub issue and pull-request numbers share one per-repository sequence, so one external link identifies a number as either an issue-task or a pull-request review with no collision, and the only adapter change is a routing branch. Events: review-created, review-status-changed, review-log-appended, review-terminal on entering approved or closed, and review-opened-for-review on entering in-review.

### A2. Lifecycle and state mapping

States: open, created and linked, not yet under active review; in-review, a reviewer is active, and for a document this is the submit-for-review state that also spawns adversarial reviewers; changes-requested, issues raised and the author must revise, cycling back to in-review on resubmission; approved, the positive concluding state that enqueues the observation review; closed, the non-approval concluding state that also enqueues the observation review. Each transition is an entry in the log.

For a board document the underlying document flow maps submit-for-review to in-review, request-changes to changes-requested, and approve to approved.

For a GitHub pull request via github-bridge, endorsed by v-github-bridge: an open non-draft pull request maps to in-review, free from the existing poll; an open draft maps to open and needs the Pulls interface (2b); a changes-requested review decision maps to changes-requested and needs the Reviews interface (2b); a closed pull request with a merge timestamp maps to approved and fires the observation, free from the existing poll (2a); a closed pull request without a merge timestamp maps to closed and fires the observation, free from the existing poll (2a). The concluding state is uniform: a merge maps to approved, and the source field disambiguates a merge from a document approval, so there is no separate merged status. An approving GitHub review is an intermediate signal, not the concluding state.

### A3. The review and observation model

Every review, whatever its kind, records its events in its log, and an actionable finding entry links to a child task; for an agent-session or task review those child tasks are the project #28 improvement proposals. An observation review is created on four occasions, each spawned by the existing observer spawn path (#187, #188) under the durable observer identity: a draft is submitted (adversarial review of the artifact, A3b); an artifact review concludes (an agent-session review of the producing session, to learn what reduces findings); a task is marked done above the substance bar (a task review); and a transcript reaches a size threshold or an agent spins down (an agent-session review, the live #176 case). An observation reviewer reads its subject — the reviewed review's timeline from its log, the agent session, or the task history and the assignee's session — grounds itself in the knowledge base and memories, and records findings whose actionable entries become improvement proposals against tooling, knowledge, workflows, processes, and checks, using the existing four project #28 categories. When the reviewed artifact was authored by a person there is no fleet session to read, so the review grounds on the log and the process instead. An observation review does not itself trigger an observation review, which prevents recursion.

### A3b. Adversarial review on submit

When a draft is submitted, the spawn path starts one or more adversarial reviewer agents, each assigned a distinct angle: correctness and completeness; clarity and writing quality; risk and security; and alternatives not considered. Each reads the draft, the knowledge base and memories, and the Fleet Doc-Writing Style Guide, and records its critique as finding entries in the log. The clarity and writing-quality angle is where the fleet's approved writing-guidance mechanism is enforced (the #367 mechanism, operator seq-919): the reviewer applies the humanize-writing three-pass — remove AI vocabulary, break AI sentence and section structures, and add human texture — and checks the tone, structure, and content red-flags, reading the Fleet Doc-Writing Style Guide as the source of truth. This makes the adversarial reviewer the point at which writing-guidance adherence is judged before a person is involved. The author addresses each finding, publishing new versions, and the review becomes vetted once adversarial review has run and its findings are addressed. By default a person is routed the review only once it is vetted, with the log attached, though a person can always open a review early; the gate governs routing, not access (D17). Every critique and its resolution is an entry in the log. Cost controls: a bounded reviewer count per document, dedup of findings across angles, no re-review of an unchanged version, and a per-document cap on resubmission-triggered reviews (D18).

### A4. Guardrails

Inherited from project #28 for every observation review: evidence is required on every proposal; a change to a charter, tool, or check is proposed, never applied live; dedup is search-before-file, where a repeat is a corroborating comment that raises priority rather than a new ticket; a confidence and severity floor sends a sub-floor finding to a knowledge note rather than a ticket; there is a per-sweep cap; a declined finding is sticky; scope is meta only, so a product-code finding routes to the owning vertical; policy changes route through the concierge; and board-pm pulls a curated queue. A review's findings, whatever the review kind, classify into the same four project #28 categories — charter-prompt, tooling-gap, knowledge-gap, process-contract — so triage and routing are identical regardless of kind (board-pm co-design). One observation review runs per concluding event and per qualifying task completion. Task completion is the highest-frequency trigger, so a conservative substance bar applies: a task qualifies only when it is non-trivial, such as multi-session work, a landed pull request or decision, findings, a blocked span, or a substantive discussion, and routine or no-op closes produce no review or are batched. A review is observed once per anchor, and a re-open is a new episode only on genuinely new activity. A review that concludes positively with no findings produces no proposal but does record a positive knowledge note. Adversarial review carries its own cost controls in A3b and D18.

### A5. Measuring improvement from the log

Each review keeps one generic log — the append-only record of everything that happened to it (A1) — and reading it in order reconstructs the review's timeline. Rather than storing a scalar issue count or iteration count, any measure is derived from the logs. The headline reading is the number of finding entries a review surfaced, taken over time as a trend sliced by kind and by producing area or agent. Because an actionable finding also links to a child task, the review, its log, and its child tasks together are the record, and the reason a trend moved is visible in the timeline and the child-task history. There is no separate reporting store; the reading runs over the board's own graph, with a trend shown in the interface. It is counterbalanced by escaped defects — issues found after approval, in a later review of the same lineage, a follow-up defect, or a re-open — so a fall in findings beside a rise in escaped defects is flagged rather than counted as improvement. Each proposal states the change and the observable result that would confirm it, and a later observation review reads the timeline and checks the trend for the area.

### A6. Build increments

1. Review entity: data model, MCP tools, and events (owner v-task-board, repository task-board). The fields in A1 including the generic log, the vetted flag, and the agent-session and task kinds, the lifecycle in A2, the tools to create a review, set its status, append a log entry including a finding, get it, and list reviews, the events including review-terminal and review-opened-for-review, external-link ingest, and log recording. Gate: task-board CI.
2. GitHub pull-request review, phased (owners v-github-bridge and v-task-board). Increment 2a stops skipping pull requests, reads the merge timestamp from the existing poll, and routes a pull request to a code review with concluding states only and conversation-comment log entries, with no new GitHub endpoints, delivering the concluding trigger. Increment 2b adds intermediate states via the Pulls and Reviews interfaces and inline diff-review-comment findings. Gate: github-bridge crate tests and task-board CI.
3. Adversarial review on submit (owner v-fleet-tooling for the reviewer role and trigger, v-task-board for the vetted state and gate). A trigger on review-opened-for-review that spawns adversarial reviewers on the existing spawn path, the reviewer role of A3b including the writing-guidance angle (the #367 humanize-writing three-pass and the tone, structure, and content red-flags, reading the Fleet Doc-Writing Style Guide as truth), finding recording, the vetted transition, and the person-review gate. Reuses the spawn engine. Gate: a seeded dry run on a submitted draft. Coordinates with board-pm and the librarian on the #367 pattern list.
4. Observation reviews and the self-improvement unification (owners v-fleet-tooling and board-pm, co-designed). Model the agent-session and task observations as reviews whose actionable finding entries are project #28 proposals, classified into the four #28 categories verbatim (A4). The migration is forward-only (board-pm co-design): in-flight project #28 observation tasks complete as tasks under today's model, and the review entity begins capturing new observations only after it ships, so no live triage state, link, or ownership is converted or lost. That is this increment's preserve-in-flight gate. Gate: a seeded dry run per occasion and a check that in-flight #28 items are untouched.
5. Improvement trend from the log (owners v-task-board and v-board-ui). The reading in A5 derived from the review logs with the escaped-defect counterbalance, over the board's own graph, with no separate store. Gate: a query test on a seeded dataset.
6. Review interface (owner v-board-ui). The review view showing the lifecycle, the log timeline, the vetted state, the source link per source, the child tasks, and the trend. Gate: board-ui build.

Suggested order: increment 1, then 2a, 3, and 4, then 5 and 6 in parallel once 1 lands, with 2b and the richer interface after. On approval, routing goes to board-pm, which owns the project #28 triage and the proposal convention. Increment 2 coordinates with v-github-bridge, which has volunteered to own it, taking 2a first. Increment 4 is co-designed with board-pm because it touches the live project #28 convention.

### A7. Open decisions, with a recommended default each

1. A new review entity, not an extension of the document flow; the default is the new entity, because a document cannot model a pull request, a change request, or a session.
2. Source is metadata with one target reference per source; the interface is identical across sources.
3. The lifecycle is open, in-review, changes-requested cycling with in-review, and the concluding states approved and closed.
4. Both concluding states, approved and closed, enqueue an observation review.
5. A finding is an entry of type finding in the review's one generic log, not a separate collection or sub-entity; an actionable finding entry links to a child task; conversation comments in 2a and inline diff-review comments in 2b appear as log entries too.
6. Reuse the project #28 lane and board-pm triage; every proposal is a linked child task of a review; no parallel engine.
7. Reuse the four project #28 categories and mark a finding's source so the queue stays one filterable queue.
8. The improvement measure is derived from each review's log — a trend in finding entries, sliced by kind and area, counterbalanced by escaped defects — rather than a stored count.
9. A positive conclusion with no findings records a positive knowledge note and no proposal.
10. An observation review reads a fleet author's session where one exists, and grounds on the log and process for a person-authored artifact.
11. The interface capability list is flagged to v-task-board; this design does not specify routes.
12. GitHub pull-request sync is in scope, phased into 2a with no new endpoints and 2b with the Pulls and Reviews interfaces; change-request sync is out of scope until a bridge exists.
13. The self-improvement observations become agent-session and task reviews, unifying the current untyped project #28 tasks into the entity; the default is to unify with a forward-only cutover (in-flight tasks finish as tasks, the entity captures new observations), co-designed with board-pm; the alternative is to keep them as untyped tasks (Solution C), which is not recommended.
14. A task completion enqueues an observation review only above a conservative substance bar; routine and no-op closes produce nothing; this needs the operator's decision on the exact bar.
15. The pull-request-to-review mapping lives in the github-bridge adapter and the entity and status transitions live in v-task-board; the concluding state is uniform, disambiguated by source; endorsed by v-github-bridge.
16. A small fixed set of adversarial-review angles with one reviewer per angle, configurable per project or kind; the clarity and writing-quality angle applies the approved writing-guidance mechanism (the #367 humanize-writing three-pass and the tone, structure, and content red-flags, reading the Fleet Doc-Writing Style Guide as truth), so the adversarial reviewer is the writing-guidance adherence lever; this needs the operator's preference on how many angles and which.
17. A person is routed a review only once it is vetted, with the log attached, but can open it early; this needs the operator's decision on whether to make it a hard block.
18. Adversarial-review cost controls: a bounded reviewer count, dedup across angles, no re-review of an unchanged version, and a per-document cap; this needs the operator's preference on the count and cap.
19. The review records one generic append-only log that contains all events including findings, from which the timeline is reconstructed and taken into account for proposals and any count or trend is derived; the default is the single generic log (operator preference, comments 48 and 49); it replaces separate finding collections and stored counters.

### A8. Revision history

Versions 1 to 4 established the entity, the task-driven improvement pipeline shared by the transcript, review-close, and task-done triggers, and the GitHub pull-request source via github-bridge, folding review from board-pm, v-github-bridge, and the operator. Version 6 restored the authoritative content after a concurrent edit. Version 7 reworked the document to the Fleet Doc-Writing Style Guide. Version 8 added adversarial review on submit. Version 9 completed the style-guide conformance pass and folded the operator's request to make self-improvement observations first-class reviews. Version 10 folded board-pm's co-design answers and sharpened three goals into outcomes. Version 11 replaced the stored issue and iteration counters with a log from which any measure is derived. Version 12 unified findings into a single generic append-only log per review, so the full timeline drives the proposals. Version 13 folded board-pm's flag that the adversarial reviewer's clarity angle applies the approved writing-guidance mechanism. Approved at version 13 by the operator on 2026-09-30.
