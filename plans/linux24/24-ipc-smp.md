# 24 — IPC 与 SMP（对标 Linux 2.4 ipc/ + SMP）

## 1. SysV IPC（ipc/）

- 公共：ipc_ids（inum→id 反查、RCTLIPERM 简化为 root 全权）、key/IPC_PRIVATE、
  ipc_perm{key,uid,gid,cuid,cgid,mode}、权限位检查。
- sem（ipc/sem.rs）：
  - sem_array{nsyms, sem[]}: sem{val, pid, zcnt, ncnt}；
  - semop：P/V + UNDO 简化（不做 SEM_UNDO 调整链，标记 Stretch）、Z (val==0 等待)；
  - 等待队列与 SETVAL 时的唤醒（唤醒全部等该 sem 的）；
  - semctl：IPC_STAT/SET/RMID/GETALL/SETALL/GETVAL/SETVAL/GETPID/GETNCNT/GETZCNT。
- shm（ipc/shm.rs）：
  - shm{size, nattch, pages(匿名页组), mode}；shmat/shmdt：VMA 映入 file-backed(special)
    地址空间（MAP_SHARED 语义，页全局唯一）；shmctl：IPC_STAT/RMID(标记删除待 nattch=0)/LOCK(空)。
  - 与 COW/回收协同：SHM 页计入 RSS，不可换出（无 swap 时天然满足）。
- msg（ipc/msg.rs）：
  - msg 队列链表 + 每队列字节数上限（MSGMNB 16K 默认）；msgsnd 阻塞/非阻塞；
  - msgrcv type 精确/最小/负最大三类匹配；IPC_NOWAIT；msgctl 常用子集。
- /proc/sysvipc/{sem,shm,msg} 内容生成。

## 2. 管道与 FIFO（ipc/pipe.rs, ipc/fifo.rs）

- pipe：环形缓冲页（4K）+ 读写等待队列；容量/水位语义（写满阻塞、读空阻塞或 0 返回）、
  O_NONBLOCK、EPIPE+SIGPIPE（写端无读者）、dup 共享 file（xv6 语义兼容：fork 后 fd 表复制）。
- FIFO（命名管道）：devfs-like 注册 fifo 类型 inode，open 时无对端阻塞，
  内部复用 pipe 缓冲；unlink 与普通文件一致。
- select/poll 最小实现（fd 就绪位扫描，pipe/tty/普通文件三类 poll 方法），
  无 epoll（Out，2.5 特性）。

## 3. SMP（arch/x86/kernel/smp.rs + 全局锁纪律）

- 启动：BSP 读 MP 浮点表/ACPI(仅 MP 表，qemu -smp 支持)→ AP trampoline（16 位入口→页开→
  长 jump 到高址）→ 每 CPU 的 idle+kthread 池初始化（沿用 xv6 mp.c/entryother.S 语义翻译）。
- 中断路由：LAPIC 时钟/本地中断 + IOAPIC 路由（RTC/串口/IDE 分配）；8259 屏蔽。
- 每 CPU 数据（mm/percpu.rs）：GS 段基址指向 percpu 块（含 cpu_number、current、
  kernel stack 指针、preempt_count(恒0，非抢占)、软中断栈）。
- 锁层次（防死锁纪律，写进代码注释与审计脚本）：
  runqueue_lock > mm 锁 > inode/dcache 锁 > page_cache 树锁 > buffer/page 位锁 > 打印锁。
- 大内核锁过渡（对齐 2.4 时代）：M1–M4 单 CPU 路径先行；M8 引入 SMP 时先
  "BKL 粒度"（syscall 入口全局锁）跑通，再按子系统收缩到细粒度自旋锁；
  最终 syscall 路径无全局锁（lockdep 简化：自研顺序断言宏）。
- 每中断栈 4K、中断嵌套禁用（cli 于 IRQ 入口，softirq 出口跑）。

## 4. 同步原语（lib/sync，M1 起逐步齐全）

- Spinlock（含 irq save/restore 变体 spin_lock_irqsave）、RwLock（排队位图，读者偏好可饿写
  简化为交替批）、Semaphore（sleep 计数）、Mutex（sem=1 + 属主断言）、completion。
- 原子 API 封装在 lib/atomic.rs（test_and_set_bit/cmpxchg/set_bit...），
  供位图/引用计数使用；`atomic_t` 风格 newtype。
- per-cpu 变量访问 API：`this_cpu_write/read`（GS 基址）。

## 5. 验收（摘）

1. sem：双进程生产者/消费者（P/V）10 万次无死锁；Z 条件用例。
2. shm：双进程写同一 SHM 页互见；nattch 归零后 RMID 生效。
3. msg：优先级 type 抽取顺序正确；队列满阻塞→读取后唤醒。
4. FIFO：shell 后台 cat fifo | grep 管道链。
5. SMP：4 vCPU 下 usertests+ext4 全绿；ipi/reschedule 正常；
   压力并发下锁序断言零触发。
