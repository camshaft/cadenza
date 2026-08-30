//! Message shaping between Slack and the fleet inbox protocol — pure, no I/O, unit-tested. Ported from the
//! Node `format.js`.
//!
//! Two directions:
//!   Slack → fleet:  what the operator types becomes a fleet message. A leading `@agent` retargets the
//!                   recipient; a leading `!kind` sets the message kind; otherwise it's a `note` to the
//!                   default agent (the concierge). Same shape as `cargo xtask fleet send`.
//!   fleet → Slack:  a message drained from the bridge's own inbox is rendered as a readable Slack line.

/// The message kinds the fleet understands (AGENTS-fleet.md). The operator can force one with a leading
/// `!kind`; otherwise Slack→fleet defaults to `note`, which every agent accepts.
pub const KNOWN_KINDS: &[&str] = &[
    "merge-request",
    "merged",
    "reject",
    "issue",
    "assign",
    "ask",
    "answer",
    "backlog",
    "status",
    "note",
];

fn is_known_kind(k: &str) -> bool {
    KNOWN_KINDS.contains(&k)
}

/// The parsed intent of an operator's Slack line: where to send it, as what kind, with what subject/body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub to: String,
    pub kind: String,
    pub subject: String,
    pub body: String,
}

/// Parse an operator's Slack line into a fleet-send intent.
///
/// Grammar (both prefixes optional, order `@agent` then `!kind`):
///   `@concierge status the compiler`  → to=concierge, kind=note,   text="status the compiler"
///   `@pr-sync !status ping`           → to=pr-sync,   kind=status, text="ping"
///   `!backlog add dark mode`          → to=<default>, kind=backlog, text="add dark mode"
///   `just some words`                 → to=<default>, kind=note,   text="just some words"
///
/// `default_to` is the agent used when no `@agent` is given (the concierge). The subject is the first
/// line (trimmed, capped at 120 chars with an ellipsis), the body is the full remaining text.
pub fn parse_operator_message(text: &str, default_to: &str) -> Intent {
    let mut rest = text.trim();
    let mut to = default_to.to_string();
    let mut kind = "note".to_string();

    // `@agent …` — agent name is a strict slug `[A-Za-z0-9][A-Za-z0-9-]*` (NO dots/underscores/
    // separators), then optional whitespace, then the rest. SECURITY: a permissive charset let `@..` /
    // `@../x` parse as a `..`-style agent name that `inbox_dir` would join into a path-traversal write
    // (PR #391). The leading char must be alphanumeric; a non-matching `@…` is left as literal text. This
    // is the parse-side guard; `inbox::inbox_dir` re-validates at the filesystem sink (defense in depth).
    if let Some(stripped) = rest.strip_prefix('@') {
        let starts_slug = stripped
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric());
        if starts_slug {
            let end = stripped
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .unwrap_or(stripped.len());
            to = stripped[..end].to_string();
            rest = stripped[end..].trim_start();
        }
    }

    // `!kind …` — only when the word is a KNOWN kind (else leave the text literal, don't mis-kind).
    if let Some(stripped) = rest.strip_prefix('!') {
        let end = stripped
            .find(|c: char| !(c.is_ascii_alphabetic() || c == '-'))
            .unwrap_or(stripped.len());
        let word = &stripped[..end];
        if is_known_kind(word) {
            kind = word.to_string();
            rest = stripped[end..].trim_start();
        }
    }

    let rest = rest.trim();
    let first_line = rest.lines().next().unwrap_or("").trim();
    let subject = cap_subject(first_line);
    Intent {
        to,
        kind,
        subject,
        body: rest.to_string(),
    }
}

/// The subject: the first line, capped at 120 chars (117 + ellipsis) by CHARACTER count (not bytes, so we
/// never split a multibyte char). Empty → a placeholder so an agent's inbox always has a glanceable line.
fn cap_subject(first_line: &str) -> String {
    if first_line.is_empty() {
        return "(from Slack)".to_string();
    }
    let count = first_line.chars().count();
    if count > 120 {
        let head: String = first_line.chars().take(117).collect();
        format!("{head}…")
    } else {
        first_line.to_string()
    }
}

