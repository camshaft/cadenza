//! The reducer-world `guest` export (`cadenza:platform/guest`) — the wasm component boundary. A compute
//! target is a PURE `run` guest: it folds one `on-message` and closes immediately, putting its output value
//! in the `close` reason (that is how the host's `run` reads a program's output inline — design
//! `DESIGN-reducer-targets.md` §4). `on-response`/`on-notification` are not part of a pure `run` and close
//! with an empty reason. Built for wasm32-unknown-unknown (WASI-free), so the component imports only the
//! reducer world and instantiates on the platform's pure linker.

use crate::bindings::cadenza::platform::reducer::{Closed, Outcome};
use crate::bindings::exports::cadenza::platform::guest::{
    Guest, Message, Notification, Response, Step,
};

struct Component;

impl Guest for Component {
    /// Fold a request: run the active target's handler on `msg.payload`, return its output as the close
    /// reason. `schema` echoes the request's contract-id (the reason is that contract's OUTPUT value).
    fn on_message(msg: Message) -> Step {
        let reason = dispatch(&msg.payload);
        Step {
            requests: Vec::new(),
            outcome: Outcome::Close(Closed {
                schema: msg.contract,
                reason,
            }),
        }
    }

    fn on_response(_resp: Response) -> Step {
        close_empty()
    }

    fn on_notification(_note: Notification) -> Step {
        close_empty()
    }
}

/// A no-op close — a pure `run` guest is only driven through `on-message`; the other folds close inertly.
fn close_empty() -> Step {
    Step {
        requests: Vec::new(),
        outcome: Outcome::Close(Closed {
            schema: Vec::new(),
            reason: Vec::new(),
        }),
    }
}

/// Run the active per-target handler (exactly one target feature is active per build — nix stamps one
/// component per target with `--no-default-features --features target-<T>`). The per-target difference is
/// only which library function is wrapped + its payload codec.
#[cfg(feature = "target-rcdzc")]
fn dispatch(payload: &[u8]) -> Vec<u8> {
    crate::rcdzc_target::handle(payload)
}

#[cfg(feature = "target-sexpr")]
fn dispatch(payload: &[u8]) -> Vec<u8> {
    crate::sexpr_target::handle(payload)
}

#[cfg(feature = "target-ml")]
fn dispatch(payload: &[u8]) -> Vec<u8> {
    crate::ml_target::handle(payload)
}

crate::bindings::export!(Component with_types_in crate::bindings);
