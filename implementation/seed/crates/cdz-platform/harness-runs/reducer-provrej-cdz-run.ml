{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-provrej-cdz", program = "reducer-provrej-cdz" },
    { name = "reducer-provrej-check-cdz", program = "reducer-provrej-check-cdz" }
  ],
  spawns = [{ name = "reducer-provrej-cdz", blob = "reducer-provrej-cdz", kind = "event" }],
  deliver = [{ target = "reducer-provrej-cdz", message = { contract = "cdz-platform.deliver", payload = b"PROVREJ", token = b"short" } }],
  checker = "reducer-provrej-check-cdz"
}
