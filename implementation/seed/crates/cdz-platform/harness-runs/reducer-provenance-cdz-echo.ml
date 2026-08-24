{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-provenance-cdz", program = "reducer-provenance-cdz" }
  ],
  spawns = [{ name = "reducer-provenance-cdz", blob = "reducer-provenance-cdz", kind = "event" }],
  deliver = [
    {
      target = "reducer-provenance-cdz",
      message = { contract = "cdz-platform.deliver", payload = b"ECHOPAYLOAD" }
    }
  ]
}
