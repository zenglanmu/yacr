# Vendor plugin pass（Tianzheng / TCH_*）

> 状态：**真实 base-6 解码 + 派生表示（ABI v4）**。插件已能从真实
> `test-tangent-real.dwg` 解出 `TCH_WALL` / `TCH_COLUMN` / `TCH_TEXT` / `TCH_MTEXT` /
> `TCH_OPENING` 的显示图元（4,525 墙 + 1,000 柱 + 1,258 文字 + 1,711 门/窗 2D 图块 +
> 18 洞口 = 8,512 个 primitives），并用 `TCH_OPENING` 的 Wall 句柄切出 1,734 个墙断口；
> 宿主把结果放入 `VendorRepresentationStore`，在场景装配时挂到源实体上，文字用宿主
> `FontEngine` 成形为线条/网格，门/窗按 opening 的 **2D 图块句柄**解析已导入的块定义并展开，
> **不写 `cad-db`**。其余 TCH 类、门/窗门槛线、弧墙/带轮廓墙、尺寸/轴号/符号仍显式列为
> unsupported；`applied` 必须等于帧内 primitive 总数，空成功会被拒绝。

## 1. 形态

- 插件是 **WebView 内的 TypeScript 模块**，源码在仓库根 `plugins/tch/src/*.ts`，
  由 `scripts/build-vendor-plugin.sh`（`rolldown` 打包为单个 ESM）构建为
  `plugins/tch/dist/index.js`，再复制到 `apps/desktop-ui/public/plugins/tch.js`；
  Vite 把它复制到各载体的 `dist/plugins/tch.js`。
- 宿主用 `<script type="module" src="plugins/tch.js">` 加载（与 canvas bootstrap 同一模式），
  插件把唯一门面注册到 `globalThis.__YACR_VENDOR_PLUGIN__ = { abiVersion, decode }`。
- 插件只做 **字节 → 图元**；它不读写 `cad-db`、不调用 Tauri `invoke`、不碰文件系统。
- native 与 web 共用同一份 `.js`；CLI/headless **没有 WebView**，因此 CLI 不使用插件，
  TCH 在 CLI 侧保持显式不支持。
- 生成物 `plugins/tch/dist/` 与 `apps/desktop-ui/public/plugins/` 被 `.gitignore` 忽略；`npm run build`、
  `build:web`、`build:android`、`dev` 都会先执行 `build:plugins`。
- **所有 native 宿主共用 `cad-desktop-shell` 的同一套命令**：`app-linux`（`native-below`）、
  `app-windows`、`app-macos`、`app-android` 都注册 `cad_vendor_records/apply/status`；
  `dom-canvas`（Linux X11/XWayland、Web）由 `apps/app-web` 的 wasm 会话提供同样三个名字，
  前端 adapter 负责二进制 records 的转发。协议状态机只有一份：
  `cad_ui_core::session::vendor::VendorStore`。
- 当前只在 Linux 环境运行/契约验证；Windows/macOS/Android 仅有编译/契约证据，Web 只有
  wasm 编译与 adapter/单测证据——都不是真机或 GPU 验收。

## 2. 时序

```
打开文档
  → 宿主发布基础文档
  → owner tick 发出 vendor-pending { token, generation, entities }
  → 前端 cad_vendor_records(token) 取二进制 records 帧（ArrayBuffer）
  → 插件 decode(records) 返回二进制 result 帧
  → 前端 cad_vendor_apply(token, generation, base64(result))
  → 宿主校验 token/generation/ABI/上限
  → 下一次 tick 发出 vendor-settled { token, applied, diagnostics }
```

- 每次成功发布文档只取一次 records（即使为空）：非空才 arm 新 pass；空记录会清掉上一个
  pass，避免旧 token settle 进新文档。同一文档重复 flush 不会重复 arm。
- 迟到/旧文档的 apply 以 `vendor.stale_token` / `vendor.stale_generation` 拒绝。
- 事件与命令都**不是每帧**：只有打开/重新导入时发生。

