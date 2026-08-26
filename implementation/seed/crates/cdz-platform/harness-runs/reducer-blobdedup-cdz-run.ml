{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-blobdedup-cdz", program = "reducer-blobdedup-cdz" },
    { name = "reducer-blobdedup-check-cdz", program = "reducer-blobdedup-check-cdz" }
  ],
  spawns = [{ name = "reducer-blobdedup-cdz", blob = "reducer-blobdedup-cdz" }],
  deliver = [
    { target = "reducer-blobdedup-cdz", message = { contract = "cdz-platform.effect", payload = b"DEDUPME" } }
  ],
  checker = "reducer-blobdedup-check-cdz"
}
