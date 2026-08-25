{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-state-cdz", program = "reducer-state-cdz" },
    { name = "reducer-state-delete-check-cdz", program = "reducer-state-delete-check-cdz" }
  ],
  spawns = [{ name = "reducer-state-cdz", blob = "reducer-state-cdz" }],
  deliver = [
    { target = "reducer-state-cdz", message = { contract = "cdz-platform.deliver", payload = b"STATEVALUE" } },
    { target = "reducer-state-cdz", message = { contract = "cdz-platform.deliver", payload = b"delete" } }
  ],
  checker = "reducer-state-delete-check-cdz"
}