## 3. 命令

| 命令 | 参数 | 返回 |
|---|---|---|
| `cad_vendor_records` | `{ token }` | 二进制 `ArrayBuffer`（native `Response` / wasm `Uint8Array`） |
| `cad_vendor_apply` | `{ token, generation, payload }`（payload = base64） | `{ accepted, applied, diagnostics }`（settle 后立即 flush `vendor-settled`） |
| `cad_vendor_status` | — | `{ token, generation, entities }`（晚连接恢复） |

`payload` 用 base64 而不是 Tauri raw request：Tauri v2 在 Android 上不支持
`InvokeBody::Raw` 请求体。响应方向仍走 `ArrayBuffer`。

- `native-below`：以上三个 Tauri 命令由 `cad-desktop-shell` 注册；`dom-canvas` 分支不再
  提供 native 命令（Linux 的 `dispatch` 会显式拒绝）。
- `dom-canvas`：router 把同样三个名字交给 wasm 导出
  `vendor_records(token) -> Uint8Array` / `vendor_apply(token, generation, payload) -> JSON` /
  `vendor_status() -> JSON`；`apps/desktop-ui/src/host/web-adapter.ts` 负责把 typed array
  还原成 `ArrayBuffer`。没有导出时按名字显式拒绝，不返回空 buffer。
- 两种载体共用同一 `VendorStore` 的 token/generation/ABI/`applied` 规则，不产生第二份文档
  权威。

## 4. 二进制帧（ABI v4）

Records（宿主 → 插件，little-endian）：

```
u32 magic = 0x59414E44 ("YAND")
u32 abi   = 4
u64 token
u64 generation
u32 record_count
per record:
  u16 class_len, class bytes
  u32 dwg_version          // 1027 = AC1027
  u8  source               // 0 = UnknownEntity, 1 = ProxyEntity
  u64 handle
  u32 main_bits, u32 main_len, main bytes
  u32 text_bits, u32 text_len, text bytes
  u32 handle_bits, u32 handle_len, handle bytes
  u32 main_skip_bits, u32 text_skip_bits, u32 handle_skip_bits
  u32 reference_count, u64 references...
```

`*_skip_bits` 是 common entity header 在每个流里消耗的位数；插件从这些位置开始读
TCH class body。代理记录的 `payload/text_payload` 本来就是 class body，三个 skip 为 0。

Result（插件 → 宿主，little-endian）：

```
u32 magic = 0x59414E52 ("YANR")
u32 abi   = 4
u64 token
u64 generation
u32 status            // 0 ok, 1 unsupported
u32 applied           // 必须等于下列 primitive 总数
u32 diagnostic_count  // <= 16
per diagnostic: u16 len, utf8 bytes   // len <= 512
u32 entity_count
per entity:
  u64 handle
  u32 primitive_count
  per primitive:
    u8 kind
    0 = connected lines: u32 point_count, point_count * (f64 x, f64 y, f64 z)
    1 = segments:        u32 point_count (even), point_count * (f64 x, f64 y, f64 z)
    2 = arc:             f64 cx, cy, cz, radius, start_angle, sweep
    3 = circle:          f64 cx, cy, cz, radius
    4 = text:            f64 x, y, z, height, rotation, wrap_width (0 = none),
                         u8 attachment (1..9), u16 text_len, UTF-8 bytes
    5 = block insert:    u64 block_handle, f64 ox, oy, oz,
                         f64 sx, sy, sz, f64 rotation
    4 = text:            f64 x, y, z, height, rotation, wrap_width (0 = none),
                         u8 attachment (1..9), u16 text_len, UTF-8 bytes
```

