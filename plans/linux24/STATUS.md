# STATUS — 演进状态跟踪（活文档）

> 更新规则：每个里程碑/批次完成后更新本表；gate 未全绿不得进入下一里程碑。

## 总体进度

- 当前里程碑：**M0 已完成，M1 待启动**
- 总体自估：**8%**（M0 ✅ + 计划文档 ✅；代码主线尚未进入 M1）

## 里程碑状态

| 里程碑 | 内容 | 状态 | 证据（gate） |
|--------|------|------|--------------|
| M0 | Rust 引导骨架（multiboot + VGA/串口 + panic） | ✅ done | qemu 串口输出 `minios v0.1.0 (x86, rust)` / `boot ok`；make build 零警告 |
| M1 | xv6 功能等价移植（中断/分页/进程/系统调用/IDE/文件读写） | ⬜ pending | gate 见 20-roadmap.md |
| M2 | 内存管理现代化（buddy/slab/vmalloc/percpu） | ⬜ pending | |
| M3 | 调度与内核基础设施（O(n)/内核线程/软中断/信号/定时器） | ⬜ pending | |
| M4 | VFS + 块层（dcache/page cache/请求队列/procfs） | ⬜ pending | |
| M5 | ext4 只读 | ⬜ pending | |
| M6 | ext4 读写（无日志） | ⬜ pending | |
| M7 | jbd2-lite 日志 | ⬜ pending | |
| M8 | SMP/IPC/2.4 parity 收尾 | ⬜ pending | |

## Go-Live Gate（仓库级常绿清单）

| Gate | 状态 | 说明 |
|------|------|------|
| make build 零警告 | 🟢 | stable 1.97, i686 目标 |
| make run 冒烟通过 | 🟢 | 串口 banner 校验 |
| cargo fmt --check | 🟢 | |
| 无二进制入库 | 🟢 | CI + pre-commit 校验（见 10-git-history-cleanup.md） |
| 单元测试 | 🟡 | 当前仅骨架自检；M1 起建立 cargo test 面 |
| 集成测试（qemu 脚本化） | 🟡 | make run 已脚本化；usertests 待 M1 |
| 文件行数 ≤400（新增） | 🟢 | 最大文件 136 行 |

## 阻塞 / 待批准事项

- 无技术阻塞。
- 待用户决定（不阻塞 M1）：
  1. 是否对 `MoSunDay/xv6` 执行历史净化（破坏性，需 force-push，流程见 10 附录）；
  2. minios 用户态语言策略：M1 直接复用 xv6 C 用户程序（默认），或同步 Rust 化（stretch）。

## 下一步（M1 首批 TODO）

1. `kernel/src/arch/x86/`：GDT/IDT/TSS 加载，中断门，`interrupts/` 骨架（0–31 异常 + 32–47 IRQ 框架）。
2. PIT 时钟 + `time.rs` 节拍；`hlt` 空转主循环改为时钟驱动。
3. 串口/键盘中断驱动化（console 转中断 + 环形缓冲输入）。
4. 分页：entrypgdir（4MB 大页）→ `mm/paging.rs` 页表操作 API（参照 xv6 vm.c）。
5. 物理页分配器 `mm/frame.rs`（先 xv6 kalloc 语义：链表、4KB 粒度、0x100000 以上空闲链）。
