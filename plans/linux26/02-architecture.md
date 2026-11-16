# 02 — x86_64 目标架构与代码组织（参考 Linux 2.6）

## 1. 目录树（随里程碑到位）

```
minios/
├── Cargo.toml / rust-toolchain.toml / Makefile
├── abi/                            # Rust no_std crate：syscall/errno/flags/共享 repr(C) 布局
├── kernel/
│   ├── x86_64-minios.ld
│   └── src/
│       ├── main.rs                 # rust_main：早期初始化与启动次序
│       ├── init/                   # start_kernel/initramfs 边界
│       ├── arch/x86_64/
│       │   ├── boot/               # multiboot 入口、16/32→64 过渡、AP trampoline
│       │   ├── kernel/             # gdt/tss/idt/traps/irq/apic/time/smp/syscall
│       │   └── mm/                 # 四级页表、直映、TLB、ioremap
│       ├── kernel/
│       │   ├── sched/              # CFS、RT policy、runqueue、负载均衡
│       │   ├── fork/exit/signal/   # 生命周期、NPTL 基础、rt_sigframe
│       │   ├── kthread/workqueue/softirq/timer/hrtimer/rcu/
│       │   └── panic/printk/futex/
│       ├── mm/                     # bootmem/buddy/slub/vmalloc/memory/vmscan/percpu
│       ├── fs/
│       │   ├── vfs/                # super/inode/dentry/file/address_space
│       │   ├── ext4/ + ext4/journal/
│       │   ├── procfs/ sysfs/ tmpfs/ devtmpfs/
│       │   └── xv6fs/              # M1R 过渡，ext4 接管后移除
│       ├── block/                  # bio/request/elevator/{noop,deadline,cfq}
│       ├── drivers/{char,block,pci,net}/
│       ├── ipc/                    # SysV/POSIX IPC、pipe/fifo、futex 协作
│       ├── net/                    # Parity：socket/inet/netfilter/netlink
│       ├── security/               # Core：capabilities 基础；Parity：完整 credentials/LSM
│       ├── lib/                    # string/list/rbtree/bitmap/atomic/sync/elf64
│       └── usercall/               # syscall/sysret 分发与 x86_64 参数 ABI
├── user/                           # Rust no_std 用户 crate（无 C）
│   ├── src/bin/                    # init/sh/ls/cat/echo/forktest/usertests...
│   ├── src/rt.rs                   # _start、args、panic、exit
│   ├── src/sys.rs                  # 内联汇编 syscall 封装
│   ├── src/alloc.rs                # Rust global_allocator/heap
│   └── x86_64-minios-user.ld       # 用户 ELF64 链接布局
├── tools/                          # Rust host 工具：mkfs、镜像工具、测试 runner
├── host-tests/                     # Rust 纯逻辑与单元测试
├── tests/                          # Rust 集成/QEMU 用例与机读覆盖清单
├── scripts/                        # 仅启动、传参、timeout、日志收集等自动化
├── plans/
└── .github/workflows/ci.yml
```

引导允许 16/32 位过渡汇编，但内核、驱动、用户程序和公开 ABI 均为 64 位；
不保留 i686 target、ELF32 loader 或 ia32 compat 层。abi/kernel/user/tools/host-tests/tests
的实现及全部仓库内测试均用 Rust，仅既定硬件入口保留必要汇编；scripts 不参与测试
用例或 verdict 实现。用户 target、直接 rust-lld 链接与 runtime 详见
[04 — Rust 用户态、共享 ABI 与构建链](04-rust-userspace.md)。

## 2. xv6 → minios 模块映射

| xv6 来源 | minios 去处 | 阶段 |
|----------|-------------|------|
| main.c mp.c | init/ + arch/x86_64/kernel/smp | M1R/M8 |
| vm.c kalloc.c | arch/x86_64/mm + mm/{buddy,vmalloc} | M1R/M2 |
| proc.c swtch.S | kernel/sched + fork/exit | M1R/M3 |
| trap.c trapasm.S | arch/x86_64/kernel/traps | M1R |
| syscall.c sysfile.c sysproc.c | usercall/ | M1R |
| picirq/ioapic/lapic/timer | arch/x86_64/kernel/ | M1R/M8 |
| console/kbd/uart | drivers/char/ | M1R |
| ide.c | drivers/block/ide + block/ | M1R/M4 |
| bio.c | fs/buffer + block/bio | M1R/M4 |
| fs.c log.c | fs/xv6fs（过渡）→ fs/ext4 + journal | M1R/M5+ |
| file.c pipe.c | fs/vfs + ipc/pipe | M1R/M4 |
| exec.c elf.h | kernel/exec + lib/elf64 | M1R |

