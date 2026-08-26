{
  registry = { default = "reducer-default-handler-cdz" },
  blobs = [
    { name = "reducer-default-handler-cdz", program = "reducer-default-handler-cdz" },
    { name = "reducer-echo-cdz", program = "reducer-echo-cdz" },
    { name = "reducer-graph-forward-check-cdz", program = "reducer-graph-forward-check-cdz" }
  ],
  spawns = [
    { name = "reducer-echo-cdz", blob = "reducer-echo-cdz" },
    { name = "transform-echo-cdz", blob = "reducer-echo-cdz" }
  ],
  edges = [
    { from = "reducer-echo-cdz", contract = "cdz-platform.effect", to = ["transform-echo-cdz"] }
  ],
  deliver = [
    { target = "reducer-echo-cdz", message = { contract = "cdz-platform.effect", payload = b"EFFECTPAYLOAD" } }
  ],
  checker = "reducer-graph-forward-check-cdz"
}
