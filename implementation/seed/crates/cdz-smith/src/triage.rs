//! Convert libFuzzer crash/timeout artifacts into `spec/semantics/failures/` findings.
//!
//! The coverage-guided engine ([`cargo bolero`], see `fuzz-cycle.sh`) saves each failing input as a
//! file of **raw seed bytes** in its crashes dir: `crash-<sha>` for a panic, `timeout-<sha>` for an
//! input that exceeded the per-input `-timeout`, `oom-<sha>` for an allocation blow-up. Those bytes
//! are exactly what our [`generate`] consumes — so replaying an artifact reproduces the offending
//! program deterministically, independent of the compiler version.
//!
//! This adapter reads each artifact and files it through the SAME [`FindingStore`] the PRNG driver
//! uses (shrink + dedup by crash site), so a libFuzzer campaign and the fallback driver produce
//! byte-identical findings in the same buckets. Classification:
//!
//! * `crash-*` — replay through the in-process oracle ([`compile_catching`]); it panics again and we
//!   capture the site/message/backtrace. (If replaying does NOT reproduce — e.g. a nondeterministic
//!   crash, or one that needs the sanitizer build — we still file it as a crash using the artifact
//!   bytes, noting the site is unknown.)
//! * `timeout-*` / `oom-*` — we must NOT replay in-process (it would hang / OOM the triage run
//!   itself). File a `Timeout` finding directly from the artifact's regenerated program.

use std::path::Path;

use crate::finding::{Category, Filed, Finding, FindingStore};
use crate::generator::generate;
use crate::oracle::{Verdict, compile_catching};

/// Outcome tallies for a triage pass over an artifacts directory.
#[derive(Default, Debug)]
pub struct TriageStats {
    pub artifacts_seen: u64,
    pub new_buckets: u64,
    pub duplicate_hits: u64,
    /// Artifacts that no longer reproduced a crash on replay (e.g. already fixed, or nondeterministic).
    pub not_reproduced: u64,
}

/// Triage every libFuzzer artifact in `crashes_dir`, filing findings into `store`. `commit` is the
/// compiler SHA the campaign ran against. Non-artifact files (libFuzzer also drops merge/`.cur_input`
/// scratch) are ignored. Returns the tallies.
pub fn triage_artifacts(
    crashes_dir: &Path,
    store: &FindingStore,
    commit: &str,
) -> std::io::Result<TriageStats> {
    let mut stats = TriageStats::default();
    let entries = match std::fs::read_dir(crashes_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(stats),
        Err(e) => return Err(e),
    };

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        let kind = match ArtifactKind::classify(name) {
            Some(k) => k,
            None => continue, // not a libFuzzer failure artifact
        };
        if !path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&path)?;
        stats.artifacts_seen += 1;

        if let Some(filed) = triage_one(&bytes, kind, store, commit, &mut stats)? {
            match filed {
                Filed::New(_) => stats.new_buckets += 1,
                Filed::Duplicate(_) => stats.duplicate_hits += 1,
            }
        }
    }
    Ok(stats)
}

/// Which kind of libFuzzer artifact a filename denotes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArtifactKind {
    Crash,
    Timeout,
    Oom,
}

impl ArtifactKind {
    fn classify(name: &str) -> Option<ArtifactKind> {
        if name.starts_with("crash-") {
            Some(ArtifactKind::Crash)
        } else if name.starts_with("timeout-") {
            Some(ArtifactKind::Timeout)
        } else if name.starts_with("oom-") {
            Some(ArtifactKind::Oom)
        } else {
            None
        }
    }
}

/// Turn one artifact's bytes into a filed finding (or `None` if a crash artifact didn't reproduce).
fn triage_one(
    bytes: &[u8],
    kind: ArtifactKind,
    store: &FindingStore,
    commit: &str,
    stats: &mut TriageStats,
) -> std::io::Result<Option<Filed>> {
    let program = generate(bytes).source;

    match kind {
        // A hang / OOM must NOT be replayed in-process — it would wedge or OOM the triage run.
        // File it straight from the regenerated program.
        ArtifactKind::Timeout | ArtifactKind::Oom => {
            let finding = Finding {
                category: Category::Timeout,
                program,
                crash: None,
                commit: commit.to_string(),
            };
            Ok(Some(store.file(&finding)?))
        }
        // A crash is safe to replay (a panic unwinds; the in-process oracle catches it) and gives us
        // the exact site + backtrace for a good dedup key + triage note.
        ArtifactKind::Crash => match compile_catching(&program) {
            Verdict::Crash(info) => {
                let target = info.site.as_deref().map(crate::finding::normalize_site);
                let shrunk = crate::finding::shrink(&program, target.as_deref());
                let finding = Finding {
                    category: Category::Crash,
                    program: shrunk,
                    crash: Some(info),
                    commit: commit.to_string(),
                };
                Ok(Some(store.file(&finding)?))
            }
            // Didn't reproduce as a crash on replay — likely already fixed since the campaign, or a
            // sanitizer-only / nondeterministic fault. Count it but don't file a misleading bucket.
            _ => {
                stats.not_reproduced += 1;
                Ok(None)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_libfuzzer_artifact_names() {
        assert_eq!(
            ArtifactKind::classify("crash-abc123"),
            Some(ArtifactKind::Crash)
        );
        assert_eq!(
            ArtifactKind::classify("timeout-deadbeef"),
            Some(ArtifactKind::Timeout)
        );
        assert_eq!(ArtifactKind::classify("oom-99"), Some(ArtifactKind::Oom));
        assert_eq!(ArtifactKind::classify("my-corpus-entry"), None);
        assert_eq!(ArtifactKind::classify(".cur_input"), None);
    }

    #[test]
    fn a_timeout_artifact_files_a_timeout_finding_without_replay() {
        let tmp = std::env::temp_dir().join(format!("cdz-smith-triage-{}", std::process::id()));
        let crashes = tmp.join("crashes");
        std::fs::create_dir_all(&crashes).unwrap();
        // Any bytes: generate() is total, and we file WITHOUT compiling (so even a hanging program
        // is safe here).
        std::fs::write(crashes.join("timeout-0001"), [1u8, 2, 3, 4, 5, 6, 7, 8]).unwrap();

        let store = FindingStore::open(tmp.join("failures")).unwrap();
        let stats = triage_artifacts(&crashes, &store, "testsha").unwrap();
        assert_eq!(stats.artifacts_seen, 1);
        assert_eq!(
            stats.new_buckets, 1,
            "a timeout artifact should file one bucket"
        );
        assert!(store.dir().join("timeout.smith.md").exists());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn non_artifact_files_are_ignored() {
        let tmp =
            std::env::temp_dir().join(format!("cdz-smith-triage-skip-{}", std::process::id()));
        let crashes = tmp.join("crashes");
        std::fs::create_dir_all(&crashes).unwrap();
        std::fs::write(crashes.join("corpus-entry-xyz"), [0u8; 4]).unwrap();
        std::fs::write(crashes.join(".cur_input"), [0u8; 4]).unwrap();

        let store = FindingStore::open(tmp.join("failures")).unwrap();
        let stats = triage_artifacts(&crashes, &store, "testsha").unwrap();
        assert_eq!(stats.artifacts_seen, 0, "no failure artifacts present");
        assert_eq!(stats.new_buckets, 0);
        std::fs::remove_dir_all(&tmp).ok();
    }
}
