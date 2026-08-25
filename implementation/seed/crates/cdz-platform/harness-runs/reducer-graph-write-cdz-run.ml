{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-graph-write-cdz", program = "reducer-graph-write-cdz" },
    { name = "reducer-graph-write-check-cdz", program = "reducer-graph-write-check-cdz" }
  ],
  spawns = [{ name = "reducer-graph-write-cdz", blob = "reducer-graph-write-cdz", kind = "event" }],
  deliver = [
    {
      target = "reducer-graph-write-cdz",
      message = { contract = "cdz-platform.deliver", payload = b"GRAPHWRITE", token = b"gwtoken" }
    }
  ],
  checker = "reducer-graph-write-check-cdz"
}
