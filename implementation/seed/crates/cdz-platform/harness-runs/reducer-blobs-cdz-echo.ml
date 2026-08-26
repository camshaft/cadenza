{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-blobs-cdz", program = "reducer-blobs-cdz" },
    { name = "reducer-blobs-check-cdz", program = "reducer-blobs-check-cdz" }
  ],
  spawns = [{ name = "reducer-blobs-cdz", blob = "reducer-blobs-cdz" }],
  deliver = [
    {
      target = "reducer-blobs-cdz",
      message = { contract = "cdz-platform.deliver", payload = b"BLOBVALUE" }
    }
  ],
  checker = "reducer-blobs-check-cdz"
}
