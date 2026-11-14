# 25 — 驱动模型 / tty / 网络（对标 Linux 2.4 drivers/ + net/）

## 1. 设备模型（drivers/ + fs/devfs 接线）

- cdev 注册表：`{major, ops: CharDeviceOps{open,read,write,ioctl,poll,release}}`；
  bdev 见 23-vfs-block.md（gendisk 等）。
- 主次设备号静态分配表（Documentation式列表固化在 include 设备头文件）：
  1 mem(null/zero) / 4 console(tty0) / 4:64 ttyS0 / 3 hd0-hd1 第一IDE。
- open 路径：VFS 发现特殊 inode → 查注册表 → file.f_op 替换为设备 ops（2.4 语义简化，
  无 fops->open 链式继承）。
- ioctl 最小集：TIOCGWINSZ(常量)、HDIO_GETGEO（fdisk 类工具需要）、TCGETS/TCSETS 空实现。
- 电源/热插拔/总线模型：Out（无真实硬件面）。模块动态加载：Out（ADR 见 50）。

## 2. 字符设备（drivers/char/）

- console（VGA+串口复合）：printk 环形日志缓冲（dmesg/syslog 系统调用最小集），
  /dev/console 双写；VGA 滚动/游标（M0 已有）+ 转义序列最小集（\n \b \t 清屏）。
- kbd：PS/2 scancode→ascii（xv6 kbd.c 翻译）+ shift/ctl，中断驱动，输入环形缓冲；
  tty 行规程极简（回显、退格、^C=SIGINT 发前台组、^D=EOF），无 termios 完整字。
- serial（ttyS0）：16550 中断收发（M0 轮询版升级），输出走 printk 同步锁。

## 3. 块设备（drivers/block/ide.rs）

- ATA-PIO：IDENTIFY/READ/WRITE（CHS→LBA28），primary 通道两盘；
  IRQ14 完成中断 + 请求队列消费；多扇区单命令（最大 128 扇区/命令）。
- Stretch：bus mastering DMA（PCI IDE，qemu -device ide-hd 支持），错误重试路径完善。

## 4. 时钟与杂项

- PIT（通道0 100Hz，主时钟）+ RTC（读取实时时间挂 /proc/dmesg 时间戳，Stretch：
  CMOS 解析 date）；watchdog Out。
- null/zero 设备；full/random 为 Stretch（xorshift，登记到 mem major）。

## 5. 网络（net/，整体 Stretch，M8 后评估）

分层目标（对标 2.4 net/ 但只保留单网卡路径）：
- socket 层：BSD socket API 系统调用（socket/bind/listen/accept/connect/send/recv/
  shutdown），AF_INET + AF_UNIX（后者优先级高于 TCP，进程间调试有用）。
- proto 栈：IP（分片不做、重组收）、ICMP echo、UDP（完整）、TCP（最小状态机：
  ESTABLISHED 收发、慢启动关闭、RST 处理；无拥塞控制调优——记录为与 2.4 差距）。
- 驱动：qemu e1000（Stretch）或 virtio-net（依赖 PCI 扫描，Stretch）；loopback 必做。
- netfilter/iptables：Out。socket 选项：SO_REUSEADDR/SO_RCVTIMEO 最小集。

## 6. 验收（摘）

1. shell 交互：键入回显、^C 杀前台、^D 退出 cat。
2. /dev/null /dev/zero cat/dd 用例；ioctl HDIO_GETGEO 返回 hd0 几何。
3. printk 缓冲 dmesg 完整回放启动日志。
4. IDE：连续 128 扇区读写校验（M4 异步路径下无重入崩溃）。
5. （Stretch）loopback 上 UDP echo 自测程序；AF_UNIX 双进程 echo。
