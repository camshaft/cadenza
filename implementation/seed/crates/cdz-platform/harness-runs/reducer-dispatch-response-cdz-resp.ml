{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-dispatch-response-cdz", program = "reducer-dispatch-response-cdz" },
    { name = "reducer-dispatch-check-cdz", program = "reducer-dispatch-check-cdz" }
  ],
  spawns = [{ name = "reducer-dispatch-response-cdz", blob = "reducer-dispatch-response-cdz", kind = "event" }],
  deliver = [
    {
      target = "reducer-dispatch-response-cdz",
      message = { contract = "cdz-platform.deliver", payload = b"DISPATCHRESPONSE", token = b"tok-resp" }
    }
  ],
  checker = "reducer-dispatch-check-cdz"
}
