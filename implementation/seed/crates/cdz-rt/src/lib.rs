//! The shared native runtime interface for the Rust backend's emitted code.
//!
//! A Cadenza module compiled to Rust (`rcdzc --target rust`/`rust-async`) links against this crate
//! instead of carrying its own copy of the runtime traits. Two things live here so an application
//! defines them ONCE and every emitted module shares them:
//!
//! - [`CdzEnv`] — the gas/yield capability the async/gas-metered backend threads through every emitted
//!   function. Previously each async module emitted its OWN `CdzEnv` trait, so two modules had two
//!   incompatible env types and an application had to implement the capability once per module. With a
//!   single shared trait, the application implements it once and every module interoperates.
//!
//! Later increments add the VALUE-RUNTIME seam (`CdzRuntime` / `CdzRuntimeAsync`) — a trait the emitted
//! code calls for compound operations (list/string/bytes/map/set) so the CALLER chooses the
//! representation (a `Vec`, a persistent vector, an arena) — plus a default `RcRuntime` wiring.
//!
//! Dep-free and `no_std`-friendly in spirit (uses only `core::future`), so linking it into an existing
//! Rust codebase adds no transitive weight.

/// The gas/yield capability the async, gas-metered Rust backend threads through every emitted function.
///
/// An emitted `async fn` awaits `env.consume(1)` at entry, so the host meters fuel and MAY perform a
/// cooperative yield inside `consume` (return control to the executor after accounting) — a runaway or
/// long-running computation is then bounded at the granularity of a call. `consume` returns
/// `impl Future` (RPITIT) rather than being written `async fn` in the trait, so an implementor needs no
/// `async_trait` dependency and the emitted call site stays lint-clean.
///
/// A typical implementation increments a counter and, past a budget, either never resolves the future
/// (the executor drops the task) or panics — the emitted code is agnostic to that policy; it only
/// awaits the charge.
pub trait CdzEnv {
    /// Charge `gas` units of fuel; the returned future MAY yield cooperatively before resolving.
    fn consume(&mut self, gas: u64) -> impl core::future::Future<Output = ()>;
}

/// An OBJECT-SAFE facet of [`CdzEnv`] — the same `consume` capability, but returning a BOXED future so it
/// can be called through a `&mut dyn` env. `CdzEnv::consume` is an RPITIT (`-> impl Future`), which is NOT
/// object-safe, so a lambda-lifted async closure typed `Rc<dyn Fn(&mut dyn DynCdzEnv, A) -> Pin<Box<dyn
/// Future<Output = R> + '_>>>` (the rust-async closure ABI) cannot `await` through a bare `dyn CdzEnv`. The
/// blanket impl below makes EVERY `CdzEnv` a `DynCdzEnv` for free, so an emitted async-closure body boxes the
/// `consume` future once at the `dyn` boundary. Purely additive — `CdzEnv` itself is unchanged, no behavior
/// changes, and this touches only the rust-backend rlib (NOT the wasm value-heap `cdz-runtime` component, so
/// `REQUIRED_RUNTIME_HASH` is unaffected and no `xtask codegen` is needed).
pub trait DynCdzEnv {
    /// [`CdzEnv::consume`] behind a boxed future, callable on a `&mut dyn DynCdzEnv`.
    fn consume_boxed(
        &mut self,
        gas: u64,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = ()> + '_>>;
}

impl<E: CdzEnv> DynCdzEnv for E {
    fn consume_boxed(
        &mut self,
        gas: u64,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = ()> + '_>> {
        Box::pin(self.consume(gas))
    }
}

/// An object-safe, callable-repeatedly async closure whose returned future BORROWS the `&mut env` passed at
/// the call — the rust-async lambda-lifted-closure value representation. A bare `dyn Fn(&mut dyn DynCdzEnv, A)
/// -> Pin<Box<dyn Future<Output = R> + '_>>` CANNOT express this: a closure cannot be generic/HRTB over its
/// OWN parameter lifetimes, so the `'_` on the boxed future can't be tied to the call's `&'a mut env` borrow
/// (E0271 / "lifetime may not live long enough"). A trait with a GENERIC METHOD `call<'a>` can — `'a` ties the
/// returned future to the env borrow — and is object-safe, so an env-borrowing async closure value can be
/// represented as `Rc<dyn EnvClosure<A, R>>`: a per-closure struct (its captures as fields) whose `call` boxes
/// the body's env-borrowing future. `A`/`R` are the closure's (single) argument and result machine types (a
/// multi-arg closure tuples its args into `A`, matching the lifted-lambda calling convention). This is the
/// object-safe closure-value ABI a backend targets for an env-threading async closure.
///
/// Additive like [`DynCdzEnv`]: rust-backend rlib only, NO change to `CdzEnv`, and it does NOT touch the wasm
/// value-heap `cdz-runtime` component, so `REQUIRED_RUNTIME_HASH` is unaffected (no `xtask codegen`).
pub trait EnvClosure<A, R> {
    /// Apply the closure to `arg` with the ambient `env`; the returned future may `await` through `env` and
    /// borrows it for `'a`. Callable repeatedly (`&self`), unlike an `FnOnce`.
    fn call<'a>(
        &self,
        env: &'a mut dyn DynCdzEnv,
        arg: A,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = R> + 'a>>;
}

#[cfg(test)]
mod dyn_env_tests {
    use super::*;

