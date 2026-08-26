{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-watcher-cdz", program = "reducer-watcher-cdz" },
    { name = "reducer-childexit-cdz", program = "reducer-childexit-cdz" },
    { name = "reducer-watch-check-cdz", program = "reducer-watch-check-cdz" }
  ],
  spawns = [
    { name = "watcher", blob = "reducer-watcher-cdz" },
    { name = "child", blob = "reducer-childexit-cdz", parent = "watcher", links = { parentWatchesChild = 1 } }
  ],
  deliver = [
    { target = "child", message = { contract = "cdz-platform.effect", payload = b"EXIT" } }
  ],
  checker = "reducer-watch-check-cdz"
}
