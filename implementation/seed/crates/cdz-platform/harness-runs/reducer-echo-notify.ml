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
      notification = { contract = "cdz-platform.lifecycle", payload = b"NOTIFYPAYLOAD" }
    },
    {
      target = "reducer-echo",
      message = { contract = "cdz-platform.deliver", payload = b"ECHOPAYLOAD" }
    }
  ],
  checker = "reducer-echo-check"
}
