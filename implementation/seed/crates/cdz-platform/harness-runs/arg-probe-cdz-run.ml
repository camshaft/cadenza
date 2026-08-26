{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "arg-probe-cdz", program = "arg-probe-cdz" },
    { name = "arg-probe-check-cdz", program = "arg-probe-check-cdz" }
  ],
  spawns = [{ name = "arg-probe-cdz", blob = "arg-probe-cdz" }],
  deliver = [
    { target = "arg-probe-cdz", message = { contract = "cdz-platform.deliver", payload = b"probe" } }
  ],
  checker = "arg-probe-check-cdz"
}
