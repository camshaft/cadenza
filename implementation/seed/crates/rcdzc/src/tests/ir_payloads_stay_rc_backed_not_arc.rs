/// `MARKER` is unambiguous ONLY for a `!Send` type: the blanket impl always applies, the `Send`-gated
/// impl applies only for a `Send` type — so a `Send` type has TWO candidate impls and the path is
/// ambiguous (E0283). The classic `static_assertions::assert_not_impl_all!(T: Send)` idiom, inlined so
/// the crate needs no extra dependency.
trait AmbiguousIfSend<A> {
    const MARKER: () = ();
}
impl<T: ?Sized> AmbiguousIfSend<()> for T {}
impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}

// Naming `MARKER` forces impl selection; the path resolves iff the type is `!Send` (Rc-backed).
// Evaluating each associated const in an ANONYMOUS `const _` item IS the compile-time assertion — the
// crate fails to compile if any of these types becomes `Send` (a wholesale `Arc` revert). `const _`
// never triggers dead-code (unnamed) and needs no runtime test — it is checked at every build.
const _: () = <crate::resolved::Resolved as AmbiguousIfSend<_>>::MARKER;
const _: () = <crate::ty::Ty as AmbiguousIfSend<_>>::MARKER;
const _: () = <crate::core::Core as AmbiguousIfSend<_>>::MARKER;