## 3. 关键数据流

```
syscall/sysret → usercall → VFS file ops → page_cache/address_space
                                      ├─► ext4 extent/journal(jbd2)
                                      └─► buffer_head/BIO → request/elevator
                                                           └─► IDE/virtio 驱动
CFS schedule → context switch(+CR3/FPU) → per-cpu runqueue
缺页 → VMA → anon zero/COW 或 page_cache → 四级 PTE/NX/TLB shootdown
```

## 4. Rust 与 x86_64 约束（硬性）

1. no_std、零第三方依赖；内核 target 固定 `x86_64-unknown-none`。
2. 禁止 red zone；内核禁用 SSE/MMX 自动生成（用户态 SSE2 正常支持）；
   code model/链接地址由 target 与链接脚本共同固定。
3. unsafe 白名单：端口与 MSR、boot/context/trap 汇编、页表与物理帧指针、
   MMIO/PIO、侵入式链表/RCU 底层。每处必须有 `// SAFETY:` 不变量。
4. 多态使用 trait/函数表；禁止 class 式继承与 Rc/RefCell 内核状态；
   并发原语集中在 lib/sync 与 kernel/rcu。
5. 地址类型区分 `VirtAddr/PhysAddr/UserPtr` newtype，禁止把用户指针当内核引用；
   canonical 地址、跨页、用户范围必须逐次校验。
6. 错误统一 `Result<T, Errno>`；unwrap/panic 仅限 init 与不可恢复断言。
7. 新增文件 ≤400 行，迭代中 ≤800 行；纯逻辑与硬件解耦以 host cargo test 验证。
8. abi/kernel/user/tools/host-tests/tests 不新增 `.c/.h` 或由 C 编译器生成的 minios
   构件；仓库内测试同样只用 Rust。汇编只允许上述硬件边界，CI 用 tracked-file 与
   构建命令检查执行 Rust-only 政策，scripts 仅作自动化。

## 5. 系统调用与 ABI

- M1R 起直接采用 **Linux x86_64 原生编号**，不再经历 xv6/i386 号段迁移。
  例：read=0、write=1、open=2、close=3、mmap=9、pipe=22、clone=56、fork=57、
  exit=60、wait4=61；只实现白名单子集，其余显式 ENOSYS。
- 参数使用 x86_64 syscall ABI：RAX 编号/返回值，RDI/RSI/RDX/R10/R8/R9 参数；
  RCX/R11 由硬件占用，内核入口保存完整现场并处理 swapgs/RFLAGS。
- Core 阶段包括：进程/文件/内存/信号/时间核心子集；M8 增加 clone/futex/epoll/IPC。
  capabilities 只承诺 Core 基础，完整 credentials/LSM 属 Parity，不提前扩张 Core。
  **M9 开始前**从 2.6.39.4 权威 `syscall_64.tbl` 生成全量差距表，逐项标记实现/
  语义偏差/不做，并同时冻结 Parity 验收配置；不得开始 M9 后再选择基线。
- 编号相同**不代表**结构体、flag、错误码、信号帧或边界语义兼容；ABI 验收以
  未修改 x86_64 用户态二进制与 LTP 为准。
- 用户态使用 Rust `no_std` runtime：自定义 `_start`、syscall 内联汇编、print/args、
  panic=abort 与 global_allocator；不链接 libc。后续可增加 Rust std 的系统接口层。
- Parity 阶段的 glibc/NPTL、LTP 与发行版 rootfs 仅是**外部黑盒 ABI 验证负载**，
  不属于 minios 用户态实现。vDSO/vsyscall 在 M9 纳入 ABI 清单。
