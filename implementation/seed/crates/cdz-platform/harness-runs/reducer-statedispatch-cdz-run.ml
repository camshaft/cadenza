{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-statedispatch-cdz", program = "reducer-statedispatch-cdz" },
    { name = "reducer-statedispatch-check-cdz", program = "reducer-statedispatch-check-cdz" }
  ],
  spawns = [{ name = "reducer-statedispatch-cdz", blob = "reducer-statedispatch-cdz" }],
  deliver = [
    { target = "reducer-statedispatch-cdz", message = { contract = "cdz-platform.state.set", payload = Value("SetRequest", { key = b"K", value = b"V" }) } },
    { target = "reducer-statedispatch-cdz", message = { contract = "cdz-platform.state.get", payload = Value("GetRequest", b"K") } },
    { target = "reducer-statedispatch-cdz", message = { contract = "cdz-platform.state.delete", payload = Value("DeleteRequest", b"K") } }
  ],
  checker = "reducer-statedispatch-check-cdz"
}
