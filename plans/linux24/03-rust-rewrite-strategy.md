# 03 — C(xv6) → Rust 重写策略

## 1. 总原则："语义基准 → 机制升级"

- M1 阶段做**忠实翻译**：xv6 是可运行的语义基准（参考实现为 MoSunDay/xv6 的本地只读克隆）。
  每个子系统先达到"行为等价"（相同的系统调用、相同的用户程序可跑），
  再在 M2+ 替换内部机制（buddy/slab/VFS/...）。禁止一步到位"理想化重写"。
- 对照验证：xv6 用户程序（usertests 等）在 minios 上跑通 = 等价性证据。

## 2. 翻译次序（与 M1 内部批次一致）

| 批次 | xv6 源 | 依赖 | 验证 |
|------|--------|------|------|
| B1 | printf.c string.c → lib/{printk,string}.rs | M0 骨架 | 输出一致性 |
| B2 | mmu.h vm.c kalloc.c → arch/x86/mm + mm/frame | B1 | 页表单测 + qemu 映射自检 |
| B3 | picirq timer traps(0x–31) → arch/x86/kernel | B2 | 时钟中断计数打印 |
| B4 | spinlock proc.c swtch.S → lib/sync + sched 基础 | B3 | 双进程 Ping-Pong |
| B5 | syscall.c sysproc.c exec.c → usercall + exec | B4 | xv6 echo/forktest 可执行 |
| B6 | console kbd uart（中断驱动化） | B3 | sh 交互输入 |
| B7 | ide.c + fs.c log.c bio.c file.c pipe.c → xv6fs 过渡栈 | B2,B5 | usertests 全绿 |
| B8 | xv6 C 用户程序移植 user/（gcc -m32, ELF 可加载） | B5,B7 | usertests/zombie/stressfs |

## 3. 翻译规约

1. **数据结构先行**：每个 C struct 先落成 `#[repr(C)]`/`repr(transparent)` 的 Rust 类型与模块，
   再翻译函数；函数签名保持一一对应（如 `allocuvm→alloc_user_vm`），文件头注释标注来源
   （`//! ported from xv6:vm.c::allocuvm`）。
2. **类型强化**：C 的裸 int 状态翻译为 newtype/enum（pstate→ProcState），位标志用
   `bitflags` 语义的常量模块（零依赖，手写）。
3. **所有权映射**：引用计数资源（buf、inode、file）→ 手写 refcount + 明确 drop 点
   （brelse/iput/close），禁止引入 Rc；共享可变用 Spinlock<T>/RwLock<T> 包裹。
4. **unsafe 逐条搬运**：C 中每个指针操作在 Rust 中成为局部 unsafe 块并注明 SAFETY；
   不允许整函数 unsafe。
5. **每文件一个职责**，≤400 行；xv6 大文件（proc.c 500 行、fs.c 490 行）拆为多个模块
   （proc.c → sched/core.rs + fork.rs + exit.rs + signal.rs）。
6. 用户程序暂不翻译（保留 C，gcc -m32 构建，sysv ABI/变参调用约定不变），Rust 用户态为 Stretch。

## 4. xv6 C 用户程序复用方案

- `user/` 直接拷贝 xv6 用户程序源（cat echo forktest grep init kill ln ls mkdir rm sh
  stressfs usertests wc zombie stree(s) ulib user.h usys.S syscall.h...），Makefile 用
  host gcc -m32 -fno-pie -nostdlib 构建（开发环境已验证 -m32 可用）。
- ABI 约定：int $T_SYSCALL（向量 0x40），参数寄存器 x86 SysV（xv6 用户态经 usys.S 内联
  int 指令）；minios 侧 M1 完全兼容该入口与错误返回（eax=-errno 风格暂缓，沿用 xv6 0/-1）。
- usertests 通过即 M1 gate；M4 起新增 minios 专属测试程序（lseek/getdents/ipc/mmap）。

## 5. 风险与对策

| 风险 | 对策 |
|------|------|
| 翻译走样（语义漂移） | 逐函数注释溯源 + usertests 回归 + 对照打印关键路径 |
| Rust 借用检查与中断上下文摩擦 | 全局状态用 Spinlock/&'static mut 经 OnceLock 初始化；上下文保存代码用裸指针+SAFETY |
| 内联汇编差异（Intel/AT&T） | 统一 options(att_syntax)（M0 已定型） |
| 链接布局差异导致启动失败 | 保持 x86-minios.ld 单一来源；CI 每次 make run 冒烟 |
| C 用户程序与内核 ABI 漂移 | syscall 编号/结构冻结在 include 化的 user.h 副本中，双向引用同一文件 |
