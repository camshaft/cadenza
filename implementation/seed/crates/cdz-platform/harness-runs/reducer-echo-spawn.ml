{
  system = "$system",
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-echo", program = "reducer-echo" }
  ],
  spawns = [{ name = "reducer-echo", blob = "reducer-echo" }]
}
