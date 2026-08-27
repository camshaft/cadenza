//! Fleet inbox I/O — a faithful Rust port of the atomic message delivery/drain that `cargo xtask fleet`
//! implements (see `xtask/src/fleet.rs` `deliver` / `Message`). This is the substrate the Slack bridge is
//! built on: the sidecar delivers the operator's Slack replies into an agent's inbox as `answer`s, and
//! drains its own inbox to relay agent messages back out to Slack.
//!
//! A fleet message is ONE JSON file in the recipient's inbox
//! (`<fleet_dir>/inbox/<to>/<seq:012>-<pid>-<kind>.json`), written via a temp-file + atomic rename so a
//! reader never sees a half-written file and two senders never corrupt a shared file. It MUST stay
//! byte-compatible with the Rust `xtask fleet` side so both can read each other's messages.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A per-process monotonic sequence, mirroring `xtask`'s `next_seq` atomic (starts at 1). Combined with
/// the pid in the filename, deliveries from THIS process sort in send order and never collide with the
/// Rust xtask's (a different pid). Resets across restarts, which is fine — the pid differs.
static SEQ: AtomicU64 = AtomicU64::new(1);

fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// A message as delivered into an inbox — identical field set + serde defaults to `xtask`'s `Message`, so
/// the two sides read each other's files. `r#ref`/`body` default to empty when absent (serde `default`),
/// matching the Rust side.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub subject: String,
    #[serde(default)]
    pub r#ref: String,
    #[serde(default)]
    pub body: String,
    pub seq: u64,
}

impl Message {
    /// A convenience constructor for an outgoing message; `seq` is stamped by `deliver`.
    pub fn new(
        from: impl Into<String>,
        to: impl Into<String>,
        kind: impl Into<String>,
        subject: impl Into<String>,
    ) -> Self {
        Message {
            from: from.into(),
            to: to.into(),
            kind: kind.into(),
            subject: subject.into(),
            r#ref: String::new(),
            body: String::new(),
            seq: 0,
        }
    }

    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    #[allow(dead_code)]
    pub fn with_ref(mut self, r: impl Into<String>) -> Self {
        self.r#ref = r.into();
        self
    }
}

/// True iff `agent` is a safe fleet agent name: a strict slug — a leading alphanumeric then
/// alphanumerics/hyphens (`pr-sync`, `design-jsx`, `v-slack-bridge`, `fix-foo`). NO dots, slashes, or
/// other separators, so it can never be a path component like `..` or contain a directory boundary.
///
/// SECURITY: the agent name flows from untrusted Slack input (a `@agent` retarget) into a filesystem path
/// via [`inbox_dir`]; an unvalidated `..`/`../../x` would let a Slack sender write OUTSIDE the inbox tree
/// (PR #391). This is the sink-side guard; the parser (`format`) also restricts the charset.
pub fn is_valid_agent_name(agent: &str) -> bool {
    let mut chars = agent.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false, // empty, or non-alphanumeric first char
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// The inbox directory for `agent` under `fleet_dir` (`<fleet_dir>/inbox/<agent>`). Returns `None` on an
/// invalid agent name rather than building a traversal-capable path — the single chokepoint `deliver` and
/// `drain` route through, so validating here sandboxes every filesystem access.
pub fn inbox_dir(fleet_dir: &Path, agent: &str) -> Option<PathBuf> {
    if !is_valid_agent_name(agent) {
        return None;
    }
    Some(fleet_dir.join("inbox").join(agent))
}

/// Deliver `msg` into `to`'s inbox as one atomic JSON file, byte-for-byte in the shape the Rust `xtask`
/// side reads (pretty-printed, temp-file + rename). `msg.to` and `msg.seq` are filled here from `to` and a
/// fresh sequence. Creates the inbox dir on demand (matching `xtask`, which delivers even before the
/// recipient is registered). Returns the written file path.
pub fn deliver(fleet_dir: &Path, to: &str, mut msg: Message) -> io::Result<PathBuf> {
    let dir = inbox_dir(fleet_dir, to).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid fleet agent name (path-traversal guard): {to:?}"),
        )
    })?;
    std::fs::create_dir_all(&dir)?;

    msg.to = to.to_string();
    msg.seq = next_seq();
    let kind = if msg.kind.is_empty() {
        "note"
    } else {
        &msg.kind
    };

    let json = serde_json::to_string_pretty(&msg)?;
    let fname = format!("{:012}-{}-{}.json", msg.seq, std::process::id(), kind);
    let tmp = dir.join(format!(".{fname}.tmp"));
    std::fs::write(&tmp, json)?;
    let dst = dir.join(&fname);
    std::fs::rename(&tmp, &dst)?;
    Ok(dst)
}

/// One drained inbox entry: the file path (so the caller can `mark_processed` it) and the parsed message.
#[derive(Debug, Clone)]
pub struct Drained {
    pub file: PathBuf,
    pub msg: Message,
}

