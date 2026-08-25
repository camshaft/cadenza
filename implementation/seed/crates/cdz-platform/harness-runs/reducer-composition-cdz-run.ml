{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-composition-cdz", program = "reducer-composition-cdz" },
    { name = "reducer-composition-check-cdz", program = "reducer-composition-check-cdz" }
  ],
  spawns = [{ name = "reducer-composition-cdz", blob = "reducer-composition-cdz", kind = "event" }],
  deliver = [
    {
      target = "reducer-composition-cdz",
      message = { contract = "cdz-platform.deliver", payload = b"COMPOSEPAYLOAD", token = b"composetoken" }
    }
  ],
  checker = "reducer-composition-check-cdz"
}
