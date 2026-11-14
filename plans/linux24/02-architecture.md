# 02 — 目标架构与代码组织（参考 Linux）

## 1. 目录树（成熟态，随里程碑逐步到位）

```
minios/
├── Cargo.toml / rust-toolchain.toml / Makefile      # workspace 与顶层构建
├── kernel/                    # 内核 crate（no_std bin）
│   ├── x86-minios.ld
│   └── src/
│       ├── main.rs            # rust_main：早期初始化与子系统启动次序
│       ├── init/              # 对应 linux init/：start_kernel 流程编排
│       ├── arch/x86/          # 对应 arch/i386/
│       │   ├── boot/          # multiboot 头、入口 asm、（辅）xv6 式 bootasm/bootmain
│       │   ├── kernel/        # idt.rs gdt.rs traps.rs irq8259.rs ioapic.rs lapic.rs time.rs smp.rs
│       │   └── mm/            # pgtable.rs tlb.rs ioremap.rs
│       ├── kernel/            # 对应 linux kernel/
│       │   ├── sched/         # core.rs(runqueue) policy.rs(idle/rr) signal.rs fork.rs exit.c 等价
│       │   ├── kthread.rs softirq.rs timer.rs panic.rs printk.rs syscall.rs
│       ├── mm/                # 对应 linux mm/：bootmem.rs buddy.rs slab.rs vmalloc.rs
│       │   ├── page_alloc.rs memory.rs(mmap/demand paging) cow.rs page_io.rs swap.rs(s)
│       │   └── percpu.rs
│       ├── fs/                # 对应 linux fs/
│       │   ├── vfs/           # super.rs inode.rs dcache.rs namei.c 等价、file_table.rs
│       │   ├── page_cache.rs buffer.rs block_dev.rs
│       │   ├── ext4/          # super.rs inode.rs extents.rs namei.rs dir.rs alloc.rs
│       │   │   └── journal/   # jbd2-lite：handle.rs transaction.rs commit.rs replay.rs
│       │   ├── procfs/ devfs/
│       │   └── xv6fs/         # M1 过渡（移植 xv6 fs.c+log.c），M5 后移除
│       ├── block/             # 对应 linux block/：ll_rw.rs request.rs elevator.rs
│       ├── drivers/           # 对应 linux drivers/
│       │   ├── char/          # console.rs serial.rs kbd.rs
│       │   └── block/         # ide.rs（PIO；DMA stretch）
│       ├── ipc/               # sem.rs shm.rs msg.rs pipe.rs fifo.rs
│       ├── net/               # Stretch：socket.rs loopback.rs tcp_min.rs
│       ├── lib/               # string.rs list.rs(侵入式链) rbtree.rs(s) spinlock.rs原子封装
│       ├── multiboot.rs io.rs console/ panic.rs     # M0 已有
│       └── usercall/          # 系统调用分发层（sys_*.rs）
├── user/                      # M1 复用 xv6 C 用户程序（gcc -m32）+ ulib；后续 Rust libc(Stretch)
├── tools/                     # mkfs(xv6fs 镜像)、镜像生成、崩溃测试脚本
├── scripts/                   # run-qemu.sh、e2fsck 循环、check-no-binaries.sh
├── docs/ 或 plans/            # 本计划
└── .github/workflows/ci.yml
```

## 2. xv6 → minios 模块映射

| xv6 文件 | minios 去处 | 里程碑 |
|----------|-------------|--------|
| main.c mp.c | init/ + arch/x86/kernel/smp.rs | M1/M8 |
| vm.c kalloc.c | arch/x86/mm/ + mm/{buddy,vmalloc}.rs | M1/M2 |
| proc.c swtch.S | kernel/sched/ + fork.rs | M1/M3 |
| trap.c trapasm.S vectors.pl | arch/x86/kernel/traps.rs | M1 |
| syscall.c sysfile.c sysproc.c | usercall/ | M1 |
| picirq.c ioapic.c lapic.c timer.c | arch/x86/kernel/{irq8259,ioapic,lapic,time}.rs | M1/M8 |
| console.c kbd.c uart.c | drivers/char/ | M1 |
| ide.c memide.c | drivers/block/ide.rs + block/ | M1/M4 |
| bio.c | fs/buffer.rs | M4 |
| fs.c log.c fs.h | fs/xv6fs/（过渡）→ fs/ext4/ | M1/M5+ |
| file.c file.h | fs/vfs/file_table.rs | M1/M4 |
| exec.c elf.h | kernel/exec.rs | M1 |
| pipe.c | ipc/pipe.rs | M1 |
| printf.c string.c | lib/ + printk.rs | M1 |
| bootasm.S bootmain.c | arch/x86/boot/（辅路径） | M1(s) |
| mkfs.c | tools/（Rust 重写或守卫修复） | M1 |

## 3. 关键数据流（成熟态）

```
syscall read/write
   │
   ▼
fs/vfs (file ops) ──► page_cache(地址空间) ──► fs/ext4(extent 映射)
   │                        │                        │
   ▼                        ▼                        ▼
dcache/icache          buffer_head(块<页)      fs/ext4/journal(jbd2-lite)
                                │                        │
                                ▼                        ▼
                    block/ll_rw 请求队列 ──► elevator 合并排序 ──► drivers/block/ide
```

## 4. Rust 约束与 unsafe 政策（硬性）

1. **no_std、零第三方依赖**贯穿内核主线；用户态与工具链不受限。
2. unsafe 白名单（其余全部 safe）：`io.rs` 端口、`arch/x86/mm` 页表装配、`mm/` 物理帧→指针、
   `drivers/` MMIO/PIO、`context switch` 汇编、`lib/` 侵入式链表节点容器。每处必须 `// SAFETY:` 注明不变量。
3. 无 class/OOP 继承：子系统多态用 **trait + 函数指针表**（VFS ops、block_device_ops、
   file_operations 皆 trait 对象/static 表），状态全部显式传参或全局 `Sync` 封装，禁止 Rc/RefCell 入内核。
4. 错误处理：内核路径统一 `Result<T, Errno>`；`unwrap/expect/panic` 仅限 init 阶段与断言。
5. 文件行数：新增 ≤400；迭代中 ≤800；超限必须按职责拆分。
6. 并发原语集中在 `lib/sync`：Spinlock（含 irq save/restore 语义）、RwLock（排队位图实现）、
   Semaphore、waitqueue；其余模块不得手写原子技巧。
7. 可测性优先：磁盘格式解析（ext4/jbd2）、位图、电梯排序等**纯逻辑**与硬件解耦，
   以 `#[cfg(test)]` 在 host 上 `cargo test` 跑（内存盘字节数组仿真）。

## 5. 系统调用面（目标 ~55，M1=21 等价 xv6）

M1（xv6 等价）：fork exit wait pipe read kill exec fstat chdir dup getpid sbrk sleep
uptime open write mknod unlink link mkdir close（编号沿用 xv6 1–21）。
M3+ 新增：getppid setpgid(s) times uname rt_sigaction rt_sigprocmask rt_sigreturn kill(-pid)
mmap munmap mprotect(s) madvise(Out) futex(Out) …
M4+ 新增：mount umount2 statfs fstatfs lseek(getdents) chown(空) chmod(空) ioctl(最小)
mknod/devfs 节点 stat/lstat readlink symlink rename flock(空) fsync ftruncate select/poll(单)。
M8：ipc 系列 semget/semop/shmget/shmat/msgget/msgsnd/msgrecv socket(Stretch)。
> 完整表由 M4 的 `usercall/table.rs` 固化，编号与语义尽量贴近 Linux i386 以便后续 musl 移植。
