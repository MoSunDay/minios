# 23 — VFS / 页缓存 / BIO 块层（对标 Linux 2.6.39.4 fs/ + block/）

## 1. VFS 对象模型

- `SuperOps`：read_super/put_super/statfs/remount/sync；
  `InodeOps`：lookup/create/link/unlink/mkdir/rmdir/rename/readlink/symlink/permission；
  `FileOps`：read/write/iterate/llseek/ioctl/fsync/mmap/poll；
  `AddressSpaceOps`：readpage/writepage/write_begin/write_end 等按阶段落地。
- inode/dentry/file 与 icache/dcache 引用计数、负 dentry、rename 失效和挂载穿越；
  M4 先用锁保护，Parity 再做 RCU path walk 与完整 mount namespace 协作。
- 每任务 fd 表支持 dup/dup2/fcntl/close-on-exec；namei 校验 symlink 深度、
  跨挂载与 64 位用户缓冲区。
- M4 保持单用户的最小凭据表示，落实 DAC mode 与基础 caps 检查；完整
  credentials/capabilities 算法及 LSM 在 Parity M12，不借 VFS 权限路径提前扩张 Core。
- **M4 Core FileOps 固定为上述最小集**。M10 Parity 另接原生 Linux AIO
  （io_setup/io_destroy/io_submit/io_getevents/io_cancel）、sendfile/splice/tee/
  vmsplice，以及 inotify/fanotify/dnotify（F_NOTIFY）；不得为此回头扩张 M4 FileOps。

## 2. page cache 与回写（M4）

- address_space 使用基数树/xarray 风格索引（按文件规模裁剪实现），page 记录
  index/mapping/refcount/dirty/LRU/private。
- M4 同时接 file-backed mmap：read/readpage/readahead、write_begin/write_end 与
  文件映射页共享同一缓存；buffer_head 主要服务元数据与块小于页的场景。
- active/inactive LRU 回收干净文件页；脏页 writeback 成功并转为干净后才释放，
  持久化顺序由 flush 保证。回写采用 Linux 2.6.39.4 writeback 思路：dirty 阈值、
  周期扫描、per-super 队列与 fsync；Core 可先单回写线程，Parity 补完整 per-BDI/
  带宽与公平性。
- 回收/OOM gate 接 22 §5；所有脏页错误必须传播，禁止静默丢弃。

## 3. BIO 与请求队列（M4）

- BIO 描述 page/offset/len 向量，request 聚合相邻 BIO；IDE ISR/tasklet/workqueue
  完成回调并唤醒等待者。M1R 的同步轮询 IDE 只保留为早期启动/诊断路径。
- 队列支持合并、plug/unplug、并发完成与错误重试。M4 接 IDE **CACHE FLUSH**
  命令、完成/超时/错误传播与块层 flush 请求；普通写完成不等于 flush 完成。
- M6 ext4 fsync 必须等待相关 BIO 与真实 CACHE FLUSH；M7 ordered/jbd2 提交、
  replay/checkpoint 复用同一持久化顺序。完整 barrier/FUA 语义与统计留 Parity M10，
  但不得以“完整 barrier 后置”为由省略 M6/M7 的基础 flush。
- 电梯：先 noop/deadline 验证，再实现 CFQ 基础（队列时间片/公平性）；
  Parity 对照 Linux 2.6.39.4 配置关闭完整 elevator 差距。

## 4. proc/sys/tmp/dev 设备文件系统

- procfs：version/cpuinfo/meminfo/uptime/buddyinfo/slabinfo/mounts、pid/*、sysvipc。
- M4 建 kobject/sysfs 基础与设备注册关系；/dev 最终采用 devtmpfs 语义。
  完整 uevent/driver/bus/class 与模块协作在 M11。
- tmpfs/ramfs 作为 shm、initramfs 与容器语义依赖。**M4 最小 tmpfs gate** 包括
  create/read/write/mmap/munmap、链接/删除后的页生命周期及回收压力；无 swap 时
  有数据页不可回收，只允许回收可安全重建或引用归零的页，禁止以回收为名丢数据。

## 5. 与 ext4 的接线

M4 先让 xv6fs 与最小 tmpfs 经 VFS 回归，并接通 file-backed mmap、基础权限与
IDE CACHE FLUSH；M5 ext4 作为独立第二盘只读挂载。M6 保持 xv6 root 不变，
仅对独立 ext4 第二盘的 `^has_journal` 副本做无日志 RW，fsync 使用真实 flush；M7 仍保持
xv6 root，第二盘所有元数据修改经 jbd2 handle，ordered 数据、日志提交与 checkpoint 依赖
同一 flush 顺序。详见 [30-ext4.md](30-ext4.md)。

## 6. 验收

1. M4：dcache/page cache 命中与 IO 计数；负 dentry、rename、挂载穿越；tmpfs
   基本读写/mmap/回收；file-backed mmap；单用户 mode/caps 允许与拒绝用例。
2. M4/M6：BIO 合并/拆分、电梯公平性、并发完成、CACHE FLUSH 错误/顺序注入；
   fsync 只有在设备确认 flush 后才成功。
3. 两 FS 并存与跨 FS cp；proc/sys/devtmpfs 快照。
4. **M8 SMP 回归**：跨 CPU 并发执行 VFS 路径、page cache 缺页/回收、BIO 完成、
   ext4 RW 与 jbd2 事务/回写 IO；guest sync 并干净卸载后离线运行 e2fsck，
   严格退出 0，同时校验内容与操作计数，不能只做结构检查。
5. M10 Parity：原生 AIO、sendfile/splice/tee/vmsplice、inotify/fanotify/dnotify
   （F_NOTIFY）及 LTP fs/io、writeback 压力；这些 API 不改变 M4 Core FileOps 范围。
