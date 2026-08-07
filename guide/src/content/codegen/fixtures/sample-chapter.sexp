(chapter
  (slug "sample-chapter")
  (title "A Sample Chapter")
  (pillar "platform")
  (section "The kernel model")
  (blurb "A codegen fixture exercising every I4 schema head.")
  (lede
    "This fixture drives the sexp→TSX codegen gate: it uses " (em "every") " head the I4 schema "
    "defines, so a regression in parse or render trips " (c "check-codegen") ".")
  (h2 "Prose and emphasis")
  (p "An ordinary paragraph with " (em "emphasis") ", some " (c "inline code") ", and a link to "
    (link (slug "effects") "effects & handlers") " plus the " (app-link (route "/explorer") "platform explorer") ".")
  (h2 "A pseudocode note")
  (note "on an event arriving in session S:" (br)
    "  run S's reducer over the event → a list of effects" (br)
    "  for each effect: if authorized, perform it; else append a denial" (br)
    "  each appended result is a new event → the loop runs again"))
