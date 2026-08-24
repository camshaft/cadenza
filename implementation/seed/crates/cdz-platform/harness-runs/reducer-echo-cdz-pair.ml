{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-echo-cdz", program = "reducer-echo-cdz" },
    { name = "reducer-check-cdz", program = "reducer-check-cdz" }
  ],
  spawns = [
    { name = "echo-a", blob = "reducer-echo-cdz" },
    { name = "echo-b", blob = "reducer-echo-cdz" }
  ],
  deliver = [
    {
      target = "echo-a",
      message = { contract = "cdz-platform.deliver", payload = b"PAYLOAD-A" }
    },
    {
      target = "echo-b",
      message = { contract = "cdz-platform.deliver", payload = b"PAYLOAD-B" }
    }
  ],
  checker = "reducer-check-cdz"
}
