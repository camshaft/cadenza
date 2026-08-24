{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-dispatch-cdz", program = "reducer-dispatch-cdz" },
    { name = "reducer-dispatch-check-cdz", program = "reducer-dispatch-check-cdz" }
  ],
  spawns = [{ name = "reducer-dispatch-cdz", blob = "reducer-dispatch-cdz", kind = "event" }],
  deliver = [
    {
      target = "reducer-dispatch-cdz",
      message = { contract = "cdz-platform.deliver", payload = b"DISPATCHNOTIFY" }
    }
  ],
  checker = "reducer-dispatch-check-cdz"
}
