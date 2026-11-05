# Vendor plugin pass（Tianzheng / TCH_*）

> 状态：**真实 base-6 解码 + 派生表示（ABI v1）**。插件从真实
> `test-tangent-real.dwg`（10,612 条 vendor 记录）解出 9,889 个实体 / 14,911 个 primitive，
> 覆盖 22 个 TCH 类：墙 / 柱 / 文字 / 门窗 / 标高 / 箭头 / 多行引线 / 尺寸 / 轴号 / 半径标注 /
> 直线梯与矩形梯 / 剖面符号 / 索引符号 / 转角窗 / 天正图块插入 / 折断线 / 修订云线 /
> 图名 / 图号 / 指北针；并用 `TCH_OPENING` 的 Wall 句柄切出 1,734 个墙断口。宿主把结果放入
> `VendorRepresentationStore`，在场景装配时挂到源实体上（块内 TCH 成员按实例变换合并），
> 文字用宿主 `FontEngine` 成形为线条/网格，块引用按 **source handle** 解析已导入块定义后展开，
> **不写 `cad-db`**。其余类（`TCH_SPACE` 383、不匹配墙头/弧墙 157、未知版本或空墙的
> `TCH_OPENING` 70、旧 `TCH_DIMENSION` 35、`TCH_SHEET` 1、`TCH_ERRORMSG` 1）显式列在
> `vendor.tch.unsupported`；合法但无可见几何的记录列在 `vendor.tch.no_display`；
> `applied` 必须等于帧内 primitive 总数，空成功会被拒绝。

## 1. 形态

- 插件是 **WebView 内的 TypeScript 模块**，源码在仓库根 `plugins/tch/src/*.ts`，
  由 `scripts/build-vendor-plugin.sh`（`rolldown` 打包为单个 ESM）构建为
  `plugins/tch/dist/index.js`，再复制到 `apps/desktop-ui/public/plugins/tch.js`；
  Vite 把它复制到各载体的 `dist/plugins/tch.js`。
- 内置 `tch` 由宿主用 `<script type="module" src="plugins/tch.js">` 加载（与 canvas bootstrap
  同一模式）。经插件管理器安装的外部解码器由 `cad_plugin_read_entry` 读出 base64 字节，再用
  Blob URL 以 ES module 导入（§8）。两条路径都把唯一门面注册到
  `globalThis.__YACR_VENDOR_PLUGIN__ = { abiVersion, decode }`。
- 插件只做 **字节 → 图元**；它不读写 `cad-db`、不调用 Tauri `invoke`、不碰文件系统。
- 内置 `tch` 的 `.js` 由 native 与 web 共用；**外部安装的解码器只在 `native-below` 上加载**
  （§8）。CLI/headless **没有 WebView**，因此 CLI 不使用插件，TCH 在 CLI 侧保持显式不支持。
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

## 4. 二进制帧（ABI v1）

Records（宿主 → 插件，little-endian）：

