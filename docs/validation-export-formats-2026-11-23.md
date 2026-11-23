# 验证记录：导出的显式格式选择与源格式默认名（2026-11-23，ADR-0052）

范围：ADR-0052（裸 `EXPORT` 的源格式默认名、typed `EXPORT <TOKEN>` 选择器、
`export.format_mismatch` 拒绝），以及 File ribbon 的导出格式子菜单。`SAVE` 不变（恒 `.ymodel`）。

## 1. 已运行

### 1.1 契约测试

```
cargo test -p cad-ui-core --locked
→ 853 lib + 1 + 6 + 0 passed，0 failed（1 ignored 为既有用例）

cargo test -p cad-host --locked
→ 464 lib passed，0 failed（1 ignored 为既有 perf baseline）

cargo test -p app-web --locked
→ 148 passed，0 failed

cargo test -p cad-host export_formats --locked
→ 5 passed（最终工作树复跑）

cargo test -p cad-ui-core export_arguments --locked
→ 4 passed（最终工作树复跑）
```

新增/改变含义的契约：

| 面 | 证据（测试名） |
|---|---|
| token 语法 | `export_tokens_resolve_case_insensitively_to_their_writer`：11 个 token 的 `(format_id, extension)`，trim + 大小写不敏感；`ZIP`/空/多词不解析 |
| 默认名 | `a_source_extension_only_suggests_a_non_experimental_write_back_format`：`.dxf/.stl/.obj/.ply/.off/.coff/.ifc/.glb/.gltf` 源建议同名；`.dwg`/`.skp`/`.ymodel`/`.step`/`.dgn`/`.rvt`/无扩展名/未知回退 `.dxf` |
| 选择器接入 | `export_suggests_a_writable_source_format_extension`、`an_experimental_read_only_or_blank_source_still_suggests_dxf` |
| 强制写出器 | `a_forced_glb_export_writes_the_gltf_connector_for_a_glb_target`：`EXPORT GLB` + `.glb` 目标走 glTF |
| 不匹配拒绝 | `a_forced_format_refuses_a_mismatched_target_before_writing`：`EXPORT GLB` + `.dxf` 目标 → `export.format_mismatch`，写前拒绝、无 compat 记录 |
| 状态不泄漏 | `a_bare_export_after_a_forced_one_forces_nothing`：强制格式在一次 `SaveExport` poll 后被消费，不渗到后续裸 `EXPORT` |
| 会话/命令表 | `the_export_selector_is_an_admitted_argument_form`、`typed_export_tokens_fix_the_writer_and_the_suggested_name`（`.glb/.gltf/.coff/.dwg` 建议名）、`an_unknown_export_token_is_refused_before_the_host`、`a_bare_export_still_opens_the_picker` |
| 前端菜单 | `apps/desktop-ui/test/export-entry.test.ts`：11 行（裸 EXPORT 在前 + 10 个格式），`command` 即文本，`EXPORT ZIP` 本地拒绝；`npm test` 1952 passed |

### 1.2 门禁（本轮实际执行）

```
python3 scripts/check-i18n.py                  exit 0（3139 keys，2 catalogs）
python3 scripts/check-command-docs.py          exit 0（ARGUMENT_SELECTORS 132 项，0 mismatches）
python3 scripts/check-file-size.py             exit 0（2641 files，最大 1000；save.rs 849、commands.rs 947）
python3 scripts/check-architecture.py          exit 0（38 packages）
cargo check -p app-linux --all-targets --locked                          exit 0
cargo check -p app-web --target wasm32-unknown-unknown --locked          exit 0
cargo fmt -p cad-host -p cad-ui-core -- --check                          exit 0
cargo clippy -p cad-host -p cad-ui-core --all-targets --locked -- -D warnings   exit 0
(cd apps/desktop-ui && npm test)                                         exit 0（tsc + 1952 tests）
```

## 2. 未运行

- 真实 Windows / macOS / Android 运行时与真实保存对话框、文件系统行为 **NOT VERIFIED**（只有编译/契约证据）。
- 真实浏览器（non-headless）与真实 GPU：本轮没有运行；Web 只有既有 `web-smoke`/软件渲染证据，不能用于本功能的视觉/像素结论。
- 真实 AutoCAD / SketchUp / Revit / MicroStation 打开本功能导出的文件 **NOT RUN**（vendor 接受性仍按各连接器既有边界）。
- 全量 `cargo test --workspace`、`cargo fmt --all`、workspace 级 clippy **NOT RUN**（按 AGENTS 的“只验证自己的改动范围”）。

## 3. 证据边界

- 本轮证明的是**代码路径与语法**：token 与写出器/扩展名映射、默认名选择、强制格式的一次性状态、
  写前不匹配拒绝、菜单到 `cad_command` 的文本形态。
- `EXPORT <TOKEN>` 不限制选择器的过滤器（`PlatformFiles::begin_save` 仍不接收格式）：只在建议名里
  带扩展名，最终仍由目标扩展名决定；不一致时拒绝。原生对话框的默认过滤器行为只按 Linux 既有实现
  复核，未做真机点击。
- 浏览器 slim bundle 只编译 DWG/DXF 写出器：`EXPORT GLB` 等在该载体按
  `unsupported-runtime: export.<id>` 显式拒绝；菜单行不按载体隐藏（共享层没有已编译 lane 的 wire 枚举）。
- DWG / SKP 写出仍是实验性；本记录不新增任何 vendor 兼容性结论。