/// Slack's `chat.postMessage` rejects a `text` longer than 40000 chars, and a mrkdwn *section block*
/// caps at 3000. We post as plain `text` (not a section block), so cap well under 40000 with headroom for
/// mrkdwn markup. A fleet `ask` body can be multiple KB, and an over-limit post would FAIL — the outbound
/// mirror would then retry the same message forever, blocking the queue. Truncating keeps delivery robust;
/// the full ask still lives in the concierge inbox and tmux.
const SLACK_TEXT_CAP: usize = 3500;

/// Escape the three Slack `text` CONTROL characters (`&`, `<`, `>`) as HTML entities. Slack opens
/// link/entity parsing on these, so an unescaped one (e.g. a body with `<512KiB` or a generic `Rc<[T]>`,
/// both common in fleet notes) mis-parses or makes `chat.postMessage` return `internal_error` — this
/// deterministically failed the rich post of a real concierge note in production. Escape only message
/// CONTENT (from/kind/subject/ref/body), never the mrkdwn markers we add. `&` MUST go first, else we would
/// re-escape the `&` in the `&lt;`/`&gt;` we just produced.
fn escape_slack_entities(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Render a fleet message (drained from the bridge's inbox, i.e. an agent → operator message) as a
/// Slack-mrkdwn string: who it's from, the kind, the subject, and the body/ref when present. The result is
/// length-capped (see [`SLACK_TEXT_CAP`]) so a large body can't make the Slack post fail.
pub fn render_fleet_message(msg: &crate::inbox::Message) -> String {
    let from = escape_slack_entities(if msg.from.is_empty() {
        "unknown"
    } else {
        &msg.from
    });
    let kind = escape_slack_entities(if msg.kind.is_empty() {
        "note"
    } else {
        &msg.kind
    });
    let mut lines = Vec::new();
    if msg.subject.is_empty() {
        lines.push(format!("*{from}* · _{kind}_"));
    } else {
        lines.push(format!(
            "*{from}* · _{kind}_: {}",
            escape_slack_entities(&msg.subject)
        ));
    }
    if !msg.r#ref.is_empty() {
        lines.push(format!("`{}`", escape_slack_entities(&msg.r#ref)));
    }
    if !msg.body.trim().is_empty() {
        lines.push(escape_slack_entities(msg.body.trim()));
    }
    cap_for_slack(lines.join("\n"))
}

/// Truncate `s` to [`SLACK_TEXT_CAP`] characters (not bytes — never split a multibyte char), appending an
/// elision marker when cut. A no-op for the common short message.
fn cap_for_slack(s: String) -> String {
    if s.chars().count() <= SLACK_TEXT_CAP {
        return s;
    }
    let marker = "\n…[truncated — full text in the fleet inbox]";
    let keep = SLACK_TEXT_CAP.saturating_sub(marker.chars().count());
    let head: String = s.chars().take(keep).collect();
    format!("{head}{marker}")
}

// ── OUTBOUND-RELAY RESILIENCE (fleet → Slack) ─────────────────────────────────────────────────────
//
// A message that DETERMINISTICALLY fails to post must never head-of-line-block the outbound relay. This
// once wedged the whole queue (57 messages, ~11h): the pump posted in order and `break`'d on the first
// post error, and a single message that reliably returned Slack `internal_error` (a content/length quirk in
// Slack's mrkdwn parse — a 400-char truncation of the SAME message posted fine) blocked every message
// behind it forever, and re-blocked identically after a process restart (the poison message re-reads from
// disk). The relay now: (1) retries a transport/transient fault in place (order preserved, never
// dead-lettered); (2) counts only CONTENT-class failures per message and, past [`RELAY_DEGRADE_AFTER`],
// posts a degraded plain variant ([`render_fleet_message_plain`]); (3) past [`RELAY_QUARANTINE_AFTER`],
// dead-letters the message (moves it to `failed/`, preserving it) and keeps draining the rest.
//
// PATIENCE (thresholds are in POLLS; the outbound loop polls every ~2s): Slack `internal_error` is
// AMBIGUOUS — it is BOTH the deterministic content/mrkdwn-parse quirk this guard was built for AND a
// garden-variety transient server hiccup. The queue-never-wedges guarantee comes entirely from SKIPPING
// PAST a failing message (`continue`), NOT from quarantining it — so quarantine only needs to bound the
// truly-undeliverable case, and can afford to be very patient. It MUST be: a too-eager quarantine
// false-dead-letters a perfectly good message during a brief Slack blip (observed in production — a clean
// `note` whose degraded plain variant carried no mrkdwn triggers was still dead-lettered in ~8s because a
// transient `internal_error` window happened to span its handful of retries, so the operator never saw
// it). So we retry the RICH (full-fidelity) render for a few polls (rides out a short blip WITHOUT losing
// content), then fall back to the degraded plain variant (which both dodges a genuine content quirk AND
// keeps retrying), and only dead-letter after MINUTES of sustained failure — by which point any normal
// transient has resolved, so a give-up means the message is genuinely undeliverable.

/// Content-class post failures (≈ polls, ~2s each) after which the relay stops sending the rich render and
/// falls back to the degraded plain variant (strips mrkdwn triggers + truncates, dodging the parse quirk).
/// A handful of full-fidelity rich retries first, so a SHORT transient blip is ridden out without losing
/// content; only then do we assume a content quirk and degrade.
pub const RELAY_DEGRADE_AFTER: u32 = 5;
/// Content-class post failures (≈ polls, ~2s each) after which the relay gives up and dead-letters the
/// message to `failed/` (preserved for inspection). ~5 minutes of SUSTAINED failure — far longer than any
/// normal transient Slack `internal_error`/rate-limit/brief-outage window — so we only dead-letter a
/// genuinely-undeliverable message, never a good one caught in a blip. The queue never wedges regardless
/// (a failing message is skipped past, not blocking), so this can afford to be this patient.
pub const RELAY_QUARANTINE_AFTER: u32 = 155;
/// Relay queue depth at/above which the pump logs a backlog warning, so a wedge/backlog is VISIBLE (a
/// health signal) instead of silently piling up as it did during the ~11h wedge.
pub const RELAY_QUEUE_WARN: usize = 25;

/// The proven-safe length for the degraded post. Slack rejected a ~2073-char rendered mrkdwn message with
/// `internal_error` while a 400-char truncation of the same message posted fine — so the degraded variant
/// truncates to this. A message the operator still SEES (with a pointer to the full text in the fleet
/// inbox) beats one silently dead-lettered.
const PLAIN_TEXT_CAP: usize = 400;

/// What the relay should do with a message given how many CONTENT-class post failures it has already had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayPlan {
    /// Post the normal (rich mrkdwn) render.
    Normal,
    /// Post the degraded plain variant — the rich render keeps failing on content.
    Degraded,
    /// Give up: dead-letter the message to `failed/` and move on.
    Quarantine,
}

/// Decide the relay plan from the count of CONTENT-class failures so far (transient/transport failures are
/// retried in place and do NOT advance this count). Pure so the Rust and Node relays stay in lockstep.
pub fn relay_plan(content_failures: u32) -> RelayPlan {
    if content_failures >= RELAY_QUARANTINE_AFTER {
        RelayPlan::Quarantine
    } else if content_failures >= RELAY_DEGRADE_AFTER {
        RelayPlan::Degraded
    } else {
        RelayPlan::Normal
    }
}

/// Replace every character Slack's mrkdwn/entity parser treats as control (formatting `*_~`` `, entity/link
/// markup `<>&|`) with a space and collapse runs of whitespace. We neutralize the trigger in the CONTENT
/// (rather than a per-post `mrkdwn=false` flag, which neither transport exposes uniformly) so the degraded
/// post can't re-trigger the parse quirk that failed the rich render.
fn strip_mrkdwn(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        let c = if matches!(c, '*' | '_' | '~' | '`' | '<' | '>' | '&' | '|') || c.is_whitespace() {
            ' '
        } else {
            c
        };
        if c == ' ' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

/// Render a fleet message as a DEGRADED, plain, mrkdwn-safe, hard-truncated Slack string — the outbound
/// relay's fallback when the normal render deterministically fails to post. No mrkdwn markup, control chars
/// stripped from every field, capped at [`PLAIN_TEXT_CAP`] scalars with a marker pointing at the fleet inbox.
pub fn render_fleet_message_plain(msg: &crate::inbox::Message) -> String {
    let from = strip_mrkdwn(if msg.from.is_empty() {
        "unknown"
    } else {
        &msg.from
    });
    let kind = strip_mrkdwn(if msg.kind.is_empty() {
        "note"
    } else {
        &msg.kind
    });
    let subject = strip_mrkdwn(&msg.subject);
    let mut head = format!("[plain] {from} {kind}");
    if !subject.is_empty() {
        head.push_str(&format!(": {subject}"));
    }
    let body = strip_mrkdwn(&msg.body);
    let out = if body.is_empty() {
        head
    } else {
        format!("{head} — {body}")
    };
    if out.chars().count() <= PLAIN_TEXT_CAP {
        return out;
    }
    let marker = " …[truncated — full text in the fleet inbox]";
    let keep = PLAIN_TEXT_CAP.saturating_sub(marker.chars().count());
    let head: String = out.chars().take(keep).collect();
    format!("{head}{marker}")
}

/// A short usage/help string shown when the operator sends an empty message or `help`.
pub fn help_text(default_to: &str) -> String {
    format!(
        "*Cadenza fleet bridge* — you're talking to the fleet. Default recipient: *{default_to}*.\n\
         • plain text → a `note` to {default_to}\n\
         • `@agent …` → send to a different agent (e.g. `@pr-sync status`)\n\
         • `!kind …` → set the message kind ({})\n\
         • `@design-foo !assign build X` → both at once",
        KNOWN_KINDS.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbox::Message;

    #[test]
    fn plain_text_is_a_note_to_default() {
        let i = parse_operator_message("what's the status?", "concierge");
        assert_eq!(i.to, "concierge");
        assert_eq!(i.kind, "note");
        assert_eq!(i.subject, "what's the status?");
    }

    #[test]
    fn at_agent_retargets() {
        let i = parse_operator_message("@pr-sync are you merging?", "concierge");
        assert_eq!(i.to, "pr-sync");
        assert_eq!(i.kind, "note");
        assert_eq!(i.body, "are you merging?");
    }

    #[test]
    fn bang_kind_sets_known_kind() {
        let i = parse_operator_message("!backlog try a dark mode", "concierge");
        assert_eq!(i.kind, "backlog");
        assert_eq!(i.to, "concierge");
        assert_eq!(i.body, "try a dark mode");
    }

    #[test]
    fn at_agent_and_bang_kind_combine() {
        let i = parse_operator_message("@design-jsx !assign build the JSX surface", "concierge");
        assert_eq!(i.to, "design-jsx");
        assert_eq!(i.kind, "assign");
        assert_eq!(i.body, "build the JSX surface");
    }

    #[test]
    fn unknown_bang_kind_stays_literal_note() {
        let i = parse_operator_message("!frobnicate do a thing", "concierge");
        assert_eq!(i.kind, "note", "unknown kind falls back to note");
        assert!(
            i.body.starts_with("!frobnicate"),
            "the !word stays in the text"
        );
    }

    #[test]
    fn bang_kind_matches_the_whole_word_not_a_prefix() {
        // A `!kind` is honored only when the WHOLE alphabetic-and-hyphen word is a known kind. A word
        // that merely STARTS with a known kind (`!statusfoo`) or extends it with a hyphen (`!status-x`)
        // is NOT `status` — it must fall through to a literal `note`, else the operator's text is
        // silently mis-kinded. Pins the word-boundary contract (must stay in parity with Node).
        for text in ["!statusfoo do", "!status-x do", "!asks it"] {
            let i = parse_operator_message(text, "concierge");
            assert_eq!(i.kind, "note", "{text:?} is not a known kind → note");
            assert!(
                i.body.starts_with('!'),
                "the !word stays literal in {text:?}"
            );
        }
        // Sanity: the exact word IS honored, with or without following text.
        assert_eq!(
            parse_operator_message("!status", "concierge").kind,
            "status"
        );
        assert_eq!(
            parse_operator_message("!status ping", "concierge").kind,
            "status"
        );
    }

    #[test]
    fn subject_is_capped_body_is_whole() {
        let long = "x".repeat(200);
        let i = parse_operator_message(&format!("{long}\nsecond line"), "concierge");
        assert!(i.subject.chars().count() <= 120, "subject capped");
        assert!(i.subject.ends_with('…'), "capped subject is elided");
        assert!(i.body.contains("second line"), "body keeps full text");
    }

    #[test]
    fn cap_never_splits_multibyte() {
        // 200 multibyte chars — capping must not panic on a char boundary.
        let long = "é".repeat(200);
        let i = parse_operator_message(&long, "concierge");
        assert!(i.subject.ends_with('…'));
    }

    #[test]
    fn subject_cap_is_by_scalar_no_astral_split() {
        // Parity with the Node side: capping the subject by Unicode scalars (chars()), NOT UTF-16
        // units, so an astral char (emoji = a surrogate pair in UTF-16) is never split. `chars()`
        // yields whole scalars, so this pins the count + that the last kept char is a whole emoji.
        let i = parse_operator_message(&"👍".repeat(200), "concierge");
        let cps: Vec<char> = i.subject.chars().collect();
        assert!(cps.len() <= 120, "capped by scalar: {}", cps.len());
        assert!(i.subject.ends_with('…'), "capped subject is elided");
        assert_eq!(
            cps[cps.len() - 2],
            '👍',
            "the char before the ellipsis is a whole emoji"
        );
    }

    #[test]
    fn emptyish_yields_placeholder_subject() {
        let i = parse_operator_message("@concierge   ", "concierge");
        assert!(!i.subject.is_empty());
    }

    #[test]
    fn render_shows_all_fields() {
        let m = Message {
            from: "pr-sync".into(),
            to: "slack-bridge".into(),
            kind: "merged".into(),
            subject: "landed".into(),
            r#ref: "abc123".into(),
            body: "all green".into(),
            seq: 1,
        };
        let s = render_fleet_message(&m);
        for needle in ["pr-sync", "merged", "landed", "abc123", "all green"] {
            assert!(s.contains(needle), "missing {needle} in {s}");
        }
    }

    #[test]
    fn render_omits_absent_ref_body() {
        let m = Message::new("concierge", "slack-bridge", "answer", "go with B");
        let s = render_fleet_message(&m);
        assert!(s.contains("go with B"));
        assert!(!s.contains("``"), "no empty code span for a missing ref");
    }

    #[test]
    fn render_caps_a_huge_body_for_slack() {
        // A multi-KB ask body must not produce an over-limit post (which would fail + retry forever).
        let huge = "x".repeat(20_000);
        let m = Message::new("v-x", "slack-bridge", "ask", "big decision").with_body(huge);
        let s = render_fleet_message(&m);
        assert!(
            s.chars().count() <= SLACK_TEXT_CAP,
            "capped under Slack's limit: {}",
            s.chars().count()
        );
        assert!(s.contains("truncated"), "elision marker present");
        assert!(
            s.contains("big decision"),
            "header/subject survives the cap"
        );
    }

    #[test]
    fn render_caps_astral_chars_by_scalar_no_split() {
        // Parity with the Node side (PR #405): cap an emoji body by Unicode scalars, never producing a
        // partial char. `chars()` can't yield a lone surrogate, so this just pins the count + that the
        // kept text is still valid UTF-8 (trivially true for a String, but the count is the contract).
        let m = Message::new("v-x", "slack-bridge", "ask", "emoji").with_body("👍".repeat(4000));
        let s = render_fleet_message(&m);
        assert!(
            s.chars().count() <= SLACK_TEXT_CAP,
            "capped by scalar: {}",
            s.chars().count()
        );
        assert!(s.contains("truncated"));
        assert!(!s.contains('\u{FFFD}'), "no replacement char");
    }

    #[test]
    fn render_does_not_touch_a_normal_message() {
        let m = Message::new("pr-sync", "slack-bridge", "merged", "landed").with_body("all green");
        let s = render_fleet_message(&m);
        assert!(!s.contains("truncated"), "short message untouched");
    }

    #[test]
    fn help_names_default_and_prefixes() {
        let h = help_text("concierge");
        assert!(h.contains("concierge"));
        assert!(h.contains("@agent"));
        assert!(h.contains("!kind"));
    }

    #[test]
    fn known_kinds_is_exactly_the_fleet_protocol_set() {
        // The `!kind` prefix is honored only for a fleet message kind (AGENTS-fleet.md's protocol
        // table). This pins the EXACT set so (a) the Rust and Node `KNOWN_KINDS` can't silently drift
        // apart — an operator's `!newkind` would then be honored on one parser and mis-kinded to `note`
        // on the other — and (b) adding/removing a fleet kind forces a deliberate update here. The Node
        // side pins the identical set in smoke.test.js; keep the two edits together.
        let mut got = KNOWN_KINDS.to_vec();
        got.sort_unstable();
        let mut want = [
            "answer",
            "ask",
            "assign",
            "backlog",
            "issue",
            "merge-request",
            "merged",
            "note",
            "reject",
            "status",
        ];
        want.sort_unstable();
        assert_eq!(got, want, "KNOWN_KINDS drifted from the fleet protocol set");
    }

    #[test]
    fn render_escapes_slack_control_entities_in_content() {
        // `&`, `<`, `>` are Slack `text` control chars; unescaped they mis-parse / `internal_error` (a real
        // concierge note with `<512KiB`/`>512KiB` failed the rich post in production). Pin that the CONTENT
        // is HTML-escaped so a full-fidelity post survives — while the mrkdwn markers we add stay literal.
        let m = Message::new("v-x", "slack-bridge", "note", "shrink <512KiB & >0")
            .with_body("split file <512KiB, keep Rc<[T]> & Vec<T>");
        let s = render_fleet_message(&m);
        assert!(s.contains("&lt;512KiB"), "< escaped: {s}");
        assert!(
            s.contains("&gt;0") && s.contains("Vec&lt;T&gt;"),
            "> and generics escaped: {s}"
        );
        assert!(s.contains("&amp;"), "& escaped: {s}");
        // No RAW control chars survive in the rendered text (would be Slack's, not ours) …
        assert!(
            !s.contains('<') && !s.contains('>'),
            "no raw angle brackets: {s}"
        );
        // … and `&` only ever appears as the head of an entity (never a bare `&`).
        assert!(
            s.match_indices('&')
                .all(|(i, _)| s[i..].starts_with("&amp;")
                    || s[i..].starts_with("&lt;")
                    || s[i..].starts_with("&gt;")),
            "every & is an entity head: {s}"
        );
        // The mrkdwn structure we add is untouched.
        assert!(s.contains("*v-x*") && s.contains("_note_"));
    }

    #[test]
    fn render_does_not_double_escape_ampersand() {
        // `&` must be escaped FIRST so we never turn a produced `&lt;` into `&amp;lt;`.
        let m = Message::new("v-x", "slack-bridge", "note", "a").with_body("A && B < C");
        let s = render_fleet_message(&m);
        assert!(s.contains("A &amp;&amp; B &lt; C"), "single-escaped: {s}");
        assert!(!s.contains("&amp;lt;"), "no double-escape: {s}");
    }

    // ── OUTBOUND-RELAY RESILIENCE: plan escalation + degraded plain render ──────────────────────────

    #[test]
    fn relay_plan_escalates_normal_then_degraded_then_quarantine() {
        // 0..DEGRADE → normal; DEGRADE..QUARANTINE → degraded; >=QUARANTINE → quarantine. Pins the exact
        // escalation so a poison message is bounded to a finite number of polls before it's dead-lettered
        // (it can never block the queue forever, the ~11h wedge this fix exists for).
        assert_eq!(relay_plan(0), RelayPlan::Normal);
        assert_eq!(relay_plan(RELAY_DEGRADE_AFTER - 1), RelayPlan::Normal);
        assert_eq!(relay_plan(RELAY_DEGRADE_AFTER), RelayPlan::Degraded);
        assert_eq!(relay_plan(RELAY_QUARANTINE_AFTER - 1), RelayPlan::Degraded);
        assert_eq!(relay_plan(RELAY_QUARANTINE_AFTER), RelayPlan::Quarantine);
        assert_eq!(
            relay_plan(RELAY_QUARANTINE_AFTER + 100),
            RelayPlan::Quarantine
        );
    }

    #[test]
    fn plain_render_strips_mrkdwn_and_is_bounded() {
        // The degraded variant must carry NO mrkdwn/entity control chars (the rich render's `*..*`/`_.._`
        // markup + any in the body) so it can't re-trigger Slack's parse quirk, and must be length-bounded.
        let m = Message::new("pr-sync", "slack-bridge", "ask", "decide *A* or `B`")
            .with_body("use <https://x> & _emphasis_ ~strike~ | pipe");
        let s = render_fleet_message_plain(&m);
        for bad in ['*', '_', '~', '`', '<', '>', '&', '|'] {
            assert!(
                !s.contains(bad),
                "plain render must not contain {bad:?}: {s}"
            );
        }
        assert!(
            s.contains("pr-sync") && s.contains("ask"),
            "keeps who/what: {s}"
        );
        assert!(s.chars().count() <= PLAIN_TEXT_CAP);
    }

    #[test]
    fn plain_render_truncates_a_huge_body_with_marker() {
        // The proven-safe degraded post: a message that failed the rich post must degrade to a short,
        // operator-visible line (with a pointer to the full text) rather than being dead-lettered outright.
        let m = Message::new("v-x", "slack-bridge", "ask", "big").with_body("x".repeat(9000));
        let s = render_fleet_message_plain(&m);
        assert!(
            s.chars().count() <= PLAIN_TEXT_CAP,
            "capped: {}",
            s.chars().count()
        );
        assert!(s.contains("truncated"), "elision marker present");
        assert!(s.contains("v-x"), "header survives the cap");
    }

    #[test]
    fn plain_render_caps_astral_by_scalar_no_split() {
        // Cap by Unicode scalar so an emoji body never splits into a lone surrogate / replacement char.
        let m = Message::new("v-x", "slack-bridge", "ask", "emoji").with_body("👍".repeat(4000));
        let s = render_fleet_message_plain(&m);
        assert!(s.chars().count() <= PLAIN_TEXT_CAP);
        assert!(!s.contains('\u{FFFD}'), "no replacement char");
    }

    // ── SECURITY: a `@..`-style retarget must not parse as a traversal agent name (PR #391) ─────────

    #[test]
    fn traversal_retarget_does_not_become_the_recipient() {
        // `@.. hi` must NOT set to="..". The `@` is not a valid slug start, so it's left as literal text
        // and the message falls through to the default recipient.
        let i = parse_operator_message("@.. hi", "concierge");
        assert_eq!(
            i.to, "concierge",
            "traversal name never becomes the recipient"
        );
        assert!(crate::inbox::is_valid_agent_name(&i.to));

        let i2 = parse_operator_message("@../../etc pwn", "concierge");
        assert_eq!(i2.to, "concierge");
    }

    #[test]
    fn dotted_name_stops_at_the_dot() {
        // `@a.b` — the slug is just `a` (dots are no longer part of an agent name), and `.b` stays text.
        let i = parse_operator_message("@a.b rest", "concierge");
        assert_eq!(i.to, "a");
        assert!(crate::inbox::is_valid_agent_name(&i.to));
        assert!(i.body.starts_with(".b"));
    }

    #[test]
    fn every_parsed_recipient_is_a_valid_agent_name() {
        // Fuzz-ish: whatever the parser yields as `to` must always be sink-safe.
        for msg in [
            "@pr-sync go",
            "@.. x",
            "@a/b y",
            "plain",
            "@-bad z",
            "@design-jsx !assign do",
        ] {
            let i = parse_operator_message(msg, "concierge");
            assert!(
                crate::inbox::is_valid_agent_name(&i.to),
                "parser produced an unsafe recipient {:?} from {msg:?}",
                i.to
            );
        }
    }
}
