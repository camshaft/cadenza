(chapter
  (slug "platform-safety")
  (title "Doing things safely")
  (pillar "platform")
  (section "Doing things safely")
  (blurb "Capabilities and authorization: every reach outward is an effect through one gate, the effect row is the permission list, and dangerous power is attenuated behind safe components.")
  (lede
    "An agent that can run shell commands, call the network, and message other agents is an agent that "
    "can do real damage. The platform's answer to \"what is this thing allowed to do?\" reuses a single "
    "idea you've already seen twice: an " (em "effect") ". Every risky thing an agent does is an effect, "
    "every effect that acts on the world passes one gate, and the language makes what a program can do "
    "legible before it runs.")
  (h2 "Every reach outward is an effect, through one gate")
  (p "A reducer only ever appends to its own log. Anything else, reading another agent's status, sending "
    "it a message, running a command, fetching a URL, is an " (em "effect") ": it's requested, authorized, "
    "performed, and its result folded back as an event. Crucially there's no second mechanism. Reaching "
    "into another agent isn't special \"cross-agent\" machinery; it's the same authorized-effect path as a "
    "shell call, so there's exactly " (em "one") " place to secure, audit, and rate-limit.")
  (p "That single choke point is what makes the system auditable. Every effect that crosses a boundary "
    "records what caused it, so \"why did this agent touch production?\" is answered by following the "
    "recorded chain backward, across agents, with nothing to reconstruct after the fact.")
  (h2 "The effect row is the permission list")
  (p "Here's where the language does the platform a favor. Because Cadenza has "
    (link (slug "effects") "algebraic effects") ", a program's type already spells out the effects it "
    "performs, its " (em "effect row") ". On the platform, that row " (em "is") " the set of capabilities "
    "the program requires. You don't maintain a permission list beside the code and hope it stays in sync: "
    "the permission list is derivable from the type, before the program runs.")
  (note "what a program NEEDS (its effect row, from the type)" (br)
    "what an agent MAY DO (its granted capabilities)" (br)
    "what the authorizer CHECKS (may this principal discharge this effect on this resource?)" (br)
    "→ all three are the same object, an effect row, seen from three sides")
  (p "When a program needs more than an agent currently holds, the gap is itself something to request, and "
    "requesting a capability is just another effect, flowing down the same authorized path. Permissions "
    "stop being separate access-control lists and become a property of the code the type system already "
    "tracks.")
  (h2 "Wrapping danger in a safe shape")
  (p "A broad capability like \"run any shell command\" has a huge blast radius, so handing it out widely is "
    "exactly what you don't want. The platform's move is to " (em "attenuate") ": publish a component that "
    "internally holds the dangerous capability but exports only a narrow, safe resource. A component that "
    "runs " (c "date") " and returns the string holds the full shell capability inside, but callers need "
    "only a tiny \"may ask for the date\" grant, and the code is provably safe for anyone to invoke.")
  (p "It's the platform echo of the language's " (link (slug "opaque-types") "opaque types")
    ": hide the powerful representation behind a boundary and expose only operations that can't break the "
    "invariant. The dangerous grant lives with one audited, published program instead of being scattered "
    "across every caller. Powerful primitives get wrapped into safe, named, shareable resources, the way "
    "a language contains an " (c "unsafe") " operation behind a checked interface.")
  (h2 "One gate, checked before every world-effect")
  (p "That gate is a real, concrete check, not a vague promise. Before the kernel dispatches any effect "
    "that " (em "acts on the world") ", running a command, fetching a URL, calling a model, touching the "
    "filesystem or store, it resolves the effect's " (em "target") " (the exact resource being touched) and "
    "asks the authorizer a single yes-or-no question: may this agent, with the capabilities it holds, "
    "discharge this effect on that target? The answer decides whether the effect runs at all, and "
    (em "deny wins") ", so no grant can override an explicit denial. (A small class of read-only control "
    "queries, an agent asking what capabilities it holds, say, are exempt: they don't act on anything, so "
    "there's nothing to authorize.)")
  (p "A capability isn't a coarse on-off flag; it names " (em "which") " resources it covers by a predicate. "
    "A grant can match one exact resource, any of a fixed set, a host, a path prefix, or, for supervision, "
    "an agent and all its descendants. So \"may run " (c "date") "\" and \"may run any command\" are different "
    "capabilities with different reach, and the authorizer compares the effect's resolved target against "
    "those predicates rather than trusting a name.")
  (note "every world-effect → resolve its target → authorizer checks (capability predicate matches? no deny-rule?) → run or refuse" (br)
    "capability predicates scope reach: exact resource · one-of a set · a host · a path prefix · an agent + its descendants" (br)
    "read-only control queries (e.g. \"what can I do?\") are exempt, since they act on nothing")
  (h2 "The rule-maker is itself swappable")
  (p "The kernel doesn't hardcode that policy. It calls the authorizer through a narrow interface, never a "
    "concrete engine, so the decision-maker can be replaced without touching the kernel core. Today that "
    "interface holds a built-in authorizer, the capability-and-deny-rule check just described. The design "
    "direction, still ahead, is to drop a standard policy engine (Cedar) behind that very same interface as its own "
    "content-addressed component, referenced by hash from the log, so richer policies become possible "
    "without the kernel learning anything new.")
  (p "Either way the result is the same: the security policy is data, not fixed code, so swapping the "
    "authorizer or changing a policy is an authorized, logged event rather than a redeploy, auditable and "
    "versioned like any other content, and the kernel stays true to its one discipline of knowing nothing. "
    "Next, how the kernel runs many agents at once without losing any of this."))
