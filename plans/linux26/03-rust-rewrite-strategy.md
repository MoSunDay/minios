# 03 — xv6 → Rust → x86_64 迁移策略

## 1. 原则

- 原 i686 M0/M1 只作语义与测试证据，不作为新架构完成证据。
- 先建立 x86_64 可运行骨架，再平移可移植 Rust 逻辑；禁止在原 32 位页表/上下文
  上零散打补丁后直接宣称 64 位完成。
- M1R 保持 xv6 行为回归；M2+ 才替换为 Linux 2.6 机制。每次迁移保持可启动。

## 2. 迁移批次

| 批次 | 工作 | gate |
|------|------|------|
| R0 | target/linker/qemu 切换；32 位过渡入口→四级页表→长模式 | x86_64 banner/panic |
| R1 | 64 位 GDT/TSS/IDT、trap、PIC/PIT、直映与物理地址类型 | 中断/异常/NX 用例 |
| R2 | 64 位 context、CR3、FPU eager、syscall/sysret、per-cpu GS MSR | fork/上下文/SSE 用例 |
| R3 | ELF64 loader、UserPtr/copyin/out、x86_64 syscall 号段子集 | 最小 64 位 init/sh；fault fixup/EFAULT 用例 |
| R4 | IDE/xv6fs/pipe/file 等可移植模块接入 | 64 位到期 Rust usertests 全绿 |
| R5 | user/ 全部改写为 Rust no_std bins + runtime/ulib，移除 C 用户源 | Rust ABI/usertests 到期 required 用例 + check-rust-only |

R0–R5 对应 M0R/M1R，不另把旧 M1 的 gate 标绿。host 单测继续引用真实内核模块，
ELF 测试改 ELF64，并新增非 canonical/64 位溢出用例。每批只运行当阶段已到期的
required Rust 用例，未来项记 `not-active`，不得记作 skip-success；M1R 不提前要求
mmap/munmap/COW（M2）或 SIGPIPE（M3）。

## 3. 翻译规约

1. C struct 先落为明确布局的 Rust 类型，但磁盘格式使用字节解析，不 zeroed 强转。
2. `usize/u64` 与磁盘 `u32/u64` 分离；物理地址、虚拟地址、用户指针使用不同 newtype。
3. 内核汇编集中在 `arch/x86_64`；用户汇编仅限 `rt/sys` 的 `_start`、syscall 等
   必要硬件边界。AT&T 风格与既有代码一致，每处说明寄存器/内存 clobber。
4. FPU、syscall、trap、CR3、GS MSR 的 unsafe 不变量必须可审查。
5. 共享 syscall/errno/flags/结构布局由根 `abi/` no_std crate 单点提供，kernel/user
   不得复制第二份；`repr(C)` 只描述布局，不代表采用 C 实现。
6. 用户 runtime 提供 `_start`、syscall、堆、格式化输出与 panic；usertests 用 Rust
   逐项重建覆盖矩阵，不能靠编译旧 xv6 C 测试充数。
7. 工具、mkfs/镜像工具、host 单测和 QEMU 集成测试及其 verdict 均由 Rust 实现；
   shell/Python 只启动进程、传参、timeout、收集日志，不写测试用例或测试 verdict 逻辑。
8. UserPtr/copyin/copyout 采用 exception table 或等价可恢复 fault fixup，将访问故障
   转为 EFAULT；canonical/范围/跨页预检不能替代 fault fixup。
9. 纯逻辑 host 测试与 QEMU 集成分层；QEMU 必须使用 qemu-system-x86_64。
   用户构建、runtime 与覆盖矩阵细节见 [04-rust-userspace.md](04-rust-userspace.md)。

## 4. 风险与回退

| 风险 | 对策 |
|------|------|
| 长模式入口失败 | 分阶段串口标记；先静态直映，再增高半区/NX |
| sysret/swapgs 边界错误 | 特权级/RFLAGS/非 canonical 用例；保留 iretq 返回路径 |
| 32 位指针假设残留 | 全仓 grep + 64 位 ABI 测试 + 警告即错误 |
| FPU 串值 | M1R eager 保存恢复；惰性优化不得提前 |
| 为赶进度破坏 M1 语义 | 每批只跑到期 Rust usertests，失败即停在该批修复 |
