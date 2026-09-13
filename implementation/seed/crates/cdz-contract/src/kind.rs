//! The mutation-vs-query classification of a contract (654(a)).
//!
//! A contract is either a **mutation** or a **query**, and that class decides how the platform routes an
//! event addressed to it (`design/cadenza-platform.md` §1/§3):
//! - a **mutation** goes to the durable single-writer session — the serialized owner of the reducer's
//!   state;
//! - a **query** is routed to a FORKED read-only snapshot session, kept OFF the single-writer critical
//!   path, so reads never contend with the writer.
//!
//! The whole point of 654(a) is that this classification **travels with the contract-id**: it rides in the
//! id's leading [`HashTag`] byte (the "hash prefix"), so the router reads the class straight out of the id
//! with no side table. [`ContractKind`] is the typed name of that class, and [`route_tag`](ContractKind::route_tag)
//! / [`from_tag`](ContractKind::from_tag) are the exact byte↔class mapping the id and the router share:
//! [`Mutation`](ContractKind::Mutation) ↔ [`HashTag::Contract`], [`Query`](ContractKind::Query) ↔
//! [`HashTag::ContractQuery`]. Mutation is the DEFAULT — every pre-existing contract-id is a mutation
//! (`HashTag::Contract`), so adding the query class drifts no existing id.

use crate::HashTag;

/// How a contract routes: a [`Mutation`](Self::Mutation) to the durable single-writer session, or a
/// [`Query`](Self::Query) to a forked read-only snapshot session (654(a)). The class rides in the
/// contract-id's leading tag byte, so it is read straight off the id — this is the typed name of that byte's
/// two contract meanings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ContractKind {
    /// The contract mutates state: routed to the durable single-writer session. The default class —
    /// [`HashTag::Contract`], the tag every pre-existing contract-id already carries.
    Mutation,
    /// The contract only reads state: routed to a forked read-only snapshot session, off the single-writer
    /// critical path — [`HashTag::ContractQuery`].
    Query,
}

impl ContractKind {
    /// The [`HashTag`] that marks a contract-id of this class — the leading byte the router reads the class
    /// out of. [`Mutation`](Self::Mutation) → [`HashTag::Contract`]; [`Query`](Self::Query) →
    /// [`HashTag::ContractQuery`].
    #[must_use]
    pub const fn route_tag(self) -> HashTag {
        match self {
            Self::Mutation => HashTag::Contract,
            Self::Query => HashTag::ContractQuery,
        }
    }

    /// The contract class a [`HashTag`] names, or `None` if the tag is not a contract-id tag (a reducer/
    /// program/host/blob/system tag names no contract, so it has no mutation-vs-query class). The inverse of
    /// [`route_tag`](Self::route_tag) over the two contract tags.
    #[must_use]
    pub const fn from_tag(tag: HashTag) -> Option<Self> {
        match tag {
            HashTag::Contract => Some(Self::Mutation),
            HashTag::ContractQuery => Some(Self::Query),
            _ => None,
        }
    }

    /// Whether this is a [`Query`](Self::Query) — the read-only-snapshot class.
    #[must_use]
    pub const fn is_query(self) -> bool {
        matches!(self, Self::Query)
    }

    /// Whether this is a [`Mutation`](Self::Mutation) — the single-writer class.
    #[must_use]
    pub const fn is_mutation(self) -> bool {
        matches!(self, Self::Mutation)
    }
}

#[cfg(test)]
mod tests {
    use super::ContractKind;
    use crate::HashTag;

    #[test]
    fn route_tag_and_from_tag_are_inverse_over_the_two_contract_classes() {
        // The class ↔ tag mapping is the shared contract between the id and the router: mutation rides the
        // pre-existing Contract tag, query rides the new ContractQuery tag.
        assert_eq!(ContractKind::Mutation.route_tag(), HashTag::Contract);
        assert_eq!(ContractKind::Query.route_tag(), HashTag::ContractQuery);
        assert_eq!(
            ContractKind::from_tag(HashTag::Contract),
            Some(ContractKind::Mutation)
        );
        assert_eq!(
            ContractKind::from_tag(HashTag::ContractQuery),
            Some(ContractKind::Query)
        );
        // Round-trips both ways.
        for kind in [ContractKind::Mutation, ContractKind::Query] {
            assert_eq!(ContractKind::from_tag(kind.route_tag()), Some(kind));
        }
    }

    #[test]
    fn a_non_contract_tag_names_no_class() {
        // A reducer/program/host/blob/system hash is not a contract-id, so it has no mutation-vs-query class.
        for tag in [
            HashTag::Reducer,
            HashTag::Program,
            HashTag::Host,
            HashTag::Blob,
            HashTag::SystemProperty,
        ] {
            assert_eq!(ContractKind::from_tag(tag), None);
        }
    }

    #[test]
    fn is_query_and_is_mutation_agree_with_the_class() {
        assert!(ContractKind::Query.is_query());
        assert!(!ContractKind::Query.is_mutation());
        assert!(ContractKind::Mutation.is_mutation());
        assert!(!ContractKind::Mutation.is_query());
    }
}
