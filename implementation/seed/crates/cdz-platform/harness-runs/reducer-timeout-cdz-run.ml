{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-timeout-cdz", program = "reducer-timeout-cdz" },
    { name = "reducer-timeout-check-cdz", program = "reducer-timeout-check-cdz" }
  ],
  spawns = [{ name = "reducer-timeout-cdz", blob = "reducer-timeout-cdz" }],
  deliver = [
    {
      target = "reducer-timeout-cdz",
      response = {
        contract = "cdz-platform.deliver",
        token = b"CORRELATE",
        answer = Err("timeout")
      }
    }
  ],
  checker = "reducer-timeout-check-cdz"
}
