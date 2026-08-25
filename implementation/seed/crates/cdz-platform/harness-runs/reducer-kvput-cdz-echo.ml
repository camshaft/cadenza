{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-kvput-cdz", program = "reducer-kvput-cdz" },
    { name = "reducer-kvput-check-cdz", program = "reducer-kvput-check-cdz" }
  ],
  spawns = [{ name = "reducer-kvput-cdz", blob = "reducer-kvput-cdz", kind = "ordinary" }],
  deliver = [
    {
      target = "reducer-kvput-cdz",
      message = { contract = "cdz-platform.deliver", payload = b"\x01\x02\x03" }
    }
  ],
  checker = "reducer-kvput-check-cdz"
}
