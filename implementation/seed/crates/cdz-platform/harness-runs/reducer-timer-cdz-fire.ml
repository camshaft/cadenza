{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-timer-cdz", program = "reducer-timer-cdz" },
    { name = "reducer-timer-check-cdz", program = "reducer-timer-check-cdz" }
  ],
  spawns = [{ name = "reducer-timer-cdz", blob = "reducer-timer-cdz" }],
  deliver = [
    { target = "reducer-timer-cdz", message = { contract = "cdz-platform.timer", payload = b"go" } }
  ],
  checker = "reducer-timer-check-cdz"
}
