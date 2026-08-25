{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-state-delete-cdz", program = "reducer-state-delete-cdz" },
    { name = "reducer-state-delete-check-cdz", program = "reducer-state-delete-check-cdz" }
  ],
  spawns = [{ name = "reducer-state-delete-cdz", blob = "reducer-state-delete-cdz" }],
  deliver = [
    {
      target = "reducer-state-delete-cdz",
      message = { contract = "cdz-platform.deliver", payload = b"DELETEME" }
    }
  ],
  checker = "reducer-state-delete-check-cdz"
}
