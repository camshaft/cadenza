{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-state-cdz", program = "reducer-state-cdz" },
    { name = "reducer-state-check-cdz", program = "reducer-state-check-cdz" }
  ],
  spawns = [{ name = "reducer-state-cdz", blob = "reducer-state-cdz" }],
  deliver = [
    { target = "reducer-state-cdz", message = { contract = "cdz-platform.deliver", payload = b"STATEVALUE" } },
    { target = "reducer-state-cdz", message = { contract = "cdz-platform.deliver", payload = b"get" } }
  ],
  checker = "reducer-state-check-cdz"
}
