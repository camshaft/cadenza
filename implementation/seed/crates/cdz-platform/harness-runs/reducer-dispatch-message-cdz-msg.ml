{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-dispatch-message-cdz", program = "reducer-dispatch-message-cdz" },
    { name = "reducer-dispatch-check-cdz", program = "reducer-dispatch-check-cdz" }
  ],
  spawns = [{ name = "reducer-dispatch-message-cdz", blob = "reducer-dispatch-message-cdz", kind = "event" }],
  deliver = [
    {
      target = "reducer-dispatch-message-cdz",
      message = { contract = "cdz-platform.deliver", payload = b"DISPATCHMESSAGE", token = b"tok-msg" }
    }
  ],
  checker = "reducer-dispatch-check-cdz"
}
