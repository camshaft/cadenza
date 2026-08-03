# HEADS-UP: runtime-NFC cases NOT corpus-pinnable cross-backend until the rust NFC arm lands

breaker (2026-08-03): the #1616 NFC landing's rust arm is WIP (saved nfc-wip-save per v-runtime's log).
On the RUST backend, runtime-NFC construction (multibyte e+combining-mark concat / Symbol.of) produces
WRONG VALUES (un-normalized byte counts: 530 vs wasm's 421; 43 vs 32) — a FAIL, NOT a benign todo.
Grader consequence: a runtime-NFC case that PASSES on wasm shows rust FAIL (not todo), so it CANNOT be
corpus-pinned cross-backend yet. breaker's 8-probe NFC battery is banked, ready to pin the moment the
rust NFC arm lands. Also: this is a SILENT value divergence for rust-target users of multibyte runtime
concat/Symbol.of (literal path is fine — reader-normalized; only runtime construction diverges).
ACTION: do NOT attempt runtime-NFC corpus pins until v-runtime/v-rust-backend land the rust arm; watch
for that landing, then pin breaker's battery. (Not a new adv — tracked in the #1616 landing + saved WIP.)
