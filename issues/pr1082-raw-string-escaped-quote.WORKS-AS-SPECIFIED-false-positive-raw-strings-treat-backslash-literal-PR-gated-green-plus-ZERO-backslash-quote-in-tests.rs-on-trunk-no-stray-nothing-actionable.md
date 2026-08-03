# PR #1082 review comment — rcdzc/src/tests.rs (v-inference)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1082
(PR: "cand: v-inference — rcdzc tests.rs (rcdzc-tests lane, freed by #1070)").

## Escaped quote inside a raw-string assertion message (amazon-q, tests.rs:58931) — LOW-CONFIDENCE, verify
> Escape Sequence Error: The escaped single quote in the assertion message will cause a compilation
> error. Raw string literals (r#"..."#) cannot contain escape sequences.

⚠ Likely a FALSE POSITIVE on the "compilation error" claim: Rust raw strings (`r#"..."#`) treat `\`
literally and compile fine — a `\'` there is legal, it just embeds a literal backslash+quote in the
message text rather than erroring. Since the PR gated green, it is NOT a compile error. BUT the
underlying nit may still hold: if the intent was a line-continuation (`\` at end of line) or a plain
apostrophe, the current text may contain an unintended stray backslash. Worth a quick look at the
message string; amazon-q's suggested edit uses a trailing `\` line-continuation (valid in a
NON-raw string, not a raw one) — don't apply blindly.