```
u32 magic = 0x59414E44 ("YAND")
u32 abi   = 1
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
u32 abi   = 1
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

因此不会出现“打开一次多一批图元”的 explode 效果。代价是：`cad-db` 语义几何里没有这层
轮廓。`AREA OBJECT` 已按 source handle 接了一条**派生测量**通道（`cad-app/src/app_measure_vendor.rs`）：

- 派生图元连成**一个闭合平面环**时，直线段用鞋带公式、圆弧段加精确弓形项
  `½ r²(θ − sin θ)`（遍历反向时取负）；单个 `Circle` 直接 `π r²`；
- 记录来源标记为 `GeometrySource::ProxyCache` + `Precision::Approximate`，不冒充解析值；
- 文字、块引用、混合 circle、非平面、开口环、多环一律以
  `area.object.vendor_outline` 具名拒绝，不做采样估计；
- 选择属性面板追加只读的 `vendor_derived` / `vendor_primitives` / `vendor_revision`
  （i18n：`properties.row.vendor_*`）。

## 6. 已知缺口

0. **载体验证边界**：命令与事件在全部五个宿主上已按同一契约接线；但只有 Linux 做了运行/
   契约测试。Windows/macOS/Android 为编译证据，Web 为 wasm 编译 + adapter/单测证据。
   **worker（OffscreenCanvas）隔离载体没有页面侧的 vendor 通道**：该 carrier 的 wasm 实例
   在 Worker 内，页面 adapter 不访问它；这一限制必须在状态/文档中保持显式。
   **插件管理器只在 `native-below` 可用**（§8）：Linux X11/XWayland 的 `dom-canvas` 返回
   `plugin.unsupported_carrier: dom-canvas`，纯 Web 按名拒绝 `cad_plugin_*`；两者的内置 `tch`
   仍是 always-on，管理器不能禁用或替换它。
1. **解码覆盖**：22 个 base-6 明文类已解（见 §1 状态与 `plugins/tch/README.md` 表）。
   仍未解码/显式拒绝：
   - `TCH_SPACE`（383）：参考实现（MiniCAD）没有该类解码器，布局未验证；
   - `TCH_WALL` 中 157 条：147 条头字段不匹配已验证布局（`vendor.tch.wall_header_mismatch`）、
     10 条弧墙（`vendor.tch.wall_arc_unsupported`）；
   - `TCH_OPENING` 70 条：39 条 body 版本 64 未知、31 条 wall 句柄为 0；
   - `TCH_DIMENSION`（35，旧类；MiniCAD 也只支持 `TCH_DIMENSION2`）、`TCH_SHEET`（1）、
     `TCH_ERRORMSG`（1）；
   - 合法但无可见几何的记录进 `vendor.tch.no_display`（真实样本：26 条 `TCH_ARROW`）。
   覆盖证明：`decoded + unsupported + no_display == 记录数`，否则追加
   `vendor.tch.unaccounted`；诊断上限 16 条，超出的稳定 reason 会被截断（帧契约如此）。
   `TCH_MTEXTNEW` 显式不支持；TCH 文字的 `_TCH` 宽度因子/中西文分段/上标规则未复刻，
   只按宿主字体引擎的 MTEXT 排版成形。宿主没有可用 fallback 字体时，文字 primitive
   不产生可绘制片段（不会伪造字形）。
2. **派生表示范围**：`VendorRepresentationStore` 按 `source_handle` 索引，在模型空间顶层
   实体的 `build_expanded` 之后追加片段；**块定义内的 TCH 成员**由
   `append_block_vendor_fragments` 沿 INSERT/Compound 递归合并（组合变换
   `T(origin)·R·S·T(-base)`，深度/环路由 `MAX_INSTANCE_DEPTH` 限制），片段改挂到成员实体
   并带真实实例路径。派生片段带源实体 id、`GeometrySource::ProxyCache`、
   `Precision::Approximate`，因此拾取仍指向原 `TCH_*` 实体；块图元的子片段也强制挂回源实体。
   纸张空间（`build_paper_space`）目前不调用该 walk。
3. **插件隔离（仍未闭合）**：当前是普通 module script，和页面同权；外部插件在安装校验时
   （§8）也会在页面里执行一次。不可信插件应改为 Worker/wasm 或 sandboxed iframe。安装界面
   明示“插件拥有与应用其余部分相同的权限”，但**没有**签名、权限清单或进程隔离；`id/name/version`
   元数据只用于显示，宿主只记录字节，不做来源证明。
4. **真实格式版本**：本实现按真实样本的 base 6 / 保护标记 9 逆向；R2000 代理样本的
   `0x0e` payload、T30 基类 9–14 仍未在插件中实现。
5. **文本流共享块**：AC1027 文本流在属性包字符串之后有一个
   `TDBXDATAMAP_BLOCK_BEGIN_ … _END_` 块。所有解码器必须用 `base6.ts` 的 `ClassText`
   读类字符串；直接 `readText` 会把 sentinel 当成图面文字（曾发生，真实帧 751 条）。
   端到端断言：结果帧中不含 `TDBXDATAMAP` 文字。
6. **实测布局与参考实现（MiniCAD）的已知偏差**（都按真实样本位精确验证，不是猜测）：
   - `TCH_BLOCK_INSERT`：`AcDbBlockReference` 前缀 + version/mode 之后还有 94 bit 的
     TCH base-6 尾（base 6、scale 150、空属性包）；MiniCAD 用 `PositionInBits() > MainBits`
     容忍，我们显式消费该尾再要求精确结束；无尾的合成记录同样接受。
   - `TCH_RECTSTAIR`：真实记录 body version 为 **5**（MiniCAD 只收 4），在 4 的字段后
     多一个未使用的 `BitShort`；其他版本显式拒绝。
   - `TCH_OPENING` 组合（kind 8）：真实第 5 组使用 **87 bit** 内嵌头（37 bit 样本 + 50 bit
     中间段），两种观测头都按位严格匹配，其他位型拒绝。
   - `TCH_DIMENSION2`：4 条真实记录 `spanCount == 0`（MiniCAD 拒绝 `<1`）；接受并只画
     起止引出线，否则覆盖率停在 169/173。
   - `TCH_ARROW`：26 条记录位布局合法但顶点 <2 或已闭合，MiniCAD 也不画 → `[]` +
     `vendor.tch.no_display`，不是失败。
   - 尺寸样式：ABI 不携带图纸标注样式，统一用 MiniCAD 默认 `TchDimStyle` 并报
     `vendor.tch.dimension_style_defaults`。

## 7. 测试

- Rust：`crates/cad-ui-core/src/session/vendor.rs` + `vendor_tests.rs` 的 ABI v1 帧/token/
  `applied`/primitive/诊断上限测试；`crates/cad-desktop-shell/src/events.rs` 的 staged-once
  与 settled-once 测试；`apps/app-web/src/vendor.rs` 的 wasm 侧 stage/settle/清空测试；
  `crates/cad-app/src/render_scene/vendor_tests.rs` 的源实体合并 + 块内成员 + 派生面积测试；
  `crates/cad-app/src/host/tests/properties.rs` 的 `vendor_*` 属性行；
  `crates/cad-import-acadrust/src/vendor.rs` 的 `YACR_TEST_DWG` 抽取测试。
- 前端：`apps/desktop-ui/test/vendor-plugin.test.ts`（ABI v1 帧、墙/柱/文字/门窗、块图元、
  洞口侧线、弧墙/profile 诊断、截断/坏 magic、bulge→arc），以及每个新类的合成契约测试
  （annotation / dimension / stair / opening-symbols / block-insert / break-line /
  annotation-symbol / index-symbol / datamap）；`web-adapter.test.ts`（wasm 命令转发/拒绝）。
- 真实样本：`test-tangent-real.dwg` → plugin `status=0, applied=14911, entities=9889`，
  TEXT 870 / MTEXT 339 / WALL 4525 / COLUMN 1000 / OPENING 1734 / BLOCK_INSERT 220 /
  ELEVATION 358 / DIMENSION2 173 / ARROW 138 / MULTILEADER 127 / LINESTAIR 116 /
  RUPTURE 140 / RECTSTAIR 54 / MODI 44 / AXIS_LABEL 20 / RADIUSDIM 11 / 其余小类；
  kind 5 block 1,946（1,726 门窗 + 220 图块插入）。Rust 解析同一 result 帧并断言
  `applied == primitives`（14,911）。40 个 `TCH_BLOCK_INSERT` 块句柄全部由导入 DB 的
  `block_by_source_handle` 解析到有成员的块定义（探针实测 40/40）；
  真实 2D 块（`0x3DD82`、`0x3E6F4`）同理由定向 `#[ignore]` 测试覆盖。