上限：records 帧 ≤ 64 MiB；result ≤ 4 MiB；diagnostics ≤ 16 条、单条 ≤ 512 B；
`applied` ≤ 2,000,000、primitive 总数 ≤ 2,000,000、单 primitive 点 ≤ 1,000,000。
`applied` 与解析出的 primitive 总数不一致 → `vendor.applied_mismatch`；有 records 的
pass 返回 `status=0, applied=0` → `vendor.empty_success`；`status=1` 携带 primitive →
`vendor.unsupported_with_primitives`。任何截断、尾随字节、非有限坐标、
magic/ABI/token/generation 不匹配、非 UTF-8 诊断都显式拒绝。

## 4.1 records 从哪里来

`cad-import-acadrust::vendor::extract_vendor_records` 在导入后遍历所有 block 记录：

- `EntityType::Unknown` 且 `dxf_name` 以 `TCH_` 开头：按 R2007/R2010+ 的 framing 拆分
  `raw_dwg_data` 为 main/text/handle，再用 acadrust 的 `read_common_entity_data` 计算
  common-data skip；
- `ACAD_PROXY_ENTITY`：`dxf_subclass` 或 `class_id → CLASSES` 命中 `TCH_*` 时，
  直接使用 `payload` / `text_payload`（R2000 无 subclass，靠 class_id）。

记录按 handle 排序；单流 ≤ 16 MiB、总量 ≤ 256 MiB、条数 ≤ 200,000；超限时截断并写
`vendor.record_skipped` 诊断。真实样本已验证：`test-tangent-real.dwg` 的 `TCH_WALL`
class body 在 skip 后首字节为 6；R2000 代理样本首字节为 `0x0e`。

## 5. 失败闭合

- 插件缺失/加载失败/帧非法：`vendor-pass.ts` 构造一个真实 `status=1` result 帧回传，
  宿主 settle 为 `accepted:false` + 诊断；不会停在 pending。
- `applied` 与 primitive 总数不一致：`vendor.applied_mismatch`；有 records 却空成功：
  `vendor.empty_success`；非有限坐标/未知 primitive kind/文字 primitive 都显式拒绝。
- `dom-canvas`：由 wasm 导出服务同样三个命令；没有导出时按名字拒绝，不排队、不伪造结果。
- CLI：不使用本通道；TCH 不支持必须显式呈现。

## 5.1 落库语义：派生表示（已选 C）

插件返回的 primitives **不写入 `cad-db`**。原始 `TCH_*` 实体保持为唯一数据源；
解码结果进入 `cad-app::vendor::VendorRepresentationStore`，按 `(pass revision, source handle)`
索引；每个 pass 生成一个不可变快照，渲染会话在 `SceneVersion` 里比较 revision 后重建 base：

- 渲染/装配：`cad-app` 在生成 `DisplayFragment` 时查询该存储，把派生片段挂到
  **源实体的 id** 上，来源标记 `GeometrySource::ProxyCache`、`Precision::Approximate`；
- 拾取/选择：命中的片段带源实体 id，因此仍选中原始 vendor 实体，不产生“碎片实体”；
- 图层/可见性：派生片段继承源实体的图层与可见性；
- 打开/关闭：新文档发布时先清空派生快照（native 由 owner tick、`dom-canvas` 由 open 路径），
  重开幂等，不重复追加；
- 保存/导出：`cad-db` 里没有新实体，vendor 对象仍按原样（Opaque）存在；派生几何不参与
  保存/导出。若将来要“分解成普通图元”，必须做成显式命令，而不是打开时自动物化。

因此不会出现“打开一次多一批图元”的 explode 效果。代价是：测量/查询若只读 `cad-db`
语义几何，仍会把 vendor 实体视为不支持；派生几何的可测量性需要后续在 measure/query
层按 source handle 接一条派生通道。

## 6. 已知缺口

0. **载体验证边界**：命令与事件在全部五个宿主上已按同一契约接线；但只有 Linux 做了运行/
   契约测试。Windows/macOS/Android 为编译证据，Web 为 wasm 编译 + adapter/单测证据。
   **worker（OffscreenCanvas）隔离载体没有页面侧的 vendor 通道**：该 carrier 的 wasm 实例
   在 Worker 内，页面 adapter 不访问它；这一限制必须在状态/文档中保持显式。
