{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-blobdispatch-cdz", program = "reducer-blobdispatch-cdz" },
    { name = "reducer-blobdispatch-decodefail-check-cdz", program = "reducer-blobdispatch-decodefail-check-cdz" }
  ],
  spawns = [{ name = "reducer-blobdispatch-cdz", blob = "reducer-blobdispatch-cdz" }],
  deliver = [
    { target = "reducer-blobdispatch-cdz", message = { contract = "cdz-platform.blob.put", payload = b"not-a-valid-putrequest-encoding" } }
  ],
  checker = "reducer-blobdispatch-decodefail-check-cdz"
}
