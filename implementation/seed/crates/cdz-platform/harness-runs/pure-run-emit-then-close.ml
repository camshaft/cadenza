{
  system = "$system",
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-emit-then-close-cdz", program = "reducer-emit-then-close-cdz" }
  ],
  spawns = [],
  pure-run = {
    program = "reducer-emit-then-close-cdz",
    contract = "cdz-platform.deliver",
    input = b"PUREINPUT",
    expect-output = b"PUREINPUT"
  }
}
