# 23 — VFS / 块层 / 缓存（对标 Linux 2.4 fs/ + block/）

## 1. VFS 对象模型（fs/vfs/）

- 四个核心 trait（Rust trait 对象/static 注册，无继承）：
  - `SuperOps`：read_super/put_super/statfs/remount；
  - `InodeOps`：lookup/create/link/unlink/mkdir/rmdir/rename/readlink/symlink/permission；
  - `FileOps`：read/write/iterate(readdir)/llseek/ioctl/fsync/mmap；
  - `AddressSpaceOps`：readpage/writepage/prepare_write（M4 后期 page cache 接口）。
- inode：`{super, ino, mode, uid/gid(常量0), nlink, size, atime/mtime/ctime, op 指针,
  i_count, i_state(锁/dirty), i_mapping}`；icache 哈希+LRU。
- dentry：`{name, parent, inode(可空=负项), d_op, d_count, LRU}`；dcache 哈希（按路径分量），
  lookup 未命中建负项加速 ENOENT；rename/link 失效策略（简单 d_drop）。
- file 表：系统级 file（f_count/f_pos/f_mode/f_op/f_inode）+ 每任务 fd 表（位图扩展，
  默认 32 上限 1024，兼容 dup/dup2）。
- namei：path_walk（/、.、..、分量查找，符号链接跟随上限 8）；挂载表
  `mount_{dev,point,sb}` 查找穿越（M4 单层即可，bind 为 Out）。

## 2. 页缓存（fs/page_cache.rs）

- `address_space`：radix 树（位图数组实现的基数树，深度 2 层即可支撑 256MB 文件）挂 page；
  page：`{index, mapping, refcnt, dirty, LRU, private(bh 链)}`，物理页来自 buddy。
- 路径：read → 命中返回，未命中 readpage（经 buffer_head 读盘）；write → prepare/commit
  （M4 简化为立即拷贝+标脏，延迟分配为 ext4 阶段事项）；mmap 文件页共享同一缓存。
- 脏页回写：bdflush（脏超 60% 唤醒）+ kupdated（30s）遍历 super 的脏 inode 集合；
  fsync → 单文件强制回写（M6 接入 ext4 事务）。
- LRU/回收接 22-memory.md 的 vmscan。

## 3. buffer cache（fs/buffer.rs）

- buffer_head：`{block, size(=sb 块大小), dev, data(页内偏移或独立 kmalloc), state(DIRTY/LOCKED/UPTODATE),
  hash 链, lru}`；目标规模动态（≥64，无上限但受回收约束——对齐 2.4 思想而非 xv6 NBUF=10）。
- getblk/bread/bwrite/brelse/bforget 语义按 2.4；块大小≠页时页内多 bh 挂 page。
- ext4 元数据 IO 全走 bh（jbd2-lite 的操作对象即 bh），数据 IO 走 page cache。

## 4. 块设备层（block/ + drivers/block/）

- `BlockDeviceOps` trait：{make_request 或 直接同步 execute, geometry}；
  注册表 gendisk 等价：`{major, first_minor, minors, devname, ops, capacity, bs}`。
- 请求队列（block/request.rs）：`request{sector, count(sector), buffer|bio, cmd, complete_cb,
  elevator 私有}`；ll_rw_blk：合并（同扇区相邻/同请求扩展）+ 排序（单链表插入按扇区，
  等价 2.4 默认电梯）；队列深度阈值与 plug/unplug（简化：plug 即立刻派发也可，
  保留插拔接口以贴近 2.4）。
- 完成路径：IDE IRQ → 完成回调 → tasklet → waitqueue 唤醒。
- M1 过渡：ide 同步轮询直通（xv6 语义）；M4 切换到异步队列路径后 xv6fs 与 ext4 共用。

## 5. procfs / devfs（fs/procfs, fs/devfs）

- procfs：SuperOps 只读生成 inode；文本页生成器（seq_file 简化：fill(buf)）；
  文件集：version/cpuinfo/meminfo/uptime/buddyinfo/slabinfo/mounts/cmdline、
  pid/{status,stat,cmdline,environ(s)}、sysvipc/{sem,shm,msg}（M8）。
- devfs-like：挂 /dev，节点由驱动注册表生成（console/ttyS0/null/zero/hd0/hd1...），
  cdev/bdev 注册时自动建 dentry；`mknod` 系统调用仅允许在 /dev 下（宽松校验，root 单级）。

## 6. 与 ext4/xv6fs 的接线

- M4 交付时 xv6fs 先搬到 VFS（保 usertests 绿）→ ext4 以第二个客户端进入（hd1）→
  M6 切根文件系统为 ext4（user/ 程序进 ext4 镜像，构建管线换 mke2fs 打包）。

## 7. 验收（摘）

1. dcache 命中率：重复 stat 同路径 10k 次仅首次下发 lookup。
2. page cache：读同文件两遍，第二遍 0 次 IO（计数器）。
3. 电梯：随机 8 扇区请求注入，派发序列单调（host 单测）。
4. 缓冲回收：压力 IO 后 bh 总数收敛在阈值、无泄漏。
5. 两 FS 并存挂载 + 跨 FS cp 大文件（hd1→hd0）。
