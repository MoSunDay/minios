# 01 — 基线审计（xv6 现状 + 环境实测，2026-11-14）

## 1. xv6 代码盘点（xv6 @ 2305a8c "Create mkfs.c"）

- 规模：**101 个文件，约 9,425 行**（C/H/S）；全部平铺根目录，无分层。
- 定位：MIT xv6-public（x86, 2011 教材版）。无 VFS、无 page cache、无 IPC/信号、
  NBUF=10、日志 10 块、FS 块 512B。

### 关键静态约束（param.h / fs.h）

| 参数 | 值 | 含义 |
|------|----|------|
| NPROC | 64 | 最大进程数 |
| NOFILE / NFILE | 16 / 100 | 每进程/全系统打开文件 |
| NBUF | 10 | 块缓存深度 |
| NDEV | 10 | 设备表项 |
| LOGSIZE | 10 | xv6fs 日志块数 |
| BSIZE | 512 | FS 块大小（ext4 需 1K/2K/4K） |
| MAXFILE | 140 (12+128) | 单文件块上限 |
| FSSIZE | 1000 | 镜像块数（512KB） |

### 系统调用面（21 个）

fork exit wait pipe read kill exec fstat chdir dup getpid sbrk sleep uptime open write
mknod unlink link mkdir close —— minios M1 目标：全部等价提供（同名同语义）。

## 2. 构建问题（实测）

- gcc 11.4 下 `mkfs.c` 编译失败：`static_assert` 重定义 × `-Werror`（glibc assert.h 冲突）。
  minios 的 mkfs 以 Rust 重写或加守卫，见 30-ext4.md。
- xv6 内核本体开发环境可链接（kernel ELF 142KB）；qemu-system-i386 可用。

## 3. xv6 git 历史二进制盘点（任务 1 依据）

- 提交数 848；.git 体积 12MB。
- 历史二进制 blob 合计 **13,126,345 字节（≈13.1MB）**；对象数分布：pdf 33、gz 13、
  ps 6、ppt 3（另有 web/*.html 40 个文本遗留）。
- 最大对象：web/os-lab-2.ppt 1.35MB、web/os-lab-2.pdf 1.01MB、xv6-rev2.tar.gz 724KB、
  xv6.ps 541KB×7 版本。HEAD 树中已无这些路径，仅存历史。
- 净化工具就绪：git-filter-repo（a40bce548d2c）已安装；流程见 10-git-history-cleanup.md 附录。

## 4. 开发环境实测

| 组件 | 状态 |
|------|------|
| rustc/cargo stable 1.97.1 | ✅ i686-unknown-linux-gnu std 已装 |
| qemu-system-i386 | ✅ |
| gcc 11.4（-m32 可用）/ ld | ✅ 供 xv6 C 用户程序对照构建 |
| mke2fs / debugfs / e2fsck / dumpe2fs | ✅ ext4 测试闭环关键依赖 |
| 网络 | github HTTPS 不可用；SSH 认证可用（MoSunDay）；pypi/rustup 直连可用 |

## 5. minios 起点（M0 已完成）

- Rust workspace：kernel crate（no_std、零依赖、panic=abort），target
  i686-unknown-linux-gnu，GNU ld + kernel/x86-minios.ld（VMA 0x100000、multiboot 头、16K 栈）。
- 已验证：multiboot 引导 → rust_main；VGA+COM1 双输出；multiboot info 解析
  （lower 639K / upper 129920K）；panic handler。
- 规模：12 个源文件，最大 136 行（约束：新增 ≤400 行/文件）。
