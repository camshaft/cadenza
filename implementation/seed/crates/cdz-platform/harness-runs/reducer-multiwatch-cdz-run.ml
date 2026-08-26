{
  registry = { default = "$system" },
  blobs = [
    { name = "$system", bytes = b"itest:no-system-reducer" },
    { name = "reducer-childexit-cdz", program = "reducer-childexit-cdz" },
    { name = "reducer-watcher-cdz", program = "reducer-watcher-cdz" },
    { name = "reducer-multiwatch-check-cdz", program = "reducer-multiwatch-check-cdz" }
  ],
  spawns = [
    { name = "watcher", blob = "reducer-watcher-cdz" },
    { name = "child-a", blob = "reducer-childexit-cdz", parent = "watcher", links = { parentWatchesChild = 1 } },
    { name = "child-b", blob = "reducer-childexit-cdz", parent = "watcher", links = { parentWatchesChild = 1 } }
  ],
  deliver = [
    { target = "child-a", message = { contract = "cdz-platform.effect", payload = b"EXIT" } },
    { target = "child-b", message = { contract = "cdz-platform.effect", payload = b"EXIT" } }
  ],
  checker = "reducer-multiwatch-check-cdz"
}
