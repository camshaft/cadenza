{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-responderr-cdz", program = "reducer-responderr-cdz" },
    { name = "reducer-responderr-check-cdz", program = "reducer-responderr-check-cdz" }
  ],
  spawns = [{ name = "reducer-responderr-cdz", blob = "reducer-responderr-cdz" }],
  deliver = [
    { target = "reducer-responderr-cdz", response = { contract = "cdz-platform.deliver", token = b"C-missing", answer = Err("missing-handler") } },
    { target = "reducer-responderr-cdz", response = { contract = "cdz-platform.deliver", token = b"C-schema", answer = Err("schema-violation") } },
    { target = "reducer-responderr-cdz", response = { contract = "cdz-platform.deliver", token = b"C-faulted", answer = Err("faulted") } }
  ],
  checker = "reducer-responderr-check-cdz"
}
