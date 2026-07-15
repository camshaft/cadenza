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

/// Render a fleet message (drained from the bridge's inbox, i.e. an agent → operator message) as a
/// Slack-mrkdwn string: who it's from, the kind, the subject, and the body/ref when present. The result is
/// length-capped (see [`SLACK_TEXT_CAP`]) so a large body can't make the Slack post fail.
pub fn render_fleet_message(msg: &crate::inbox::Message) -> String {
    let from = if msg.from.is_empty() {
        "unknown"
    } else {
        &msg.from
    };
    let kind = if msg.kind.is_empty() {
        "note"
    } else {
        &msg.kind
    };
    let mut lines = Vec::new();
    if msg.subject.is_empty() {
        lines.push(format!("*{from}* · _{kind}_"));
    } else {
        lines.push(format!("*{from}* · _{kind}_: {}", msg.subject));
    }
    if !msg.r#ref.is_empty() {
        lines.push(format!("`{}`", msg.r#ref));
    }
    if !msg.body.trim().is_empty() {
        lines.push(msg.body.trim().to_string());
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
