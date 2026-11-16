# 22 — x86_64 内存管理（对标 Linux 2.6.39.4 mm/）

## 1. 分层总览

```
multiboot mmap → bootmem → zones/buddy(order 0..10) → SLUB/kmalloc
                                      ├─► vmalloc/vmap/ioremap
PML4/PDPT/PD/PT + 直映 + percpu ──────┤
                                      └─► struct page/page cache
用户 VMA → M2 匿名缺页/brk/COW ─► 匿名 OOM（无 swap）
         └► M4 file-backed mmap/page cache → LRU/干净页回收/脏页 writeback
swap/swapin/swapout 仅 Parity M10
```

## 2. x86_64 地址与页表（M0R/M1R 基础，直映/vmalloc 于 M2 完善）

- 四级页表、4KiB 基础页、48 位 canonical 虚拟地址；user 指针与内核地址用
  newtype 区分。CPU 物理地址宽度通过 CPUID 获取，不硬编码 32 位物理地址。
- 初始布局（minios 自定义，非宣称与 Linux 地址完全相同）：

```
0x0000000000000000–0x00007fffffffffff  用户 canonical 低半区
0xffff800000000000–                     物理直映区（2MiB 页起步）
0xffff900000000000–                     vmalloc/vmap/ioremap 区
0xffffa00000000000–                     静态/动态 percpu 区
0xffffffff80000000–                     内核 text/rodata/data/bss
```

- **M2** 完整直映实际探测到的物理内存，提供检查溢出和 canonical 边界的 phys↔virt
  转换；校验直映权限及 usable/reserved 边界，预留页不得进入 buddy 分配。vmalloc/vmap
  将非连续物理页映到连续虚拟区并落实权限/释放；APIC/MMIO 经 ioremap 进入 vmalloc 区。
- EFER.NXE 开启，用户栈/数据默认 NX，etext/rodata 权限分层；SMEP/SMAP/PCID/1GiB
  页仅作能力探测后的可选项，不作为 M0R 依赖。
- CR3 切换、invlpg、全局页与 SMP TLB shootdown 统一由 arch/x86_64/mm 提供。
- **M1R UserPtr/copyin/copyout fault fixup**：所有用户访问指令登记异常表/fixup，
  #PF 可恢复到错误返回并释放已持有的内部状态。非 canonical/越界、未映射或已
  unmap、跨页且后继页无效的用户指针统一按接口约定返回 `-EFAULT`（内部可记录
  已复制字节数），不得 panic、越界访问或把部分复制报成成功。
- 不支持 ia32 compat 页表或 ELF32 用户地址空间。

## 3. 物理页与 SLUB（M2）

- bootmem 登记 usable/reserved，排除内核、页表、boot 模块与固件区域；
  DMA32/Normal 简化 zones，不引入 i386 highmem/PAE 概念。
- buddy free_area[11]、每 CPU order-0 缓存、碎片统计与 /proc/buddyinfo。
- SLUB 风格 per-cpu freelist + page slab 元数据；通用 kmalloc size-class 与
  task/inode/dentry/bio/buffer_head 等专用 cache；统计 /proc/slabinfo。
- `struct page` 元数据覆盖登记的物理区间，维护 refcount/mapcount/mapping/flags。

## 4. 用户虚存（M2 匿名 Core，M4 文件映射，M10 Parity）

- **M2**：匿名 VMA 树/链（start/end/prot/flags）、合并分裂、brk、匿名 mmap、
  munmap/mprotect 最小集、栈增长与 guard；匿名 zero-fill、COW fork 和 exec ELF64
  地址空间。M2 不提前混入 page cache/file-backed mmap。
- **M4**：接 23 的 address_space/page cache，落地 file-backed mmap、文件页缺页与
  已支持映射的写回；tmpfs 使用同一 VMA/page cache 生命周期。完整 MAP_* 标志、
  populate/fault 变体及 msync/mincore/madvise/mlock/mremap 在 M9/M10 逐项关差。
- page fault 必须校验 canonical、VMA 权限、用户/内核访问来源与错误码；
  kernel 访问 user 只经带 M1R fixup 的 copyin/copyout。

## 5. 回收、OOM 与 swap（Core 无 swap）

- **M2 gate**：匿名页无 swap 时不可回收；压力测试验证 OOM 候选选择、终止与
  统计（扫描/拒绝/释放/失败计数），不得误杀无关任务或静默丢内存。
- **M4 gate**：file-backed page cache 建 active/inactive LRU；干净页可回收，
  脏页必须 writeback 成功后才释放，I/O 错误向上传播。tmpfs 在无 swap 时有数据页
  不可回收，只回收可安全重建或生命周期结束的页，并进入同一 OOM 统计。
- **M8 gate**：多 CPU 并发 fault/unmap/writeback/reclaim/OOM 压力，校验 LRU、
  page refcount/mapcount、TLB shootdown 与每 CPU 统计，无 UAF、旧映射或双重释放。
- M10 才实现 swap 分区/文件、slot 映射、swapin/out、COW/shm 协同及完整水位；
  swap 未完成前不得声明 Linux 2.6.39.4 MM parity。

## 6. 测试

1. host：buddy 守恒/合并、SLUB 复用/canary、VMA 分裂合并、页表索引纯函数。
2. M1R qemu：非法/已 unmap/跨页 copyin/out 均返回约定错误且内核继续运行；
   同步验证 canonical/NX/权限 fault。
3. M2：直映 phys→virt→phys 与 virt→phys→virt 往返、地址边界/溢出、页面权限和
   usable/reserved 区，确认预留页不被 buddy 分配；vmalloc 用非连续物理页建立连续虚拟
   映射，逐页核对只读/NX 权限，释放后确认虚拟映射失效且物理页只归还一次。再执行匿名
   brk/mmap、COW 页计数、fork+exec 压力及匿名 OOM 选择/统计。
4. M4：file-backed mmap、tmpfs 生命周期、page cache LRU、干净/脏页回收及
   writeback 错误注入；无 swap 时不得释放仍含唯一数据的 tmpfs 页。
5. M8：SMP 回收/OOM 与 TLB shootdown 压力后无旧映射、泄漏或统计失衡。
6. Parity：LTP mm、swap/OOM 矩阵、真实用户态内存压力。
