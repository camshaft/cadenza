{
  registry = { default = "reducer-default-handler-cdz" },
  blobs = [
    { name = "reducer-default-handler-cdz", program = "reducer-default-handler-cdz" },
    { name = "reducer-echo-cdz", program = "reducer-echo-cdz" },
    { name = "reducer-graph-check-cdz", program = "reducer-graph-check-cdz" }
  ],
  spawns = [{ name = "reducer-echo-cdz", blob = "reducer-echo-cdz" }],
  deliver = [
    { target = "reducer-echo-cdz", message = { contract = "cdz-platform.effect", payload = b"EFFECTPAYLOAD" } }
  ],
  checker = "reducer-graph-check-cdz"
}
