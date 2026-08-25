{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-emit-then-close-cdz", program = "reducer-emit-then-close-cdz" },
    { name = "reducer-run-cdz", program = "reducer-run-cdz" },
    { name = "reducer-run-check-cdz", program = "reducer-run-check-cdz" }
  ],
  spawns = [{ name = "reducer-run-cdz", blob = "reducer-run-cdz", kind = "ordinary" }],
  deliver = [
    {
      target = "reducer-run-cdz",
      message = { contract = "cdz-platform.run", payload = BlobHash("reducer-emit-then-close-cdz") }
    }
  ],
  checker = "reducer-run-check-cdz"
}
