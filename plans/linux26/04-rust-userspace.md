# 04 — Rust 用户态、共享 ABI 与构建链

> 本文件关闭“用户态用什么 Rust target、runtime 如何启动、旧 C 测试如何替代”的开放项。
> minios 实现代码不依赖 C 编译器；外部验证程序不计入 minios 实现树。

## 1. Workspace 与语言边界

```
abi/                       # no_std crate：syscall 号、errno、flags、共享 repr(C) 布局
kernel/                    # no_std x86_64 内核，依赖内部 abi crate
user/                      # no_std crate + 多个 Rust bins
  src/{rt,sys,alloc,io,args,lib}.rs
  src/bin/{init,sh,cat,echo,ls,...}.rs
  x86_64-minios-user.ld
tools/                     # Rust host bins：mkfs、镜像/测试清单生成
host-tests/ tests/         # Rust 单测与集成用例
```

- `repr(C)` 只表示公开 ABI 布局，不表示使用 C 语言；共享类型由 abi crate 单点提供，
  禁止 kernel/user 各自复制 syscall 编号和结构体。
- 内核与用户 runtime 零第三方依赖；host 工具如需依赖必须单独审查并锁定。
- 汇编只在 rt/sys 的 `_start` 与 syscall 指令等必要位置；不保留 `usys.S` 的 C ABI 版本。
- shell/Python 只启动进程、传参、timeout 和收集日志；不实现内核、用户程序、工具、
  测试用例或测试 verdict 逻辑，测试判定由 Rust runner 输出。

## 2. Target 与链接决策（已定型）

| 构件 | Rust target | 链接与运行约束 |
|------|-------------|----------------|
| kernel | `x86_64-unknown-none` | 高半区 kernel linker script、no red zone、禁 SSE/MMX |
| user（M1R） | `x86_64-unknown-linux-gnu`，**仅作代码生成 target** | no_std/no_main、panic=abort、rust-lld 直链、静态非 PIE `ET_EXEC` |
| host tools/tests | 宿主 Rust target | host 上运行，不进入 minios 镜像 |

选择 Linux GNU 作为 user 代码生成 target 是为了使用稳定的预编译 core 与 x86_64
SSE2 ABI；它不代表运行 Linux，也不允许调用宿主 libc/syscall。用户对象必须由
`rust-lld -flavor gnu`（ld.lld 语义）按独立 user linker script 直接链接，**不经过
cc/gcc driver**，不带 crt/crti/crtn、libc、libgcc，固定静态非 PIE `ET_EXEC`。

M1R gate 同时留存 readelf 文本、link map 与构建日志：readelf 验证 ELF64/ET_EXEC、
无 `PT_INTERP`/`DT_NEEDED`、入口为用户 `_start` 且无未解析宿主符号；map 验证仅有
批准的用户/runtime 对象且无 crt/libc/libgcc；日志验证实际链接器是 rust-lld 而非
cc/gcc。runtime 自行提供并审计 `memcpy/memmove/memset/memcmp` 等必要 memory
intrinsics，不得靠宿主导入库补符号。后续自定义 `x86_64-unknown-minios` target 须保持
同一 ABI 回归，不是 M1R 前置。

minios 自有 user bins 的构建与加载契约只支持 `ET_EXEC`；遇 `ET_DYN`、`PT_INTERP`、
`PT_DYNAMIC` 或动态依赖一律拒绝。外部 Parity 负载的规则见 §6，不套用此限制。

## 3. Rust 用户 runtime

- `_start`：按初始栈读取 `argc`、NULL 结尾的 `argv[]`/`envp[]`，再读取 key/value
  形式的 auxv 直到 `AT_NULL`；保存指针并建立 runtime 状态后调用 Rust `main`，以其
  返回值调用 exit。入口及调用栈对齐遵守 x86_64 ABI。
- `sys.rs`：RAX 编号，RDI/RSI/RDX/R10/R8/R9 参数，正确处理 RCX/R11 clobber；
  原始返回值按有符号数落在 **-4095..=-1** 时转为 `Err(Errno)`，其余保留 64 位
  位型并返回 `Ok`。内联汇编标注内存/寄存器副作用。
- `alloc.rs`：M1R 的 global_allocator 只基于 brk/sbrk，处理对齐、扩容与 OOM；
  初始可不回收。mmap/munmap 与 COW 到 M2 才激活，SIGPIPE 到 M3 才激活。
- `io.rs/args.rs`：基于 write 的格式化输出、stdin/stdout/stderr、参数迭代；
  不依赖 host std。panic 打印后异常退出，测试 runner 汇总失败数。
- 后续 Rust std 支持以独立系统接口层实现，不能通过在用户程序里链接宿主 std/libc
  获得“假兼容”。

### M1R 内核侧执行 ABI

1. 在启用 `EFER.SCE` 前，先建立匹配 syscall/sysret 选择子的 GDT/TSS，并初始化
   `IA32_STAR/LSTAR/FMASK`、各 CPU 的 GS base/`IA32_KERNEL_GS_BASE`；最后才开放
   syscall，避免半初始化入口。
2. syscall 从 CPL3 进入时硬件不切栈：入口先 `swapgs` 取得内核 per-cpu GS，把用户
   RSP 保存到 per-cpu scratch，再切换到**本任务内核栈**并保存完整现场；不得在用户栈上
   写内核帧。异常/中断入口按保存的 CS.CPL 条件执行 `swapgs`，用 IST 或等价入口约定
   覆盖切栈/GS 窗口，异常返回不得重复交换 GS。
