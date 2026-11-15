# STATUS — 演进状态跟踪（活文档）

> 更新规则：每个里程碑/批次完成后更新本表；gate 未全绿不得进入下一里程碑。

## 总体进度

- 当前里程碑：**M1 ✅ done；下一里程碑 M2（内存管理现代化）**
- 总体自估：**25%**（M0/M1 完成，8 个里程碑口径）
- 最近更新：2026-11-15（M1 收尾：host 三元组自动探测、usertests 强制重建 fs.img、引号键密钥匹配）

## 里程碑状态

| 里程碑 | 内容 | 状态 | 证据（gate） |
|--------|------|------|--------------|
| M0 | Rust 引导骨架（multiboot + VGA/串口 + panic） | ✅ done | qemu 串口 banner；make build 零警告 |
| M1 | xv6 功能等价移植（中断/分页/进程/系统调用/IDE/文件读写） | ✅ done | `make run` 进入 sh；usertests `ALL TESTS PASSED`；`make test` 4/4；`make test-qemu` 判绿 |
| M2 | 内存管理现代化（buddy/slab/vmalloc/percpu） | ⬜ pending | |
| M3 | 调度与内核基础设施（O(n)/内核线程/软中断/信号/定时器） | ⬜ pending | |
| M4 | VFS + 块层（dcache/page cache/请求队列/procfs） | ⬜ pending | |
| M5 | ext4 只读 | ⬜ pending | |
| M6 | ext4 读写（无日志） | ⬜ pending | |
| M7 | jbd2-lite 日志 | ⬜ pending | |
| M8 | SMP/IPC/2.4 parity 收尾 | ⬜ pending | |

## M1 已落地（Rust，`kernel/src/`）

- 启动/异常：multiboot 低区跳板 → 高地址 C 入口；GDT/TSS/IDT；0–31 异常 + 32–47 IRQ 框架。
- 时钟/中断：PIT 100 Hz（相对 xv6-lapic 的 plan-approved deviation）、8259 PIC、tick/wakeup。
- 内存：xv6 kalloc 等价页分配器、内核页表、用户页表 copyuvm/allocuvm/deallocuvm、copyin/copyout。
- 进程：`Proc` 表、`swtch` 上下文切换、fork/exec/exit/wait/kill、sleep/wakeup、procdump。
- 系统调用：xv6 21 个全集（编号与 `syscall.h` 逐一对齐，编译期断言校验）。
- 设备/文件：console（键盘+COM1 中断）、IDE PIO、buffer cache、xv6fs inode/namei/log、
  file table、pipe；console 作为 devsw major 1。
- 用户态：xv6 C 用户程序全集（gcc -m32），init/sh/ls/cat/echo/forktest 等 16 个 UPROGS。

### M1 关键修复记录

1. `insl/outsl` 内联汇编：LLVM i686 保留 `%esi`，改为逐 dword `inl/outl`，修复启动 brelse panic。
2. `sys_unlink`：`iupdate+iput` → `iunlockput`，消除 busy inode iput。
3. 日志协议：统一为 xv6-public 单事务 `busy` 协议（begin_op/end_op/log_write），
   修复并发写目录块导致的 inode 类型损坏。
4. 警告清零：128 → 0（debug/release），未使用项按“头文件常量/待用适配器”逐条说明或删除。
5. 可移植代码抽出：`lib/bitmap.rs`（越界安全位操作）、`lib/elf.rs`（ELF32 头解析）；
   `exec` 改为读字节 + `parse_ehdr/parse_phdr`，去掉 zeroed 结构体强转。
6. host 单测用 `#[path]` 引入真实内核模块（非副本）；`cfg(test)` 关闭 `#[no_mangle]`
   避免与宿主 libc 冲突。`scripts/run-qemu.sh` 按串口日志判绿并自动退出。
7. M1 收尾：`make test` 改用 `rustc -vV` 自动探测宿主三元组（不再假设 x86_64）；
   `run-qemu.sh --usertests` 无条件重建 fs.img（`--no-build` 仅跳过内核构建）；
   `check-no-secrets.sh` 关键字赋值匹配支持带引号键（JSON/YAML）。

## Go-Live Gate（仓库级常绿清单）

| Gate | 状态 | 说明 |
|------|------|------|
| make build 零警告 | 🟢 | stable 1.97.1 / i686；debug+release 均 0 warning |
| make run 冒烟通过 | 🟢 | 串口 banner `minios v0.2.0` + `init: starting sh` 校验 |
| cargo fmt --check | 🟢 | |
| 无二进制入库 | 🟢 | `scripts/check-no-binaries.sh` / `check-no-secrets.sh` 通过 |
| 单元测试 | 🟢 | `make test`：4 tests（string/bitmap/ELF），零 warning |
| 集成测试（qemu 脚本化） | 🟢 | `scripts/run-qemu.sh --smoke` / `--usertests`（`make test-qemu`） |
| 文件行数 ≤400（新增内核源） | 🟢 | 内核最长 399 行（console.rs）；user/ 为 xv6 上游测试源 |
| usertests 全绿 | 🟢 | bigarg…bigdir/exec，`ALL TESTS PASSED` |

## 阻塞 / 待批准事项

- 无技术阻塞。
- 待用户决定（不阻塞 M1 收尾）：
  1. 是否对 `MoSunDay/xv6` 执行历史净化（破坏性，需 force-push，见 10-git-history-cleanup.md）；
  2. 用户态长期策略：M1 复用 xv6 C 程序（当前实现），后续是否 Rust 化（stretch）。

## 下一步

1. M2 起步：bootmem 区间登记（multiboot mmap → memblock 风格保留/释放）。
2. buddy order 0–10 + 每 CPU 单页缓存；slab 通用/专用 cache，接入 `kmalloc/kfree`。
3. 为 buddy/slab 增加 host 单测（分配释放、碎片注入），并用 `make test-qemu` 保持 usertests 回归。
