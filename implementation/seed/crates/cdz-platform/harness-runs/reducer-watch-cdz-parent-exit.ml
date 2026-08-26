{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-childexit-cdz", program = "reducer-childexit-cdz" },
    { name = "reducer-watcher-cdz", program = "reducer-watcher-cdz" },
    { name = "reducer-watch-check-cdz", program = "reducer-watch-check-cdz" }
  ],
  spawns = [
    { name = "parent", blob = "reducer-childexit-cdz" },
    { name = "child", blob = "reducer-watcher-cdz", parent = "parent", links = { childWatchesParent = 1 } }
  ],
  deliver = [
    { target = "parent", message = { contract = "cdz-platform.effect", payload = b"EXIT" } }
  ],
  checker = "reducer-watch-check-cdz"
}
