{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-selfid-cdz", program = "reducer-selfid-cdz" },
    { name = "reducer-selfid-check-cdz", program = "reducer-selfid-check-cdz" }
  ],
  spawns = [{ name = "reducer-selfid-cdz", blob = "reducer-selfid-cdz" }],
  deliver = [
    { target = "reducer-selfid-cdz", message = { contract = b"\x01\x86\x0c\x7a\xcb\x43\xd2\x6c\xb9\x3a\x8c\xed\xd7\xd6\x97\xb2\x30\x08\x8a\x50\xd5\x0e\xb2\x37\x6c\xcf\xee\x60\xca\xba\x2c\x52\x3a", payload = b"probe" } }
  ],
  checker = "reducer-selfid-check-cdz"
}
