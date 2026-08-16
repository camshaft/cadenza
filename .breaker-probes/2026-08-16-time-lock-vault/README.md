# vlt1 — time-lock vault with duress code (2026-08-16, tick 1641)

Attack: a 3-way code dispatch (correct/duress/wrong) where the CORRECT branch
nests a timer-zero test — 4 leaves with distinct state-touch patterns: unlock
mutates locked only; still-counting resumes st untouched; duress mutates TWO
fields (alarms + timer PENALTY, the timer moving the WRONG direction vs
ticks); wrong resumes st untouched. tick has its own floor guard. Cross-op
coupling: duress's +2 penalty interacts with tick's -1 countdown so the
final correct entry races the timer.

Differential: seed sets the initial timer (1 vs 3): n=0's countdown reaches
zero before the last entry (vault OPENS, 111, status 100 with locked=0);
n=10's duress penalty still holds it shut (202, status 121). Rows diverge
from position 1 (different timer) AND in kind at position 5.

Hand model: n=10 → 21901031021202121; n=0 → 1901011001111100 (base-1000).
Trimmed from 7 ops after an Int64 overflow assert.

Pass ×3 wasm + rust + rust-async on trunk 3c06de590 (B2 gate-3 pin landed).
