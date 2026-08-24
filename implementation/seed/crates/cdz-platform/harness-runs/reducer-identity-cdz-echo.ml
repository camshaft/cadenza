{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-identity-cdz", program = "reducer-identity-cdz" },
    { name = "reducer-identity-check-cdz", program = "reducer-identity-check-cdz" }
  ],
  spawns = [{ name = "reducer-identity-cdz", blob = "reducer-identity-cdz" }],
  deliver = [
    {
      target = "reducer-identity-cdz",
      message = { contract = "cdz-platform.deliver", payload = b"ECHOPAYLOAD" }
    }
  ],
  checker = "reducer-identity-check-cdz"
}