- 构建：`npm run build:plugins` → `scripts/build-vendor-plugin.sh` 产出 `plugins/tch/dist/index.js`
  并复制到 `public/plugins/tch.js`；`npm test` 同时 typecheck 插件。

## 8. 插件管理器（2026-11-05，仅 `native-below`）

### 8.1 范围与入口

Options 对话框新增 **Plugins / 插件** 页签；File 选项卡的 **Plugins** 项可直接打开该页签。
它列出**内置 `tch` 解码器**和用户安装的解码器，
并在同一页显示上一轮 `vendor-settled`（应用对象数 + 诊断）与插件扫描诊断。实现位置：

- `apps/desktop-ui/src/components/OptionsPlugins.tsx`：页签 UI（即时提交，不参与其它 Options
  页的 draft/Apply 模型）；
- `apps/desktop-ui/src/options/plugins-view.ts`：纯视图模型（行状态、动作可用性、文件预检、
  `vendor-settled` 摘要）；
- `apps/desktop-ui/src/plugins/plugin-manager.ts`：前端 wire 调用、Blob 导入校验、安装流程；
- `crates/cad-platform/src/plugins.rs`：`PluginStore` 持久化契约；
- `crates/cad-ui-core/src/session/plugins.rs`：纯 wire 模型/策略/manifest；
- `crates/cad-desktop-shell/src/plugins.rs`：五个 Tauri 命令与启动时解码器选择。

