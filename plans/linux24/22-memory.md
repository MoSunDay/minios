# 22 — 内存管理（对标 Linux 2.4 mm/）

## 1. 分层总览

```
早期(bootmem: multiboot 内存图→空闲区间登记)
  └─► 物理页管理 mm/page_alloc.rs：buddy(order 0..10) + 每CPU页缓存
        └─► slab mm/slab.rs：kmem_cache(专用/通用 size-class)
              └─► 内核对象：inode/dentry/bh/req/pid/task...
虚存：arch/x86/mm/pgtable.rs(页表API) + mm/memory.rs(VMA/demand paging/COW)
      + mm/vmalloc.rs(内核动态映射) + mm/percpu.rs
页缓存与回收：fs/page_cache.rs ↔ mm/vmscan.rs(kswapd, LRU) ↔ (Stretch)mm/swap.rs
```

## 2. 物理内存（mm/page_alloc.rs）

- bootmem：解析 multiboot mmap（M0 已能读 mem_lower/upper，扩展读 mmap 数组），
  登记 usable 区间，排除内核镜像 [__kernel_base,_kernel_end)、BIOS/显存区域。
- buddy：zone 单一（无 DMA/Normal 分区，qemu 内存小）；
  free_area[11]（4K..4M）；`alloc_pages(order)`/`__free_pages`；
  碎片统计 /proc/buddyinfo。
- 每CPU 页缓存（2.4 hot/cold 简化）：order-0 单页 per-cpu 数组，减少锁争用（M8 SMP 生效）。
- `get_free_pages` 族 API 兼容层：GFP_KERNEL/GFP_ATOMIC 语义（后者不睡眠）。

## 3. slab（mm/slab.rs）

- kmem_cache：name/size/objslab/构造器；slab 由 buddy 页组成，对象位图管理；
  每 cache 有 per-cpu 数组缓存（2.4 slab 语义简化：无 per-node）。
- 通用 size-class 缓存（8..2048B），`kmalloc/kfree` 落在最近 class；
- 专用 cache 注册：task_struct(对齐 4096 特例→直接页)、inode、dentry、buffer_head、
  request、ext4_extent_path、sigqueue 等（M3+ 各模块建）。
- 统计接口：/proc/slabinfo。

## 4. 内核虚拟地址布局（i386 三级？否——两级页表）

```
0x00000000-0x0FFFFFFF  用户区（低端 256MB，M1 沿 xv6 习惯；后续按需扩）
0x10000000-0xEFFFFFFF  空洞/Stretch 扩展用户
0xF0000000-0xF7FFFFFF  vmalloc 区（128MB，M2）
0xF8000000-0xF8FFFFFF  kmap 固定映射窗口（Stretch，块>物理直映时才需要）
0xF9000000-            percpu 区（每CPU 64K）
0xFFFF0000-            内核直映区：物理页全量线性映射（xv6 KERNBASE 风格但换高址）
0xFFFFF000             APIC/设备映射页（Stretch）
```
> i386 两级页表：PDE 1024×4MB 项。直映区 PDE 提前静态填充（大页 4M，
> 等价 xv6 entrypgdir 全局化），vmalloc/percpu 动态填 PTE。

## 5. 用户虚存（mm/memory.rs）

- VMA 链（vm_area_struct 等价：start/end/prot/flags/{file-backed|anon}）；
  brk 堆扩展、栈自动增长（GUARD 页）。
- 缺页：匿名页 zero-fill；文件页走 page_cache（M4 接通，M2 前仅匿名+ELF 段）。
- COW：fork 标记只读+引用计数页，写缺页时复制（页计数在 struct page，
  buddy 元数据区直映前端）。exec 丢弃旧 mm。
- mmap 系统调用：匿名 MAP_PRIVATE 必做；MAP_SHARED/file-backed 随 page_cache 在 M4；
  mprotect 最小实现；munmap。

## 6. 回收与交换（mm/vmscan.rs，Stretch: swap）

- LRU 双链（active/inactive）挂 page_cache 页；kswapd 周期 + 分配水位（low/min）触发。
- 回收顺序：干净页直接弃、脏页经 address_space writepage 回写、匿名页无 swap 时不可回收
  （记录并按 2.4 语义 OOM：杀最大 badness 进程——badness = rss×√cpu_time 简化）。
- swap（Stretch）：分区/文件作 swap（页槽位图），核心 swap_out/in 路径，
  成功后 shm 与 COW 页换出可达；不做 swapcluster 优化。

## 7. 校验与测试

- host 单测：buddy（随机 alloc/free 序列后空闲页守恒、order 合并正确）、
  slab（对象复用、越界 canary）、VMA 合并/分裂。
- qemu 集成：/proc/meminfo|buddyinfo|slabinfo 快照；fork 风暴（500 并发）无泄漏
  （空闲页计数回归）；COW 写后页计数正确（/proc/pid/stat 校验 min_flt/maj_flt）。
