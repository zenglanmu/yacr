# 验证：ymodel zstd entry codec + 两项既存修复（ADR-0046，2026-11-22）

- 范围：`cad-store-zip` 的 payload codec（zip `Stored` + `structured-zstd =0.4.3` level 6）、
  manifest v2（`codec`/`len_c`/`sha256_c`、`crc` 重定义）、`FORMAT_READ/WRITE = 2`；
  以及验证中暴露的两项**既存**问题的修复：`BlockId(0)` 与 `serde_json` 浮点解析。
- 环境：Linux（本机），pinned toolchain 1.99.1；无 GPU 参与（纯编解码/序列化）。
- 结论边界：本页只证明**库层与本机真实语料**；Windows/macOS/Android/Web 运行时、
  跨平台字节一致性、真机/像素/产品验收均 **NOT VERIFIED**。

## 门禁

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-store-zip --locked` | **134 passed**（49 unit + 85 integration：archive 18、container 9、raw_copy 7、resources 12、textures 6、parameters 11、parameters_raw_copy 2、document_fixed_point 8、bim_minor 8、bim_relations 1、unrepresentable_model 3） |
| `cargo test -p cad-import-acadrust --locked` | **540 passed**, 0 failed（含新 `imported_block_ids_start_at_one_and_never_use_the_reserved_zero`） |
| `cargo test -p cad-db --locked` | **493 passed**, 0 failed |
| `cargo test -p cad-import-rvt --locked` | passed |
| `cargo test -p cad-domain --locked` | passed |
| `cargo check -p cad-store-zip --lib --target wasm32-unknown-unknown --locked` | passed |
| `cargo check --workspace --lib --target wasm32-unknown-unknown --locked` | passed（1 条既存 `app-web` dead_code warning） |
| `cargo check -p app-linux --tests --locked` | passed |
| `cargo clippy -p cad-store-zip -p cad-import-acadrust --all-targets --locked -- -D warnings` | passed |
| `cargo fmt --all -- --check` | passed |
| `python3 scripts/check-file-size.py` | passed（`write.rs` 949 行；`sink.rs` 101 行） |
| `python3 scripts/check-architecture.py` | passed（44 packages） |

新增容器契约（`tests/container.rs` + `tests/raw_copy.rs`）：Stored + zstd magic、
manifest 明文、`sha256_c` 不匹配在解压前拒绝、非帧 payload、帧声明长度不符、截断帧、
受建模条目写成 Deflated 拒绝、v1 归档 `unsupported_version`、超过 1 GiB entry 上限、
`sha256_c` 单独错误时在解压前拒绝。

## 真实语料（本机，Linux/native）

独立 harness（`/tmp/opencode/ymodelbench`，路径依赖 workspace crates）：
真实 DWG/RVT → `AcadrustImporter`/`RvtImporter` → `DrawingDatabase::snapshot` →
`ZipDocumentStore::write` → `read`。

| 语料 | JSON 总量 | 旧 zip deflate 归档 | 新 zstd 归档 | 变化 | 写 / 读 |
|---|---:|---:|---:|---:|---:|
| `rmeadvancedsampleproject.rvt`（38 MB，6,783 元素 / 4 分片） | 189.1 MB | 23,967,922 B | **7,825,642 B** | **−67.4%** | 2.33 s / 3.15 s |
| `test-fonts.dwg`（14 MB，50,901 元素 / 140 分片） | 45.5 MB | 4,249,627 B | **3,570,128 B** | **−16.0%** | 0.72 s / 0.93 s |
| `fixtures/complex-test.dwg`（4.1 MB，44,591 元素 / 1,100 分片） | — | 未测 | 9,289,364 B | — | 1.49 s / — |
| `fixtures/test-tangent-real.dwg`（9.1 MB，87,299 元素 / 637 分片） | — | 未测 | 8,891,890 B | — | 2.31 s / — |

旧归档数字来自改动前的同一 harness（`/tmp/opencode/ymodel-bench-{dwg,rvt}`）。

### 读回结果

- **RVT：完整 `DocumentSnapshot` 相等 = true。** 读侧对每个 entry 先校验 `sha256_c`
  （帧字节）再解压并校验 `sha256`（逻辑 JSON），因此该成功同时证明 zstd 帧路径逐字节无损。
- **DWG：读回成功**（`test-fonts.dwg` 不再被 `BlockId(0)` 阻断）；排序后的
  `shards`/`elements` 与写前完全相等（`canonical_shards_equal=true`）。完整 `Vec` 比较为
  false 仅因为格式按 id 排序写出/读回，而导入的 snapshot 分片顺序不同——这是既有的规范化，
  不是数据差异（header/models/tables/textures/resources 均 equal）。
- 抽查 `.ymodel`：所有条目 `Stored`（zip method 0），`manifest.json` 明文可读，
  `unzip -p … | zstd -t` 通过并解出原 JSON。

## 本轮修复的既存问题

1. **`BlockId(0)`**：`cad-import-acadrust::read_blocks` 曾从 0 起分配块 id，而格式保留
   `BlockModel` seq 0。现在：
   - 导入块 id 从 **1** 起（与 `cad-db` 的 `next_block_id: 1` 一致）；
   - `.ymodel` 写出侧新增 `cad-store-zip::models::storable_model_id` 校验：任何无法经
     `PersistentId::from_raw` + `ModelKey::from_persistent` 回读的 model key 在
     `sink.begin` 之前以新稳定标记 **`ymodel.unrepresentable_model`** 拒绝，不再产生自己的
     reader 拒绝的归档；
   - 回归：importer 1 项 + store 3 项（含“可表示的 block 仍可写读”）。
2. **`serde_json` 浮点解析**：workspace 由 `features = ["raw_value"]` 改为
   `["raw_value", "float_roundtrip"]`。此前快速解析会把 f64 的最后 1–2 位改写
   （例如 `0.11999999731779099` → `0.119999997317791`，entry 哈希仍匹配但 snapshot 不等）。
   现在 RVT 的完整 snapshot 相等；该 feature 是正确性设置，不是性能旋钮。

## 附带修复（full gate 暴露的既存 stale fixture）

`cad-desktop-shell/tests/fixtures/cad_status_{probe_mode,no_snapshot,clean_session,projected_popup}.json`
缺少 ADR-0044 新增的 `iblEnabled` 字段，导致 `wire_goldens::cad_status_wire_matches_the_pinned_fixtures`
在 HEAD 上本就失败。本轮按当前 wire 输出补上 `"iblEnabled": false`，未改变任何断言或代码。

## 附带修复：`document_is_3d` 的 culling-bound 误判（51 项 cad-ui-core 测试）

`08571234`（"default 3D opens isometric"）用 `DrawingDatabase::bounds()` 的 z 跨度判断 3D。
该 bounds 是**裁剪**边界，会把圆/圆弧/椭圆膨胀成 `center ± radius` 的球；demo 图形
（平线 + 平面圆 + 凸度多段线）的 z 跨度因此被算成 1600，所有以 demo 开局的 cad-ui-core
契约都从 2D 变成 3D（51 项失败）。

修复：`document_is_3d` 改为 **tight** 检查——平面 conic 只取 `center.z`；倾斜 conic、
`Body3d`、非平面 `Insert` 变换判为 3D；点式几何（line/polyline/spline/mesh/text）直接累加
z，只有 hatch/annotation 回退到有界 tessellation。新增回归
`a_planar_conic_does_not_make_a_flat_drawing_three_dimensional`。
结果：`cad-app` 3128 passed、`cad-ui-core` 848 passed（原 51 项失败全部恢复）。

## 全量测试

`cargo test --workspace --locked -- --test-threads=1` **通过（exit 0）**。此前报告的
`cad-ui-core` 51 项失败已由 tight `document_is_3d` 修复（见上）；`cad-desktop-shell` 的
stale wire golden 也已补齐。

## NOT RUN

- Windows / macOS / Android / Web 的 `.ymodel` 运行时保存/打开（本轮只做 Linux + wasm 编译检查）。
- 跨平台归档字节一致性；跨版本 codec 字节稳定性（只承诺 pinned `=0.4.3` + level 6 + 固定
  feature 下同一构建的确定性）。
- 真实窗口内保存/重开、崩溃恢复、性能门槛（ADR-0037 D17）。
- 任何 GPU/像素/产品验收结论（本轮不涉及）。