3. 返回前净化现场并恢复用户 GS。`sysretq` 只用于已验证 canonical 的用户 RIP 且其余
   返回条件安全的路径；RIP 非 canonical 或任一条件不安全时走 `iretq`，禁止让
   `sysretq` 在内核态产生 #GP。
4. BSP/AP 在使用 SSE 前初始化 x87/SSE 控制状态：清 `CR0.TS/EM`、设置 `CR0.MP`，
   设置 `CR4.OSFXSR/OSXMMEXCPT`，初始化 FCW/MXCSR。M1R 上下文切换采用对齐的
   `fxsave64/fxrstor64` eager 保存恢复，禁止惰性 FPU 优化提前进入 gate。

## 4. M1R 用户程序与测试迁移

当前实现树中的 C 程序全部以 Rust 重写：cat、echo、forktest、grep、init、kill、ln、
ls、mkdir、rm、sh、stressfs、usertests、wc、zombie；printf/ulib/umalloc/usys 的能力
分别由 runtime 的 io/sys/alloc 取代，而不是继续编译 C 版本。

覆盖矩阵输入为机读 `tests/user/coverage.toml`，在 M1R 建立。每条静态字段严格采用
[40 — 测试与 CI](40-testing-ci.md) 的 schema：`case_id`、`source`、`milestone`、`config`、
`mode`（仅 `host`、`qemu`、`external`）、`runner_test_name`、`requirement`、`adr`；
runner 在派生运行/结果文件中补充 `activation` 与 `evidence`，不回写输入。本文件不另造
平行字段或 verdict 词表。原进程、文件、目录/路径、管道/描述符、内存、压力、架构分类
仍逐项展开，其中 mmap/munmap/COW 的 `milestone` 为 M2，SIGPIPE 的 `milestone` 为 M3。

下面是 **runner 展开后的派生结果** TOML 形状，不是 `coverage.toml` 输入，也不是当前
通过证据；`example-run` 和 `PASS` 仅示范完整字段。`activation` 由当前阶段与 `milestone`
计算，`evidence` 同时记录本轮证据 ID、派生结果路径和该用例 `result`：

```toml
[[cases]]
case_id = "xv6-usertests-forktest"
source = "xv6@2305a8c:usertests:forktest"
milestone = "M1R"
activation = "active"
config = ["CORE-UP"]
mode = "qemu"
runner_test_name = "user::xv6::forktest"
requirement = "required"
adr = ""
evidence = { id = "example-run/CORE-UP/xv6-usertests-forktest", path = "results/example-run/CORE-UP.json", result = "PASS" }
```

Rust runner 只读输入 `coverage.toml`，将实际展开项及上述结果写入本轮派生结果文件，
严禁把 `activation`、证据 ID/path 或 `result` 回写输入。runner 按 `milestone` 计算
`activation`：M1R 只验到期用例，未来项为
`not-active`，不得作为 skip 或通过。活动且 `requirement = "required"` 的集合为空、
任一到期 required 用例 skip/缺失结果或存在失败，gate 均为 fail；M8 时所有 Core 项
必须激活且为 required。仓库内脚本只传递 Rust runner 的退出状态并收集日志，不得以
“旧 C 已测”、空套件或覆盖率文本代替 Rust 用例结果。

## 5. 构建与镜像流水线

1. cargo 构建 abi/kernel/user/tools/host-tests/tests；kernel 与 user 使用各自链接脚本。
2. 镜像按阶段隔离，host mke2fs/debugfs 管线从 **M5 起**使用：

   | 阶段 | 本轮独立镜像与用途 |
   |------|--------------------|
   | M1R–M4 | Rust mkfs 生成 xv6 root，嵌入 Rust 用户 ELF |
   | M5 | 保持 xv6 root；host mke2fs/debugfs 生成第二盘 ext4 RO 模板 |
   | M6 | 保持 xv6 root；仅为独立 ext4 第二盘生成 `^has_journal` RW 副本，不复用 M5 已挂载镜像 |
   | M7 | 保持 xv6 root；仅为独立 ext4 第二盘生成 `has_journal` 崩溃/replay 副本，每轮从干净基线重来 |

   每轮使用独立镜像，禁止在上一轮已写镜像上续测。固定并记录 e2fsprogs 版本、完整
   feature 集、UUID、时间戳和文件/目录项排序；记录命令清单及各镜像 SHA-256。
3. qemu-system-x86_64 启动；QEMU 参数只在 40 维护，此处不复制。fs.img/kernel/target
   均为生成物，遵守无二进制入库政策。
4. CI 执行 `check-rust-only.sh` 启动 Rust checker：abi/kernel/user/tools/host-tests/tests
   无 C 源、无 cc/gcc/clang minios 构建命令，用户 ELF 通过 §2 检查；外部 LTP/rootfs
   在独立 job 和目录中管理。

## 6. 外部验证负载

glibc/NPTL、LTP、coreutils/bash 和发行版 rootfs 只用于 Parity 的黑盒兼容性验证。
它们可以包含第三方 C 实现，但不得复制进 minios 实现树、不得替代 Rust 用户程序、
不得作为 M1R 的交付物。外部源须固定来源、版本、hash 与包清单，下载/解包/镜像等
生成物放在独立 job 和隔离目录；测试报告区分“Rust 自身用例”与“外部 ABI 用例”。

§2 的 readelf/ET_EXEC 检查只针对 minios 自有 user ELF，不得套到外部动态 ELF。
M9 先使用静态外部 fixtures；M11 若纳入动态发行版 rootfs，须先实现外部 ELF 的
`ET_DYN`/`PT_INTERP` 映射及动态 loader 所需 auxv（至少包含 `AT_BASE`），再运行该
rootfs。解释器由外部 rootfs 提供，**不需要也不开发 minios 自有动态链接器**。