/// Read every message file in `agent`'s inbox, oldest-first (filename sort = send order). Skips the
/// `processed/` subdir, dotfiles (in-flight temp writes), non-`.json` files, and any file that fails to
/// parse (left in place rather than crashing the drain). Does NOT move anything — the caller decides
/// success then calls `mark_processed` per handled file, so a crash mid-relay re-delivers rather than
/// drops. A missing inbox is an empty drain, not an error.
pub fn drain(fleet_dir: &Path, agent: &str) -> io::Result<Vec<Drained>> {
    let dir = match inbox_dir(fleet_dir, agent) {
        Some(d) => d,
        // An invalid name has no valid inbox; treat as empty rather than error (drain is a read).
        None => return Ok(Vec::new()),
    };
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut names: Vec<String> = Vec::new();
    for entry in rd {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !name.ends_with(".json") {
            continue;
        }
        // read_dir also yields the `processed` dir; a dir name never ends in `.json`, but guard on the
        // file type too so we never try to parse a directory.
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        names.push(name);
    }
    names.sort();

    let mut out = Vec::new();
    for name in names {
        let file = dir.join(&name);
        match std::fs::read_to_string(&file).and_then(|s| {
            serde_json::from_str::<Message>(&s)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        }) {
            Ok(msg) => out.push(Drained { file, msg }),
            Err(_) => { /* malformed/partial: leave in place */ }
        }
    }
    Ok(out)
}

