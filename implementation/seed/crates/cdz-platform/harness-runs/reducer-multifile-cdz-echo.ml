{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-multifile-cdz", program = "reducer-multifile-cdz" }
  ],
  spawns = [{ name = "reducer-multifile-cdz", blob = "reducer-multifile-cdz" }],
  deliver = [
    {
      target = "reducer-multifile-cdz",
      message = { contract = "cdz-platform.deliver", payload = b"ECHOPAYLOAD" }
    }
  ]
}
