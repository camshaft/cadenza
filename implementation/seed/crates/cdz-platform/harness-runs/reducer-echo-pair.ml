{
  system = "$system",
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-echo", program = "reducer-echo" },
    { name = "reducer-echo-check", program = "reducer-echo-check" }
  ],
  spawns = [
    { name = "echo-a", blob = "reducer-echo" },
    { name = "echo-b", blob = "reducer-echo" }
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
  checker = "reducer-echo-check"
}
