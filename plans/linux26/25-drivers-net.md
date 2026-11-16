# 25 — 驱动模型 / tty / PCI / 网络（对标 Linux 2.6.39.4）

## 1. 设备与驱动模型

- Core：cdev/bdev 注册、主次设备号、file ops 替换、sysfs/kobject 基础与
  devtmpfs 节点；驱动和设备的绑定关系不能只靠硬编码全局表。
- M11：bus/class/device/driver、uevent、模块参数、kmod/insmod/rmmod、
  initramfs 早期用户空间；模块重定位/符号/引用计数必须纳入安全边界。
- Core 只承诺 QEMU 参考设备；全主线驱动广度不属于当前 Parity 承诺。

## 2. 字符设备与 tty

- VGA+COM1 printk、环形 dmesg；PS/2 keyboard 与 16550 中断输入。
- **M4 最小行规程**：回显、退格、^D EOF、TCGETS 与 winsize 基础读写；
  先覆盖规范输入和 EOF 边界，不以 shell 能回显代替用例。
- **M8** 接进程组/前台组后才实现 ^C/^Z 信号；M4 不得伪造无前台组语义的
  job control。M11 补完整 termios、controlling tty/session、SIGTTIN/SIGTTOU、
  Unix98 pty、poll/ioctl 与多终端测试。

## 3. 块设备与总线

- IDE ATA-PIO 用于 M1R；M4 切 IRQ+BIO 队列并接 CACHE FLUSH；M6 fsync、M7
  ordered/jbd2 使用真实 flush；M11 增加 PCI 枚举与 bus-master DMA。
- M11 按验收配置补 ACPI、APIC 拓扑与必要的 virtio 块/控制设备；网卡只按 §5
  派生配置。USB/SCSI 等仅当 Parity 范围明确列入才实现，不默认承诺全驱动。

## 4. 时间与平台

- M1R/M3 UP 只用 PIT/PIC；TSC 可作 clocksource。M8 才接 LAPIC timer、IOAPIC
  与 IPI，并补 RTC TOD；Parity 补 HPET/NO_HZ/CPU 变频或电源项（按冻结配置）。
- null/zero/full；random 先提供明确非密码学伪随机，Parity 若运行真实用户态
  必须实现符合预期的熵池与 /dev/{random,urandom} 语义。

## 5. 网络（Stretch；Parity 必须关闭）

- 网络整体是 **Stretch，不进 M8 Core gate**：可选交付 socket 系统调用骨架、
  AF_UNIX、loopback、UDP echo 与最小 TCP；未完成不得影响 M8 判定，也不得写成
  Core 网络已完成。
- M12 Parity 必须关闭约定差距：sk_buff 风格包生命周期、IPv4/必要 IPv6、完整 TCP
  状态机/重传/拥塞控制、UDP/ICMP、路由与 ARP、netlink、netfilter/iptables。
- 测试配置遵循 [40-testing-ci.md](40-testing-ci.md)：**Core 默认 `-nic none`**。
  网络 Stretch/Parity 使用 40 §1.2 的显式派生配置，不改默认 Core job；配置 ID、QEMU
  型号、netdev 名称和完整参数均直接引用 40，本文件不复制或另设参考名称。派生证据
  仍须记录 host 对端与测试版本。
- 新增或变更网卡类型（包括 virtio-net）必须先修订并冻结 40 的派生配置，再执行测试；
  不得在本文件另建替代配置，也不得把不同网卡写成可互换的参考项。
- loopback、AF_UNIX、socket options、超时/非阻塞/poll 全部纳入派生 ABI 测试。
  NFS/其他高层协议是否属于目标由 M9 范围冻结决定，不默认宣称全网络 1:1。

## 6. 验收

1. M4：tty 回显、退格、EOF、TCGETS/winsize；M8：^C/^Z 前台组与 job control
   基础；M11：pty/完整 termios。各阶段另做 dmesg 完整回放。
2. M1R：独立 expected-fault job 注入受控 #PF/#GP，验证完整寄存器现场、故障 RIP 与
   预期符号化栈回溯，并静态确认 M0R/M1R 内核 force-frame-pointers 未失效。普通套件
   仍执行 panic=FAIL；该诊断结果不计入普通套件通过数，也不替代任何普通 required 用例。
3. M4：主次设备号注册与冲突拒绝、驱动/设备绑定和 sysfs driver↔device 链接逐项验收；
   字符 `/dev` open 必须按主次设备号路由到绑定 file ops，块设备 open/read/write 必须
   经 bdev 到达 request/BIO，不能绕过设备模型直连测试钩子。
4. IDE PIO/DMA、BIO 并发、CACHE FLUSH 与错误注入；PCI/ACPI 枚举快照；模块加载卸载。
5. 默认 Core 以 `-nic none` 跑全套回归；仅派生网络 job 跑 loopback/e1000 UDP/TCP
   互操作，与 host/参考 Linux 对测而非仅自发自收。网络结果不进入 M8 gate。
6. Parity：发行版网络工具、iptables/netlink、LTP 网络与冻结网卡压力矩阵。
