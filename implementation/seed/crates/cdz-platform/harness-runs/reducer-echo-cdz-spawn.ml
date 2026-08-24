{
  system = "$system",
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-echo-cdz", program = "reducer-echo-cdz" }
  ],
  spawns = [{ name = "reducer-echo-cdz", blob = "reducer-echo-cdz" }]
}
