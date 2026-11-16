# 10 — 无二进制历史政策 + xv6 历史净化（附录）

## 1. minios 仓库政策（任务 1 的主方案）

**minios 是全新仓库、全新历史：不导入任何旧提交，因此历史天然无二进制。**
首推 main 分支即首个提交。为保持"永不入库二进制"，落实四道防线：

1. `.gitignore`（已提交）：`/target`、`*.o/*.d/*.a/*.img/*.iso/*.bin/*.elf/*.map` 等构建产物。
2. `scripts/check-no-binaries.sh`（旧 M1 历史事实，当时已落地）：扫描全历史 blob，
   `git rev-list --objects --all | git cat-file --batch-check=...` 检查
   文本扩展名白名单 + 大小阈值（>512KB 即失败，Cargo.lock/.ld/.S/.rs/.c/.h/.md/.pl/.sh/.py/.toml 之外扩展名需人工白名单）。
3. pre-commit 钩子（scripts/install-hooks.sh）：按文件级 `git diff --cached --name-only`
   拦截白名单外或 >1MB 的暂存文件。
4. CI 门禁：工作流每次跑 check-no-binaries.sh（见 40-testing-ci.md）。

> 注意：合法文本大文件（如未来 dump 的黄金用例）若确需入库，走 LFS 或白名单评审，不搞豁免提交。

## 2. 为何不导入 xv6 历史

- xv6 历史 848 提交中含 **13.1MB 二进制**（web/ 教学材料、xv6.ps/pdf 多版本、release tar.gz），
  且其源码演化史对 minios（Rust 重写）无二分定位价值。
- minios 需要"参考实现"而非"历史"：xv6 本地工作副本保留为只读参考即可（见 ADR-0005）。

## 3. 附录：MoSunDay/xv6 历史净化流程（可选执行，破坏性操作）

> 状态：**未执行，待仓库所有者批准**。执行会改写全部提交哈希并需 force-push。
> 前置实测：git-filter-repo 已安装（a40bce548d2c）；待删对象合计 13,126,345 字节。

```bash
# 0) 全新镜像克隆（filter-repo 要求非 bare 也建议如此）
git clone --no-local git@github.com:MoSunDay/xv6.git xv6-clean && cd xv6-clean

# 1) 用独占临时文件记录净化前校验基准，退出时清理
tree_before="$(mktemp)"
trap 'rm -f -- "$tree_before"' EXIT HUP INT TERM
git rev-parse 'HEAD^{tree}' > "$tree_before"
git count-objects -vH | grep size-pack

# 2) 清除二进制路径（web/ 整目录 + 各版本 pdf/ps/tar.gz；其余历史保留）
git filter-repo --invert-paths \
  --path web \
  --path-glob '*.pdf' --path-glob '*.ps' --path-glob '*.ppt' \
  --path-glob '*.gz'  --path-glob 'xv6-rev*'

# 3) 校验：最终树必须与净化前完全一致（HEAD 树本就不含这些文件）
test "$(git rev-parse 'HEAD^{tree}')" = "$(cat "$tree_before")" \
  && echo TREE-IDENTICAL || echo TREE-CHANGED  # 必须输出 TREE-IDENTICAL

# 4) 复核仓库体积（预期从 ~12MB 降到 ~2MB 级）并确认无残留
git reflog expire --expire=now --all && git gc --prune=now --aggressive
git rev-list --objects --all \
  | git cat-file --batch-check='%(objecttype) %(objectsize) %(rest)' \
  | awk '$1=="blob" && $2>100000'   # 期望为空

# 5) 人审后强推（不可逆）
git push --force origin master
```

失败回退：远端未覆盖前，旧历史仍在 GitHub；覆盖后如需找回，只能依赖本地/镜像备份。
故第 5 步前建议 `git bundle create ../xv6-backup.bundle --all` 本地留档。