插件管理器管理的是**离线解码器 bundle**，不是 CAD 文档状态：它不实例化第二份 `cad-db`，
不改变当前会话的文档，也不把显示表示当数据源。

### 8.2 单解码器规则（radio 语义）

- **同一时间只有一个 active decoder**。策略文件里只有一个 `activeDecoder` 字段
  （`"tch" | "<id>" | null`）；`null` 表示“全部禁用”。
- 没有任何策略文件时（首次运行），内置 `tch` 是默认 active。
- 安装一个用户插件会把它写成 active；启用另一个会替换当前 active；禁用**当前 active** 会把
  策略写成 `null`；禁用非 active 的插件是幂等 no-op（仍返回 `accepted:true`）。
- `tch` 是保留 id：不能安装同名插件，不能卸载内置插件，`cad_plugin_read_entry("tch")` 拒绝
  （内置 bundle 由应用自己的 URL 提供）。内置 `tch` **可以**被禁用（下次启动厂商解析关闭）。
- 每行有两个独立事实：`enabled`（已保存的选择）与 `sessionActive`（本进程启动时实际加载的
  解码器）。两者不一致就是“需要重启”，不由计时器或刚发出的请求猜测。

### 8.3 持久化布局

`<config-dir>` 与 `config.json`/`preference.json` 同一目录：

```
<config-dir>/plugins.json                         # 策略
<config-dir>/plugins/<id>/plugin.js               # 解码器 bundle（入口固定为 plugin.js）
<config-dir>/plugins/<id>/manifest.json           # 宿主合成/校验的 manifest
```

策略（`version` 必须为 `1`；缺 `activeDecoder` 键是非法策略，不是静默禁用）：

```json
{"version":1,"activeDecoder":"tch"}
```

`activeDecoder` 也可以是用户插件 id 或 `null`。manifest 字段：
`manifestVersion`、`id`、`name`、`version`、`abi`、`sourceFilename`、`entry`、`installedAt`
（UTC RFC 3339）；`manifestVersion`/`abi` 只接受 `1`，`entry` 只接受 `plugin.js`，`id` 必须与
目录名一致。`installedAt` 由宿主按系统时钟生成。

- **id 语法**：`[a-z0-9][a-z0-9_-]{0,63}`。前端校验通过后，模块注册对象上的 `id` 若存在且合法，
  就是目录 id；缺失时才由文件名派生（`My Tools.js` → `my-tools`；`.js`/`.mjs` 大小写不敏感）。
  保留 id `tch` 与已存在的目录都显式拒绝（`plugin.reserved_id` / `plugin.already_installed`），
  不覆盖、不静默改名。
- **写入**：插件目录用同文件系统的临时目录 + `rename` 原子落盘；策略文件同样临时文件 +
  `rename`。安装后若策略写入失败，宿主删除刚写入的插件目录，避免“已安装但无法激活”的半成品。
- **路径安全**：id 先过上面的语法；插件目录或入口/manifest 是 symlink 时拒绝
  （`plugin.unsafe_path`）；越界 id（`..`、`a/b`）拒绝；入口读取与写入都有 4 MiB 上限。
- **扫描**：插件根不存在是空列表；不可读目录是 `plugin.scan_failed` 诊断，不是空成功。
  manifest 缺失/非法仍以行显示（`manifest_missing`/`manifest_invalid`），并保留稳定诊断。
- 策略文件损坏时，状态投影回退到内置默认并附 `plugin.policy_invalid` 诊断；**写操作拒绝**，
  不覆盖一个看不懂的策略。

### 8.4 安装流程

