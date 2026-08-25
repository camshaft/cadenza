{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-stateid-cdz", program = "reducer-stateid-cdz" },
    { name = "reducer-stateid-check-cdz", program = "reducer-stateid-check-cdz" }
  ],
  spawns = [{ name = "reducer-stateid-cdz", blob = "reducer-stateid-cdz" }],
  deliver = [
    { target = "reducer-stateid-cdz", message = { contract = "cdz-platform.deliver", payload = b"SID", token = b"idkey" } }
  ],
  checker = "reducer-stateid-check-cdz"
}
