{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-rejected-cdz", program = "reducer-rejected-cdz" },
    { name = "reducer-rejected-check-cdz", program = "reducer-rejected-check-cdz" }
  ],
  spawns = [{ name = "reducer-rejected-cdz", blob = "reducer-rejected-cdz", kind = "event" }],
  deliver = [
    {
      target = "reducer-rejected-cdz",
      message = { contract = "cdz-platform.deliver", payload = b"REJ", token = b"short" }
    }
  ],
  checker = "reducer-rejected-check-cdz"
}