/// Move a handled inbox file into `processed/` (created on demand), matching how agents archive drained
/// messages. A missing source is ignored (already moved).
pub fn mark_processed(file: &Path) -> io::Result<()> {
    let Some(dir) = file.parent() else {
        return Ok(());
    };
    let processed = dir.join("processed");
    std::fs::create_dir_all(&processed)?;
    let Some(name) = file.file_name() else {
        return Ok(());
    };
    match std::fs::rename(file, processed.join(name)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Move an un-postable inbox file into a `failed/` dead-letter dir (created on demand), a sibling of
/// `processed/`. Used by the outbound relay to quarantine a message that deterministically fails to post so
/// it can NEVER head-of-line-block the queue, while PRESERVING it (not dropping it) for inspection. Mirrors
/// [`mark_processed`]; a missing source is ignored (already moved).
pub fn mark_failed(file: &Path) -> io::Result<()> {
    let Some(dir) = file.parent() else {
        return Ok(());
    };
    let failed = dir.join("failed");
    std::fs::create_dir_all(&failed)?;
    let Some(name) = file.file_name() else {
        return Ok(());
    };
    match std::fs::rename(file, failed.join(name)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp dir under the system temp, unique per-test via the pid + a counter (no `rand` dep, and the
    /// toolchain's determinism rules don't apply to a normal Rust test).
    fn tmp_fleet(tag: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cadenza-slack-bridge-{}-{}-{}",
            tag,
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn deliver_writes_fleet_message_shape() {
        let dir = tmp_fleet("shape");
        let file = deliver(
            &dir,
            "concierge",
            Message::new("slack-bridge", "", "note", "hi").with_body("hello there"),
        )
        .unwrap();
        assert!(file.exists());
        let rec: Message = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(rec.from, "slack-bridge");
        assert_eq!(rec.to, "concierge");
        assert_eq!(rec.kind, "note");
        assert_eq!(rec.subject, "hi");
        assert_eq!(rec.body, "hello there");
        assert_eq!(rec.r#ref, "");
        assert!(rec.seq >= 1);
    }

    #[test]
    fn delivered_filename_matches_xtask_pattern() {
        let dir = tmp_fleet("fname");
        let file = deliver(
            &dir,
            "pr-sync",
            Message::new("slack-bridge", "", "status", "ping"),
        )
        .unwrap();
        let base = file.file_name().unwrap().to_string_lossy();
        // <seq:012>-<pid>-<kind>.json
        let parts: Vec<&str> = base.trim_end_matches(".json").splitn(3, '-').collect();
        assert_eq!(parts.len(), 3, "three dash-separated parts: {base}");
        assert_eq!(parts[0].len(), 12, "seq is 12-digit zero-padded: {base}");
        assert!(parts[0].chars().all(|c| c.is_ascii_digit()));
        assert!(
            parts[1].chars().all(|c| c.is_ascii_digit()),
            "pid is numeric: {base}"
        );
        assert_eq!(parts[2], "status");
    }

    #[test]
    fn no_dotfile_left_after_deliver() {
        let dir = tmp_fleet("dot");
        deliver(
            &dir,
            "concierge",
            Message::new("slack-bridge", "", "note", "x"),
        )
        .unwrap();
        let leftover: Vec<_> = std::fs::read_dir(inbox_dir(&dir, "concierge").unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftover.is_empty(), "no leftover .tmp dotfile");
    }

    #[test]
    fn drain_oldest_first_and_mark_processed() {
        let dir = tmp_fleet("drain");
        deliver(
            &dir,
            "slack-bridge",
            Message::new("concierge", "", "answer", "first"),
        )
        .unwrap();
        deliver(
            &dir,
            "slack-bridge",
            Message::new("pr-sync", "", "merged", "second"),
        )
        .unwrap();
        let pending = drain(&dir, "slack-bridge").unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].msg.subject, "first", "oldest first");
        assert_eq!(pending[1].msg.subject, "second");

        mark_processed(&pending[0].file).unwrap();
        let after = drain(&dir, "slack-bridge").unwrap();
        assert_eq!(after.len(), 1, "processed message no longer drained");
        assert_eq!(after[0].msg.subject, "second");
        assert!(
            inbox_dir(&dir, "slack-bridge")
                .unwrap()
                .join("processed")
                .join(pending[0].file.file_name().unwrap())
                .exists()
        );
    }

    #[test]
    fn mark_failed_dead_letters_and_removes_from_drain() {
        // A quarantined (un-postable) message moves to `failed/` — preserved, not dropped — and no longer
        // drains, so it can't block the relay queue while its content is still recoverable for inspection.
        let dir = tmp_fleet("failed");
        deliver(
            &dir,
            "slack-bridge",
            Message::new("v-x", "", "ask", "poison"),
        )
        .unwrap();
        let pending = drain(&dir, "slack-bridge").unwrap();
        assert_eq!(pending.len(), 1);
        mark_failed(&pending[0].file).unwrap();
        assert!(
            drain(&dir, "slack-bridge").unwrap().is_empty(),
            "dead-lettered message no longer drains"
        );
        let dead = inbox_dir(&dir, "slack-bridge")
            .unwrap()
            .join("failed")
            .join(pending[0].file.file_name().unwrap());
        assert!(dead.exists(), "message preserved under failed/");
        // And it's the real message, not lost.
        let rec: Message = serde_json::from_str(&std::fs::read_to_string(&dead).unwrap()).unwrap();
        assert_eq!(rec.subject, "poison");
    }

    #[test]
    fn drain_missing_inbox_is_empty() {
        let dir = tmp_fleet("missing");
        assert!(drain(&dir, "nobody").unwrap().is_empty());
    }

    #[test]
    fn drain_ignores_processed_dir_and_dotfiles() {
        let dir = tmp_fleet("ignore");
        let box_dir = inbox_dir(&dir, "slack-bridge").unwrap();
        std::fs::create_dir_all(box_dir.join("processed")).unwrap();
        std::fs::write(box_dir.join(".000000000001-1-note.json.tmp"), "{partial").unwrap();
        deliver(&dir, "slack-bridge", Message::new("x", "", "note", "real")).unwrap();
        let pending = drain(&dir, "slack-bridge").unwrap();
        assert_eq!(pending.len(), 1, "only the real .json");
        assert_eq!(pending[0].msg.subject, "real");
    }

    #[test]
    fn message_roundtrips_with_xtask_defaults() {
        // A message written WITHOUT ref/body (as `xtask send` does with empty defaults) must parse back.
        let json = r#"{"from":"unknown","to":"v-slack-bridge","kind":"ask","subject":"s","seq":1}"#;
        let m: Message = serde_json::from_str(json).unwrap();
        assert_eq!(m.r#ref, "");
        assert_eq!(m.body, "");
        assert_eq!(m.kind, "ask");
    }

    // ── SECURITY: agent-name path-traversal guard (PR #391) ─────────────────────────────────────────

    #[test]
    fn valid_agent_names_accepted() {
        for name in [
            "pr-sync",
            "concierge",
            "v-slack-bridge",
            "fix-foo",
            "design-jsx",
            "a",
            "a1",
        ] {
            assert!(is_valid_agent_name(name), "should accept {name}");
        }
    }

    #[test]
    fn traversal_and_separator_names_rejected() {
        // The exact PR #391 payloads plus assorted escapes/separators.
        for name in [
            "..",
            "../x",
            "../../etc",
            ".",
            ".hidden",
            "a.b", // dots are out (no roster name uses them; a dot enables `..`-style tricks)
            "a/b",
            "a\\b",
            "",         // empty
            "-leading", // must start alphanumeric
            "a b",      // space
            "a\0b",     // NUL
            "inbox/../x",
        ] {
            assert!(!is_valid_agent_name(name), "should REJECT {name:?}");
            assert!(
                inbox_dir(std::path::Path::new("/tmp/f"), name).is_none(),
                "no path for {name:?}"
            );
        }
    }

    #[test]
    fn deliver_to_traversal_name_errors_and_writes_nothing() {
        let dir = tmp_fleet("traversal");
        // A Slack sender's `@.. hi` would reach here as to="..". It must NOT write at the fleet root.
        let err = deliver(&dir, "..", Message::new("attacker", "", "note", "pwn")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        // Nothing created outside a valid inbox: the fleet dir has no stray files/dirs from this.
        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(
            entries.is_empty(),
            "no traversal write happened: {entries:?}"
        );
    }

    #[test]
    fn drain_of_traversal_name_is_empty() {
        let dir = tmp_fleet("traversal-drain");
        assert!(drain(&dir, "../../x").unwrap().is_empty());
    }
}