1. 页面上的隐藏 `<input type=file>`（由可见的“安装插件…”按钮驱动）接受 `.js`/`.mjs`；
   扩展名与 **4 MiB（4,194,304 B）** 上限在读取文件内容**之前**预检。
2. 前端把源码放进 `Blob`，用 Blob URL 以 ES module 导入**一次**做 ABI 校验：
   `globalThis.__YACR_VENDOR_PLUGIN__.abiVersion === 1` 且 `decode` 是函数。
   导入前后保存/恢复该全局，校验不会替换正在使用的解码器；Blob URL 无论成败都撤销。
   校验失败给稳定原因 `plugin.contract_invalid` / `plugin.abi_mismatch: <n>`。
3. 校验通过后调用 `cad_plugin_install({ filename, payload, meta? })`，`payload` 是文件原始字节
   的 base64（不是文本重编码）。宿主重新检查文件名/扩展名、base64、大小和保留 id，然后写
   `<id>/plugin.js` + `manifest.json`，并把 `activeDecoder` 设为新 id。
4. 变更立即持久化；**当前会话不热加载**。成功安装只意味着“下次启动会用这个解码器”，页面
   通过重新读取 `cad_plugin_status` 显示新行和 `restartRequired`。

**信任警告（UI 明示）**：安装校验会先运行一次插件代码；插件拥有与应用其余部分相同的权限，
因此只应安装可信文件。宿主只记录字节，没有签名验证；`name/version` 是显示元数据，
`id` 是安装身份（宿主校验语法与保留/占用，但无法从字节重新验证它）。

### 8.5 重启语义（无热重载、无热卸载）

- 安装 / 启用 / 禁用 / 卸载都**立即写策略**，但**只在下次应用启动时生效**。没有 hot reload、
  没有 live unload、没有自动重启；当前进程继续使用启动时加载的解码器。
- `restartRequired` 由宿主计算为 `activeDecoder != sessionActiveDecoder`；页签顶部据此显示
  “插件改动将在下次启动后生效”横幅。`accepted:true` 只表示宿主已保存，不表示已生效。
- 下次启动读取策略：`tch` → 内置 bundle；其他 id → 校验 manifest 后通过
  `cad_plugin_read_entry` 读入口并用 Blob URL 导入；`null` 或配置的 id 已不存在 → 不启用解码器，
  厂商解析以 `vendor.decoder_disabled` 形成真实的 unsupported 结果（不挂起、不伪造成功）。
- 卸载是真正的目录删除（不是隐藏/禁用）：删除 `<config-dir>/plugins/<id>/`；若它正是 active，
  策略同时写为 `null`。内置插件没有卸载路径。

### 8.6 命令（五个，native shell only）

这些是 **shell IPC 命令**，不是 CAD 选择子，不进入 `COMMAND_TABLE`；同名表也记录在
`docs/commands.md`。线格式为 camelCase。

| 命令 | 参数 | 返回 |
|---|---|---|
| `cad_plugin_status` | — | `{ supported, carrier, plugins[], activeDecoder, sessionActiveDecoder, restartRequired, diagnostics[] }` |
| `cad_plugin_install` | `{ filename: string, payload: string(base64), meta?: { id?, name?, version? } }` | `PluginMutationResult` |
| `cad_plugin_enable` | `{ id: string, enabled: boolean }` | `PluginMutationResult` |
| `cad_plugin_uninstall` | `{ id: string }` | `PluginMutationResult` |
| `cad_plugin_read_entry` | `{ id: string }` | `{ payload: string(base64), entry: "plugin.js" }`；内置 id 拒绝 |

`PluginMutationResult` = `{ accepted: boolean, reason?: string, restartRequired: boolean,
status: PluginStatusFrame }`。拒绝不是异常也不是空成功：它带 `accepted:false`、稳定 `reason`
和**当前真实**的 registry 快照。`PluginRow` = `{ id, name, version, abi, source:
"builtin"|"user", enabled, sessionActive, status: "ok"|"manifest_missing"|"manifest_invalid",
installedAt? }`。

页签把行投影为四种状态：`active`、`pendingRestart`、`disabled`、`invalid`（任何非 `ok` 的
宿主状态）。只有 `user` 行有卸载按钮；只有 `enabled` 行有禁用按钮。

