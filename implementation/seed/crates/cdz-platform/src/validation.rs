//! Validating a payload against its contract (`design/cadenza-platform.md` §4).
//!
//! A contract declares the type of its input and output, so a payload either *is* a value of that type or it
//! is not — and the platform checks rather than trusting the emitter. The check is not a program someone
//! registers against a contract; it is **derived from the schema** by two ordinary pure-function runs (§3):
//!
//! 1. compile the schema into a validator — `run(compiler, schema) -> validator-hash`;
//! 2. check the payload with it — `run(validator, payload) -> verdict`.
//!
//! Both are memoized by the [`Runner`], so a schema compiles once ever per `(compiler, schema)` and a
//! repeated payload validates for free. The **compiler is a parameter, not baked in** — it is just another
//! content-addressed program — so validation semantics are pinned by the chosen compiler and evolve by
//! pointing at a new one, without changing any contract-id.
//!
//! This is the validation **mechanism**. *When* to apply it — only to inputs that came from outside,
//! trusting an input the reducer emitted itself (grounded by the `program_of` provenance read so the check
//! does not recurse, §4) — is the event reducer's policy, layered over this. A non-conforming payload is a
//! [`ValidateError::SchemaViolation`], which the event reducer turns into the `Err(schema-violation)` that
//! bubbles back to the sender (§4) — an ordinary recorded outcome, not a special kernel event.

use crate::{Bytes, ContractId, ProgramHash, ProgramStore, RunError, Runner};
use cadenza_ast::ast::{Leaf, Struct};
use cadenza_ast::codec;

/// Why validating a payload against its contract did not conclude "conforms".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidateError {
    /// The payload is a well-formed check that concluded the payload is **not** a value of the contract's
    /// declared type. This is the ordinary "malformed payload" outcome the event reducer answers with
    /// `Err(schema-violation)` (§4).
    SchemaViolation,
    /// Compiling the schema to a validator (`run(compiler, schema)`) did not produce an output.
    CompileFailed(RunError),
    /// Running the validator on the payload (`run(validator, payload)`) did not produce an output.
    CheckFailed(RunError),
    /// The compiler's output is not a valid program-hash, so no validator could be run.
    MalformedValidatorHash,
    /// The validator's output is not a verdict this platform understands (a canonical boolean value).
    MalformedVerdict,
}

/// Validate `payload` against the contract whose `schema` is given, using `compiler` to derive the validator
/// (§4). Returns `Ok(())` when the payload conforms, `Err(ValidateError::SchemaViolation)` when it provably
/// does not, and the other [`ValidateError`]s when the check itself could not be carried out.
///
/// `compile_contract` and `validate_contract` are the contract-ids the schema and payload are delivered
/// against in the two pure runs — the contracts the compiler and the validator answer. They are parameters
/// (not fixed here) so the validation interface is pinned by the caller alongside the `compiler` it chooses,
/// exactly as the compiler is a parameter and not baked in.
///
/// The verdict is a canonical boolean value (`true` = conforms); a validator that emits anything else is a
/// [`ValidateError::MalformedVerdict`]. A richer verdict (a reason for the violation) is a later refinement
/// — the platform's outcome here is the binary conforms-or-not §4 specifies.
pub async fn validate<P: ProgramStore + ?Sized>(
    runner: &Runner<P>,
    compiler: ProgramHash,
    compile_contract: ContractId,
    validate_contract: ContractId,
    schema: Bytes,
    payload: Bytes,
) -> Result<(), ValidateError> {
    // 1. Compile the schema to a validator program (memoized per (compiler, schema)).
    let validator_bytes = runner
        .run(compiler, compile_contract, schema)
        .await
        .map_err(ValidateError::CompileFailed)?;
    let validator = ProgramHash::try_from(validator_bytes.as_ref())
        .map_err(|_| ValidateError::MalformedValidatorHash)?;

    // 2. Check the payload with the validator (memoized per (validator, payload)).
    let verdict = runner
        .run(validator, validate_contract, payload)
        .await
        .map_err(ValidateError::CheckFailed)?;

    match decode_bool(&verdict) {
        Some(true) => Ok(()),
        Some(false) => Err(ValidateError::SchemaViolation),
        None => Err(ValidateError::MalformedVerdict),
    }
}

