{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-statedispatch-cdz", program = "reducer-statedispatch-cdz" },
    { name = "reducer-statedispatch-miss-check-cdz", program = "reducer-statedispatch-miss-check-cdz" }
  ],
  spawns = [{ name = "reducer-statedispatch-cdz", blob = "reducer-statedispatch-cdz" }],
  deliver = [
    { target = "reducer-statedispatch-cdz", message = { contract = "cdz-platform.state.get", payload = Value("GetRequest", b"ABSENT") } }
  ],
  checker = "reducer-statedispatch-miss-check-cdz"
}
