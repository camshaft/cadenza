
**✅ DONE 2026-07-07 (Run 114) — moved pending→done.** Re-confirmed on compiler.cdz 19:10: bare float → decline
(no trap), and the whole byte gate is GREEN (0 disagree, 0 `run error`). The follow-on ask-56 (wrong code) also
landed (via the ask-54 KFloat work), so the entire float story is closed: floats decline where unimplemented, and
int/float mixes reject with the correct CDZ0301. No crash-on-valid-input remains.
