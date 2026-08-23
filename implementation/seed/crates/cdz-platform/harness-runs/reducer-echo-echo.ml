{
  system = "$system",
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-echo", program = "reducer-echo" },
    { name = "reducer-echo-check", program = "reducer-echo-check" }
  ],
  spawns = [{ name = "reducer-echo", blob = "reducer-echo" }],
  deliver = [
    {
      target = "reducer-echo",
      message = { contract = b"\x01ABCDEFGHIJKLMNOPQRSTUVWXYZ012345", payload = b"ECHOPAYLOAD" }
    }
  ],
  checker = "reducer-echo-check"
}
