{
  system = "$system",
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-echo-cdz", program = "reducer-echo-cdz" },
    { name = "reducer-check-cdz", program = "reducer-check-cdz" }
  ],
  spawns = [{ name = "reducer-echo-cdz", blob = "reducer-echo-cdz" }],
  deliver = [
    {
      target = "reducer-echo-cdz",
      notification = { contract = "cdz-platform.lifecycle", payload = b"NOTIFYPAYLOAD" }
    },
    {
      target = "reducer-echo-cdz",
      message = { contract = "cdz-platform.deliver", payload = b"ECHOPAYLOAD" }
    }
  ],
  checker = "reducer-check-cdz"
}