/// Decode a canonical boolean value (the verdict), or `None` if the bytes are not a bare boolean — total,
/// so a malformed verdict is a rejected check, not a panic.
fn decode_bool(bytes: &[u8]) -> Option<bool> {
    let arenas = codec::decode(bytes)?;
    match arenas.get(arenas.root) {
        Struct::Atom(leaf) => match arenas.leaf(*leaf) {
            Leaf::Bool(b) => Some(*b),
            _ => None,
        },
        Struct::List(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{ValidateError, validate};
    use crate::{
        Bytes, ContractId, Message, Notification, Outcome, ProgramHash, Reducer, Request, Response,
        Runner,
    };
    use cadenza_ast::ast::{Builder, Leaf};
    use cadenza_ast::codec;
    use std::sync::Arc;

    fn cid(tag: &[u8]) -> ContractId {
        ContractId::of(tag)
    }
    fn prog(tag: &[u8]) -> ProgramHash {
        ProgramHash::of(tag)
    }

    /// Encode a canonical boolean value — what a validator returns as its verdict.
    fn encode_bool(b: bool) -> Bytes {
        let mut builder = Builder::new();
        let root = builder.atom_leaf(Leaf::Bool(b));
        Bytes::from(codec::encode(&builder.finish(root)))
    }

    /// A stub compiler: on any schema it `Break`s with the raw hash bytes of `validator` — as if it compiled
    /// the schema into that validator program.
    struct StubCompiler {
        validator: ProgramHash,
    }
    #[async_trait::async_trait]
    impl Reducer for StubCompiler {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (
                Vec::new(),
                Outcome::Break {
                    schema: cid(b"validator-hash"),
                    reason: Bytes::copy_from_slice(self.validator.hash().as_bytes()),
                },
            )
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    /// A stub validator: conforms iff the payload equals `b"good"`, returning a boolean verdict.
    struct StubValidator;
    #[async_trait::async_trait]
    impl Reducer for StubValidator {
        async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
            let conforms = m.payload.as_ref() == b"good";
            (
                Vec::new(),
                Outcome::Break {
                    schema: cid(b"verdict"),
                    reason: encode_bool(conforms),
                },
            )
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    /// A stub whose output is not a program-hash (too short) — a broken compiler.
    struct BadCompiler;
    #[async_trait::async_trait]
    impl Reducer for BadCompiler {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (
                Vec::new(),
                Outcome::Break {
                    schema: cid(b"x"),
                    reason: Bytes::from_static(b"not a hash"),
                },
            )
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    /// A stub validator whose output is not a boolean — a broken validator.
    struct NonBoolValidator;
    #[async_trait::async_trait]
    impl Reducer for NonBoolValidator {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (
                Vec::new(),
                Outcome::Break {
                    schema: cid(b"verdict"),
                    reason: Bytes::from_static(b"garbage"),
                },
            )
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    /// A runner over a store with `compiler` registered to produce `validator`, plus `validator` itself.
    fn runner_with(
        compiler: ProgramHash,
        make_compiler: impl Fn() -> Box<dyn Reducer> + Send + Sync + 'static,
        validator: ProgramHash,
        make_validator: impl Fn() -> Box<dyn Reducer> + Send + Sync + 'static,
    ) -> Runner<crate::testing::program::Store> {
        let mut store = crate::testing::program::Store::new();
        store.register(compiler, make_compiler);
        store.register(validator, make_validator);
        Runner::new(Arc::new(store))
    }

    #[tokio::test]
    async fn a_conforming_payload_validates() {
        let runner = runner_with(
            prog(b"compiler"),
            || {
                Box::new(StubCompiler {
                    validator: prog(b"validator"),
                })
            },
            prog(b"validator"),
            || Box::new(StubValidator),
        );
        assert_eq!(
            validate(
                &runner,
                prog(b"compiler"),
                cid(b"compile"),
                cid(b"validate"),
                Bytes::from_static(b"schema"),
                Bytes::from_static(b"good"),
            )
            .await,
            Ok(())
        );
    }

    #[tokio::test]
    async fn a_non_conforming_payload_is_a_schema_violation() {
        let runner = runner_with(
            prog(b"compiler"),
            || {
                Box::new(StubCompiler {
                    validator: prog(b"validator"),
                })
            },
            prog(b"validator"),
            || Box::new(StubValidator),
        );
        assert_eq!(
            validate(
                &runner,
                prog(b"compiler"),
                cid(b"compile"),
                cid(b"validate"),
                Bytes::from_static(b"schema"),
                Bytes::from_static(b"bad"),
            )
            .await,
            Err(ValidateError::SchemaViolation)
        );
    }

    #[tokio::test]
    async fn a_compiler_that_yields_a_non_hash_is_malformed() {
        let runner = runner_with(
            prog(b"compiler"),
            || Box::new(BadCompiler),
            prog(b"validator"),
            || Box::new(StubValidator),
        );
        assert_eq!(
            validate(
                &runner,
                prog(b"compiler"),
                cid(b"compile"),
                cid(b"validate"),
                Bytes::from_static(b"schema"),
                Bytes::from_static(b"good"),
            )
            .await,
            Err(ValidateError::MalformedValidatorHash)
        );
    }

    #[tokio::test]
    async fn a_validator_that_yields_a_non_bool_is_a_malformed_verdict() {
        let runner = runner_with(
            prog(b"compiler"),
            || {
                Box::new(StubCompiler {
                    validator: prog(b"validator"),
                })
            },
            prog(b"validator"),
            || Box::new(NonBoolValidator),
        );
        assert_eq!(
            validate(
                &runner,
                prog(b"compiler"),
                cid(b"compile"),
                cid(b"validate"),
                Bytes::from_static(b"schema"),
                Bytes::from_static(b"good"),
            )
            .await,
            Err(ValidateError::MalformedVerdict)
        );
    }

    #[tokio::test]
    async fn an_unknown_compiler_is_a_compile_failure() {
        // Nothing registered: the compiler program cannot be instantiated, so the compile run fails.
        let store = crate::testing::program::Store::new();
        let runner = Runner::new(Arc::new(store));
        assert_eq!(
            validate(
                &runner,
                prog(b"absent"),
                cid(b"compile"),
                cid(b"validate"),
                Bytes::from_static(b"schema"),
                Bytes::from_static(b"good"),
            )
            .await,
            Err(ValidateError::CompileFailed(
                crate::RunError::UnknownProgram
            ))
        );
    }
}
