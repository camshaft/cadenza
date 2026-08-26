{
  registry = {
    default = "reducer-default-handler-cdz",
    handlers = [{ contract = "cdz-platform.effect", program = "reducer-override-handler-cdz" }]
  },
  blobs = [
    { name = "reducer-default-handler-cdz", program = "reducer-default-handler-cdz" },
    { name = "reducer-override-handler-cdz", program = "reducer-override-handler-cdz" },
    { name = "reducer-echo-cdz", program = "reducer-echo-cdz" },
    { name = "reducer-override-check-cdz", program = "reducer-override-check-cdz" }
  ],
  spawns = [{ name = "reducer-echo-cdz", blob = "reducer-echo-cdz" }],
  deliver = [
    { target = "reducer-echo-cdz", message = { contract = "cdz-platform.effect", payload = b"OVERRIDEEFFECT" } }
  ],
  checker = "reducer-override-check-cdz"
}
