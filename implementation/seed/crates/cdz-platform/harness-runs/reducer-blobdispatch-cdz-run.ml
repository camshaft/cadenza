{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-blobdispatch-cdz", program = "reducer-blobdispatch-cdz" },
    { name = "reducer-blobdispatch-check-cdz", program = "reducer-blobdispatch-check-cdz" }
  ],
  spawns = [{ name = "reducer-blobdispatch-cdz", blob = "reducer-blobdispatch-cdz" }],
  deliver = [
    { target = "reducer-blobdispatch-cdz", message = { contract = "cdz-platform.blob.put", payload = Value("PutRequest", b"DEDUP") } },
    { target = "reducer-blobdispatch-cdz", message = { contract = "cdz-platform.blob.put", payload = Value("PutRequest", b"DEDUP") } },
    { target = "reducer-blobdispatch-cdz", message = { contract = "cdz-platform.blob.get", payload = Value("GetRequest", b"nonexistent-hash") } }
  ],
  checker = "reducer-blobdispatch-check-cdz"
}