1. **解码覆盖**：当前解 base-6 明文 `TCH_WALL`（直墙；弧墙/带 profile 墙跳过）、
   `TCH_COLUMN`（版本 6）、`TCH_TEXT` / `TCH_MTEXT`（宿主字体成形）及 `TCH_OPENING` 的
   **墙断口 + 门/窗 2D 图块 + 洞口侧线**。门/窗图块以 opening 记录里的 2D block 句柄
   （ABI kind 5）交给宿主，由 `DrawingDatabase::block_by_source_handle` 解析为已导入的块定义
   后展开；句柄不存在时只报 `vendor.tch.opening_block_missing`，不画替代符号。门槛线
   （door flags 0x200/0x400）、组合门窗（kind 8）和带形窗（kind 5）仍不绘制；
   轴号、尺寸、楼梯、符号、表格、机电等仍未解码，
   只进 `vendor.tch.unsupported` 诊断。`TCH_MTEXTNEW` 显式不支持；TCH 文字的 `_TCH`
   宽度因子/中西文分段/上标规则未复刻，只按宿主字体引擎的 MTEXT 排版成形。宿主没有可用
   fallback 字体时，文字 primitive 不产生可绘制片段（不会伪造字形）。
2. **派生表示范围**：`VendorRepresentationStore` 按 `source_handle` 索引，在模型空间顶层
   实体的 `build_expanded` 之后追加片段；块定义内/INSERT 展开中的 TCH 实体尚未合并。
   派生片段带源实体 id、`GeometrySource::ProxyCache`、`Precision::Approximate`，因此拾取
   仍指向原 `TCH_*` 实体；块图元的子片段也强制挂回 opening 源实体并清空 instance path，
   不会把临时 INSERT 暴露给拾取。测量/查询仍按 `cad-db` 语义几何，暂不识别派生几何。
3. **插件隔离**：当前是普通 module script，和页面同权；不可信插件应改为 Worker/wasm
   或 sandboxed iframe。
4. **真实格式版本**：本实现按真实样本的 base 6 / 保护标记 9 逆向；R2000 代理样本的
   `0x0e` payload、T30 基类 9–14 仍未在插件中实现。

## 7. 测试

- Rust：`crates/cad-ui-core/src/session/vendor.rs` 的 v4 帧/token/`applied`/primitive 边界
  测试；`crates/cad-desktop-shell/src/events.rs` 的 staged-once 与 settled-once 测试；
  `apps/app-web/src/vendor.rs` 的 wasm 侧 stage/settle/清空测试；
  `crates/cad-app/src/render_scene/vendor_tests.rs` 的源实体合并测试；
  `crates/cad-import-acadrust/src/vendor.rs` 的 `YACR_TEST_DWG` 抽取测试。
- 前端：`apps/desktop-ui/test/vendor-plugin.test.ts`（v4 帧、base-6 墙/柱/文字/门窗、
  块图元、洞口侧线、弧墙/profile 诊断、截断/坏 magic、bulge→arc）；
  `apps/desktop-ui/test/web-adapter.test.ts`（wasm vendor 命令转发/拒绝）。
- 真实样本：`test-tangent-real.dwg` → plugin `status=0, applied=8512,
  TCH_WALL=4525, TCH_COLUMN=1000, TCH_TEXT=876, TCH_MTEXT=382, TCH_OPENING=1729`，
  kind 5 block 图元 1,711，opening cut 1,734，洞口侧线 18；Rust 解析同一 result 帧并断言
  `applied == primitives`。真实 2D 块（`0x3DD82`、`0x3E6F4`）可由导入 DB 的
  `block_by_source_handle` 解析并展开（定向 `#[ignore]` 测试）。
- 构建：`npm run build:plugins` → `scripts/build-vendor-plugin.sh` 产出 `plugins/tch/dist/index.js`
  并复制到 `public/plugins/tch.js`；`npm test` 同时 typecheck 插件。
