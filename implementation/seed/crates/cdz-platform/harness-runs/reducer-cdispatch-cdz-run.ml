{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-cdispatch-cdz", program = "reducer-cdispatch-cdz" },
    { name = "reducer-cdispatch-check-cdz", program = "reducer-cdispatch-check-cdz" }
  ],
  spawns = [{ name = "reducer-cdispatch-cdz", blob = "reducer-cdispatch-cdz" }],
  deliver = [
    { target = "reducer-cdispatch-cdz", message = { contract = "cdz-platform.effect", payload = b"E" } },
    { target = "reducer-cdispatch-cdz", message = { contract = "cdz-platform.timer", payload = b"T" } }
  ],
  checker = "reducer-cdispatch-check-cdz"
}
