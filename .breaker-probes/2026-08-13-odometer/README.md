# 2026-08-13 base-100 odometer (tick 1395)

- `odo1.sexp` — the list state is a wheel array: a recursive carry-add helper in
  the arm rebuilds cells by List.update as the carry cascades and GROWS the list
  (List.push 0) when it overflows the top wheel — mixing in-place update and
  growth in ONE recursive walk per dispatch (cst1 mixes them across ops; here
  one helper interleaves them within a dispatch, depth data-dependent: the 9990
  tick pushes two new wheels). Budget-guard inherent (carry strictly shrinks /
  list strictly grows). PASS ×3 (153213303/245205395).
