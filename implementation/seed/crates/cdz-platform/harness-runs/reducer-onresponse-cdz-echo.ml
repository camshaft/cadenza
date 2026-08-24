{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-onresponse-cdz", program = "reducer-onresponse-cdz" },
    { name = "reducer-onresponse-check-cdz", program = "reducer-onresponse-check-cdz" }
  ],
  spawns = [{ name = "reducer-onresponse-cdz", blob = "reducer-onresponse-cdz" }],
  deliver = [
    {
      target = "reducer-onresponse-cdz",
      response = {
        contract = "cdz-platform.deliver",
        token = b"CORRELATE",
        answer = Ok(b"ANSWERPAYLOAD")
      }
    }
  ],
  checker = "reducer-onresponse-check-cdz"
}
