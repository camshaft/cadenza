{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-tworecordops-cdz", program = "reducer-tworecordops-cdz" },
    { name = "reducer-tworecordops-check-cdz", program = "reducer-tworecordops-check-cdz" }
  ],
  spawns = [{ name = "reducer-tworecordops-cdz", blob = "reducer-tworecordops-cdz", kind = "event" }],
  deliver = [
    {
      target = "reducer-tworecordops-cdz",
      message = { contract = "cdz-platform.deliver", payload = b"TWORECORDOPS" }
    }
  ],
  checker = "reducer-tworecordops-check-cdz"
}