### 8.7 载体支持与拒绝

| 载体 | 插件管理器 | 存储 / 行为 |
|---|---|---|
| Linux 原生 Wayland（`native-below`） | **支持** | `$XDG_CONFIG_HOME/yacr`（未设时 `~/.config/yacr`）下的 `plugins.json` + `plugins/` |
| Linux X11/XWayland（`dom-canvas`） | **不支持** | `cad_plugin_status` 返回 `supported:false`、`carrier:"dom-canvas"`、诊断 `plugin.unsupported_carrier: dom-canvas`；安装/启用/卸载返回 `accepted:false`（同一 reason），`read_entry` 返回同一 reason 的错误。内置 `tch` 在 wasm 会话中仍然 always-on |
| Windows（`native-below`） | **契约支持** | `%APPDATA%\yacr\plugins.json` + `%APPDATA%\yacr\plugins\<id>\`；`APPDATA` 不可解析时用 `NonPersistentPluginStore`：`tch` 仍可用，安装/写入以 `unsupported-runtime: plugin-store-unavailable` 拒绝 |
| macOS（`native-below`） | **契约支持** | `$HOME/Library/Application Support/yacr/plugins.json` + `plugins/<id>/`；`HOME` 不可解析时同上 |
| Android（`native-below`） | **契约支持** | 应用私有 `filesDir` 下的 `plugins.json` + `plugins/<id>/`（当前包名 `/data/user/0/io.github.zenglanmu.yacr/files`）；路径在 `nativeAssetsReady` 后惰性解析，握手前返回 `unsupported-runtime: plugin-store-unavailable` |
| Web（`dom-canvas`） | **不支持（无持久存储）** | `web-adapter` 对五个名字逐个抛 `unsupported-runtime: cad_plugin_<name>`；内置 `tch` 仍由打包的 `plugins/tch.js` 提供 |

`cad-desktop-shell` 在 `native-below` 上注册五个命令；Linux 的 `dom-canvas` 仍把命令路由到
native handler，由 handler 给出上述**线格式拒绝**（避免 Tauri “command not found”）。纯 Web
没有 Tauri bridge，由前端 adapter 按名拒绝。

### 8.8 CSP 与安全取舍

四个 native `tauri.conf.json`（Linux / Windows / macOS / Android）的 `script-src` 增加了
`blob:`，用于导入用户安装的 ES module；Web 的 CSP 不变。**这是一个明确的取舍**：`blob:` 让
所有 native 用户页面的 CSP 对脚本来源更宽松。之所以接受，是因为安装流程本来就要在页面里
执行任意插件代码，且没有 worker/sandbox 隔离；它不使插件变得可信，只是让“已安装插件”的
加载路径成立。

仍未闭合：插件隔离（普通 module script，与应用同权）、签名/权限清单、热重载/热卸载。
内置 `tch` 仍是打包文件；外部插件只有 `native-below` 能安装。

### 8.9 证据（实现轮次，2026-11-05）

- `npm test`：**1096/1096 pass**（含 `plugin-manager.test.ts`、`options-plugins.test.ts`、
  `web-adapter.test.ts` 的按名拒绝、`carrier-routing`/`host-adapter` 的 shell-only 路由）。
- `cargo test -p cad-ui-core -p cad-platform -p cad-desktop-shell --locked`：通过（wire 模型/
  manifest/policy、`FsPluginStore` 原子写/越界/symlink/超限、五个命令的 install→status→read→
  uninstall、dom-canvas 拒绝、restart 语义）。
- `cargo check -p app-linux --all-targets --locked`：通过。
- `python3 scripts/check-architecture.py`、`scripts/check-file-size.py`、`scripts/check-i18n.py`：
  通过（本条按实现轮次记录）。
- **NOT RUN / NOT VERIFIED**：Windows/macOS/Android 真实目标运行、Android 设备、真实 GUI
  （Options → Plugins 的实际交互/像素）、真实浏览器、GPU/像素、CSP 在真机 WebView 的行为。
  本轮文档改动只重跑了 `check-command-docs.py`、`check-i18n.py` 与 `git diff --check`。
