# 验证记录：ymodel 外部资源打包 + 自定义字体目录（2026-11-20，ADR-0040）

范围：ADR-0040（`resources/{font,xref,image}/<key>`、`document.json.embed_resources`、
`tables/resources.json`、per-document minor 4/5）与同轮落地的**可持久化字体目录**设置
（`ui.paths.fontsDir`）及目录选择器（`cad_pick_directory`）。

分 lane 实现（格式核心 / store-zip / 宿主采集 / 字体目录设置 / 目录选择器 / React 界面），
下方每个数字都是本轮实际输出，未跑的一律写 **NOT RUN**。

## 1. 已运行

### 1.1 Rust 测试（七个受影响的 crate，一次运行）

```
cargo test -p cad-domain -p cad-db -p cad-store-zip -p cad-host \
           -p cad-ui-core -p cad-desktop-shell -p app-web --locked
→ exit 0；2189 passed, 0 failed
```

分 crate 的主要计数（该次运行及 lane 内运行一致）：

| crate | 证据 |
|---|---|
| `cad-domain` | 219 passed；`resource.rs` 键校验矩阵（每个 kind 的合法/非法扩展名、大写扩展名、`..`/`/`/绝对路径）、`embed_resources` 缺省/非布尔拒绝/往返 |
| `cad-db` | 362 + 13 + 5 passed；`tests::resources::*` 14 项：空负载/空名、幂等、同名冲突、快照往返（字节+名+source）、drop、无字节时**不写资源表**、悬空名/孤儿负载拒绝、表行形状（`source` 缺失/`null`/字符串） |
| `cad-store-zip` | 39 + 18 + 7 + 6 + 12 + 6 passed，含：往返、per-document minor（普通文档仍 4）、raw-copy 增量、键/摘要不符在写侧与读侧都拒绝、非法键三种形态、外来同路径碰撞（同字节接管 / 异字节 `resource.conflict`）、无 source 资源的不动点、一份表示里改名 → 拒绝保存、有表无字节 → 拒绝 |
| `cad-host` | 427 passed；保存契约 (a)–(g)：未开开关 → 归档无 `resources/` 且字节不变、可解析字体+缺失字体 → 一条条目 + 一条 miss 且保存成功、超预算跳过、preview 无副作用、连续两次保存不推进 revision、恢复归档不含资源且记 `recovery.resources_omitted`、Web 两条命令拒绝 |
| `cad-ui-core` | 790 passed（1 ignored）；`ymodel_resources` wire 形状、`directory_pick` 三个标记 |
| `cad-desktop-shell` | 62 lib + 3/6/12/9/4 integration；`cad_pick_directory` 注册表存在、取消/失败/忙语义、字体目录 wire 字段省略 vs `null` |
| `app-web` | 135 passed；两条资源命令与目录选择器在 `dispatch` 与 no-session 两条路径都按标记拒绝 |

### 1.2 全量门禁（本轮实际执行，全部 exit 0）

```
cargo fmt --all -- --check
cargo clippy --workspace --exclude app-android --exclude app-web --all-targets --locked -- -D warnings
python3 scripts/check-architecture.py
python3 scripts/check-tauri-capabilities.py
python3 scripts/check-fixture-manifest.py
python3 scripts/check-file-size.py
python3 scripts/check-command-docs.py && python3 scripts/test-check-command-docs.py
python3 scripts/check-i18n.py
python3 scripts/check-workflows.py
cargo check --workspace --all-targets --locked
cargo check --workspace --lib --target wasm32-unknown-unknown --locked
```

要点：架构 44 packages 无环、边界不变；capabilities 仍只允许 `@tauri-apps/api/{core,event}`
（目录选择器因此走 Rust 侧 XDG portal，**没有**引入 `plugin-dialog`）；i18n 3014 键两目录一致；
最大文件行数仍 ≤1000。

### 1.3 前端

```
cd apps/desktop-ui && npm test   → 1840 passed, 0 failed（tsc --noEmit + node --test）
cd apps/desktop-ui && npm run build → exit 0（只有既有的 chunk 体积提示）
```

### 1.4 独立复核

两轮独立架构/代码复核（读代码、不改代码）各产出 1 个 BLOCKER + 若干 SHOULD-FIX，均已修复并由
契约测试钉住：

1. 设计阶段：`tables/resources.json` 原稿写成对象外壳（会让带资源的归档**完全打不开**）；
   `is_modelled` 前缀匹配方向反了；恢复快照并不存在 `with_previous`。
2. 落地后：映射被存了两份且"表静默赢"；有表无字节会写出 minor 4 的不可打开文件；
   `source = None` 有两种拼写导致不动点不成立；读取侧无单条上限（解压炸弹）；采集方算错键时
   被报成"归档损坏"；外来碰撞只信未校验的摘要。

## 2. NOT RUN / NOT VERIFIED

以下均为**未运行**，不得据此宣称能力已验收：

- `cargo test --workspace --locked -- --test-threads=1`（全量测试）与
  `.github/workflows` 的 `linux-host-smoke`、`web-smoke`、release/package job。
- **真实跨机自包含**：没有用户提供的真实 DWG + 外部参照 + 字体组合，也没有第二台机器上的
  打开验证。仅库层与宿主契约。
- **字体度量一致性**：内嵌字体注册后的实际排版与源机是否逐像素一致，未测量。
- **外部参照重新导入保真**：`XREF RELOAD` 未实现，内嵌源目前只被保存/读回，没有任何消费路径
  被运行过。
- **真实 XDG portal 目录选择器**：没有在真实桌面弹出对话框；只测了取消/失败/忙/拒绝四条分支。
- **真实 GPU / 窗口 / 像素 / 性能**：全部未跑；软件渲染也不得替代。
- **Windows / macOS 真实目标**：只做了 Linux 主机上的 `cargo check`（其 target-gated 空壳）；
  真实目标编译与运行 NOT VERIFIED。
- **Android**：`app-android` 未做 `--target aarch64-linux-android` 检查（需要 NDK），运行时
  NOT VERIFIED；其 tick drain 只是编译级接线。
- **X11/XWayland `dom-canvas` 运行**：只验证了 `surfaceMode` 分支与拒绝路径，未真实运行。
- **Web 运行时**：`app-web` 只在宿主机与 wasm32 target 编译 + 单元测试；浏览器内 `.ymodel`
  仍不在其能力内。

## 3. 已知边界（写进代码与文档，不是缺陷清单）

- **raw-copy 的字节不变性从第二次保存起成立**：首次是流式重写 zip，raw-copy 会重写 local
  header。测试按实测断言"第三次 == 第二次"，而非假设"首次 == 第二次"。
- **内嵌资源键必须是 `sha256(bytes)` 前 32 hex**：写入与读取两侧都断言，采集方算错键会在保存时
  以 `resource.invalid_key` 失败，不会静默留下旧字节。
- **恢复快照永不含资源字节**（`recovery.resources_omitted`）；恢复后资源回到宿主解析，与
  "恢复不等于回滚"一致。
- **`ChangeScope::tables` 不驱动写出**：`tables/resources.json` 每次保存都从快照重编码；
  scope 条目只是信息，评论已改为如实描述。