    /// A minimal `CdzEnv` counting consumed gas; its `consume` future resolves immediately.
    struct Counter {
        used: u64,
    }
    impl CdzEnv for Counter {
        fn consume(&mut self, gas: u64) -> impl core::future::Future<Output = ()> {
            self.used += gas;
            async {}
        }
    }

    /// Poll a `'_`-borrowing future to completion on a no-op waker (the future never yields here).
    fn block_on(mut fut: core::pin::Pin<Box<dyn core::future::Future<Output = ()> + '_>>) {
        use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        const VT: RawWakerVTable = RawWakerVTable::new(
            |_| RawWaker::new(core::ptr::null(), &VT),
            |_| {},
            |_| {},
            |_| {},
        );
        let waker = unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VT)) };
        let mut cx = Context::from_waker(&waker);
        loop {
            if let Poll::Ready(()) = fut.as_mut().poll(&mut cx) {
                break;
            }
        }
    }

    #[test]
    fn dyn_cdz_env_is_object_safe_and_consume_boxed_awaits_through_a_dyn_ref() {
        // The whole point: DynCdzEnv IS object-safe (a `&mut dyn DynCdzEnv` compiles — CdzEnv itself does not),
        // and `consume_boxed` awaits through it, charging gas via the underlying CdzEnv.
        let mut counter = Counter { used: 0 };
        let env: &mut dyn DynCdzEnv = &mut counter;
        block_on(env.consume_boxed(7));
        block_on(env.consume_boxed(5));
        assert_eq!(
            counter.used, 12,
            "consume_boxed threads gas through the underlying CdzEnv"
        );
    }

    /// Poll an `R`-yielding `'_`-borrowing future to completion (the EnvClosure result variant of `block_on`).
    fn block_on_val<R>(
        mut fut: core::pin::Pin<Box<dyn core::future::Future<Output = R> + '_>>,
    ) -> R {
        use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        const VT: RawWakerVTable = RawWakerVTable::new(
            |_| RawWaker::new(core::ptr::null(), &VT),
            |_| {},
            |_| {},
            |_| {},
        );
        let waker = unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VT)) };
        let mut cx = Context::from_waker(&waker);
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    /// A lambda-lifted async closure: captures `base`, its future BORROWS the call's `&mut env` (awaits gas
    /// through it) — the exact shape a bare `dyn Fn` cannot express.
    struct Adder {
        base: i64,
    }
    impl EnvClosure<i64, i64> for Adder {
        fn call<'a>(
            &self,
            env: &'a mut dyn DynCdzEnv,
            arg: i64,
        ) -> core::pin::Pin<Box<dyn core::future::Future<Output = i64> + 'a>> {
            let base = self.base;
            Box::pin(async move {
                env.consume_boxed(1).await; // the returned future borrows env for 'a — the whole point
                base + arg
            })
        }
    }

    #[test]
    fn env_closure_is_object_safe_and_its_future_borrows_the_call_env() {
        // Stored as a trait object, called repeatedly through a `&mut dyn DynCdzEnv`; the returned future ties
        // to the env borrow and awaits through it. A `dyn Fn` returning such a future does NOT type-check.
        use std::rc::Rc;
        let c: Rc<dyn EnvClosure<i64, i64>> = Rc::new(Adder { base: 100 });
        let mut counter = Counter { used: 0 };
        let r1 = block_on_val(c.call(&mut counter, 5));
        let r2 = block_on_val(c.call(&mut counter, 40));
        assert_eq!(r1, 105);
        assert_eq!(r2, 140);
        assert_eq!(
            counter.used, 2,
            "each call awaited one gas unit through the borrowed env"
        );
    }
}
