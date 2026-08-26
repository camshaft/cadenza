{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-statedispatch-cdz", program = "reducer-statedispatch-cdz" },
    { name = "reducer-statedispatch-decodefail-check-cdz", program = "reducer-statedispatch-decodefail-check-cdz" }
  ],
  spawns = [{ name = "reducer-statedispatch-cdz", blob = "reducer-statedispatch-cdz" }],
  deliver = [
    { target = "reducer-statedispatch-cdz", message = { contract = "cdz-platform.state.set", payload = b"not-a-valid-setrequest-encoding" } }
  ],
  checker = "reducer-statedispatch-decodefail-check-cdz"
}
