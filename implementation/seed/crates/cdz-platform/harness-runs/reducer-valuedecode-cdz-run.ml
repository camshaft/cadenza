{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-valuedecode-cdz", program = "reducer-valuedecode-cdz" },
    { name = "reducer-valuedecode-check-cdz", program = "reducer-valuedecode-check-cdz" }
  ],
  spawns = [{ name = "reducer-valuedecode-cdz", blob = "reducer-valuedecode-cdz" }],
  deliver = [
    { target = "reducer-valuedecode-cdz", message = { contract = "cdz-platform.effect", payload = Value("Effect", b"DEEP") } }
  ],
  checker = "reducer-valuedecode-check-cdz"
}
