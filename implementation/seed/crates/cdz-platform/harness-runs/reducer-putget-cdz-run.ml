{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-putget-cdz", program = "reducer-putget-cdz" },
    { name = "reducer-putget-check-cdz", program = "reducer-putget-check-cdz" }
  ],
  spawns = [{ name = "reducer-putget-cdz", blob = "reducer-putget-cdz" }],
  deliver = [{ target = "reducer-putget-cdz", message = { contract = "cdz-platform.deliver", payload = b"PUTGETVALUE", token = b"pgkey" } }],
  checker = "reducer-putget-check-cdz"
}
