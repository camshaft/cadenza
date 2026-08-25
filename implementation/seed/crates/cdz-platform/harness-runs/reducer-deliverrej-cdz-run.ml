{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-deliverrej-cdz", program = "reducer-deliverrej-cdz" },
    { name = "reducer-deliverrej-check-cdz", program = "reducer-deliverrej-check-cdz" }
  ],
  spawns = [{ name = "reducer-deliverrej-cdz", blob = "reducer-deliverrej-cdz", kind = "event" }],
  deliver = [{ target = "reducer-deliverrej-cdz", message = { contract = "cdz-platform.deliver", payload = b"DELIVERREJ", token = b"short" } }],
  checker = "reducer-deliverrej-check-cdz"
}
