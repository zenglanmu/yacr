# 验证记录：保存/导出分离 + 非公开格式提示 + 四载体 `.ymodel`（2026-11-21，ADR-0041）

范围：ADR-0041（`SAVE` 恒写 `.ymodel`、`EXPORT` 独立命令、允许覆盖源文件并由 `.bak` 兜底、
`SpecVisibility` + `cad_status.openedSource` + 打开时横幅），以及随之补齐的
Windows / macOS / Android 的 `.ymodel` 归档接线。

## 1. 已运行

### 1.1 测试（10 个受影响 crate，一次运行）

```
cargo test -p cad-import-core -p cad-platform -p cad-ui-core -p cad-host -p cad-app \
           -p cad-desktop-shell -p app-web -p app-android -p app-windows -p app-macos --locked
→ 3474 passed，1 failed（见下），其余全部 exit 0
```

唯一失败：`crates/cad-desktop-shell/tests/command_aliases.rs` 仍断言 `EXPORT → PLOT`（旧别名表）。
已改为 `EXPORT` 无别名、`PRINT → PLOT`，单独重跑 `cargo test -p cad-desktop-shell` →
`62 + 3 + 6 + 12 + 9 + 4` 全通过，exit 0。**该失败是文档/断言未同步，不是实现缺陷。**

新增/改变含义的契约：

| 面 | 证据 |
|---|---|
| SAVE | 任意源（dwg/dxf/skp/无源）→ `SaveYmodel` + `<stem>.ymodel`；无归档载体在开选择器**之前**以 `unsupported-runtime: ymodel` 失败（`cad-host` 共享契约 + `app-web` 真实会话：拒绝且不产生下载） |
| EXPORT | 规范命令、不再映射 `PLOT`；`PLOT`/`PRINT` 行为不变；选择器 11 个扩展名，写出器按目标扩展名分派 |
| 覆盖 | Linux 真实 `atomic_replace`：导出覆盖已存在文件 → `<name>.dxf.bak` 含旧字节、目标为新字节（`apps/app-linux/src/platform/tests.rs`） |
| `.bak`/原子写 | Windows：临时文件 → `ReplaceFileW`（回退 `MoveFileExW`）→ `.bak` 硬链接；macOS：临时文件 → `rename` → `.bak`；两者均有"失败不破坏目标"的测试 |
| 归档 | Windows 42 passed、macOS 34 passed、Android 73 passed；各含 `.ymodel` 往返 与 "`.ymodel` 打开不走连接器探测" |
| 提示 | `openedSource` 仅对 `dwg/dgn/skp/rvt` 出现（`canWrite`/`experimental` 取连接器描述符）；DXF/STEP/IFC/glTF/网格 → 字段缺失；dwg→dxf 打开与 New/Close 会清除；前端 1860 passed 覆盖逐格式文案、`canWrite:false` 与 `experimental` 互斥、按文档记忆关闭（**2026-11-22 起新增持久开关 `formatNoticeEnabled` 与“不再提示”，见 `docs/validation-mobile-chrome-properties-notice-2026-11-22.md`；本行记录的是当时状态**） |
| Android 无备份 | 覆盖成功但 provider 无法建 `.bak` → 发布 `save.no_backup`；chrome 把它翻成句子（新增令牌映射 + 测试） |

### 1.2 门禁（本轮实际执行）

```
cargo fmt --all -- --check                                    exit 0
cargo clippy --workspace --exclude app-android --exclude app-web --all-targets -D warnings   exit 0
cargo check --workspace --all-targets --locked                exit 0
cargo check --workspace --lib --target wasm32-unknown-unknown --locked   exit 0
cargo check -p app-windows --all-targets --target x86_64-pc-windows-msvc   exit 0
cargo check -p app-macos   --all-targets --target aarch64-apple-darwin     exit 101（见 §2）
python3 scripts/check-architecture.py                          exit 0
python3 scripts/check-tauri-capabilities.py                    exit 0
python3 scripts/check-fixture-manifest.py                      exit 0
python3 scripts/check-file-size.py                             exit 0
python3 scripts/check-command-docs.py && python3 scripts/test-check-command-docs.py   exit 0
python3 scripts/check-i18n.py                                  exit 0（3029 keys）
python3 scripts/check-workflows.py                             exit 0
python3 scripts/test-android-host.py                           exit 0
python3 scripts/test-macos-host.py                             exit 0
```

前端：`npm test` → 1860 passed，0 failed；`npm run build` → exit 0。

### 1.3 跨目标类型检查（Win32 分支确实被检查到了）

本机安装 `x86_64-pc-windows-msvc` rust-std 并把 `llvm-rc` 加入 PATH 后，
`cargo check -p app-windows --all-targets --target x86_64-pc-windows-msvc` **exit 0**——
COM `IFileSaveDialog` / `ReplaceFileW` / `MoveFileExW` 分支通过了类型检查（唯一告警是既有的
`cad-desktop-shell::performance::last_cpu_sample` 死代码）。**这不等于能运行。**

## 2. NOT RUN / NOT VERIFIED

- **macOS target 在本机不可检**：`objc2-exception-helper` 的构建脚本需要 macOS C 编译器
  （`cc: unrecognized command-line option '-arch'`）。该 lane 用**桩 `cc`** 跑通了
  `cargo check/clippy --target aarch64-apple-darwin`（Rust 侧类型检查通过，C 辅助库未构建）。
  因此 AppKit 调用是**类型检查**级别，链接与运行均未发生。
- **真实 Windows / macOS / Android 运行**：保存对话框点击、预填名与过滤器、取消/失败路径、
  `.bak` 在 NTFS（`ReplaceFileW` 与备用数据流）/ APFS 硬链接 / SAF provider 上的行为，全部
  **NOT RUN**。
- **恢复快照的设备路径**：Windows/macOS/Android 的 `recovery` 目录写入、列举、`OPEN` 恢复回读
  都只有契约测试；真实崩溃恢复未跑。
- **厂商接受性**："导出的 DWG/DGN/SKP 可能不被原软件正确打开、会丢信息"是依据本仓库既有
  `vendor NOT VERIFIED` 结论的保守表述，**没有**用 AutoCAD / MicroStation / SketchUp / Revit
  实测过。
- **Web**：`SAVE` 拒绝只有 `app-web` 单元/真实会话契约；浏览器内 `.ymodel` 不存在，
  `EXPORT` 下载的真实交互未跑。
- 全量 `cargo test --workspace --test-threads=1` 与 `.github/workflows` 的
  `linux-host-smoke` / `web-smoke` / release job 未运行。

## 3. 已知代价与边界

- Android 的原地保存需要在循环线程上同步读一遍目标文件做冲突检查，超大文件会阻塞一个 tick。
- Windows/macOS 复用既有的 `plot.save_*` / `linux.save_*` 文案键作为保存对话框标题与过滤器名
  （仓库没有 `windows.save_*` 键）；用户可见文案正确，但键名与载体不匹配，属待整理的命名债。
- macOS 工作区的 `-D warnings` 仍会被既有的 `cad-desktop-shell` 死代码告警挡住，与本轮无关。
