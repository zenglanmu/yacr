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
- 宿主用 `<script type="module" src="plugins/tch.js">` 加载（与 canvas bootstrap 同一模式），
  插件把唯一门面注册到 `globalThis.__YACR_VENDOR_PLUGIN__ = { abiVersion, decode }`。
- 插件只做 **字节 → 图元**；它不读写 `cad-db`、不调用 Tauri `invoke`、不碰文件系统。
- **可选注册扩展（ABI 仍为 1，二进制帧不变；ADR 0028）**：模块可导出
  `register(host)`，宿主在启动时调用一次；插件用
  `host.on('properties' | 'command' | 'cad-status', handler)` 注册。未导出/注册抛错只失去
  属性面板与编辑拦截，`decode` 不受影响。插件把 `decode` 解析出的 `VendorRecord[]` 保存在模块作用域
  （≤64 MiB records frame；流是同一 `ArrayBuffer` 的视图，不复制载荷），按 `source_handle`
  回答属性/命令询问。插件输出仍是 display-only，不写 `cad-db`。
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

## 5.2 拾取/选择（ADR 0028，2026-11-05）

`TCH_*` 在 `cad-db` 里仍是 `Opaque`；插件解出的 primitive 不落库。为了让“看得见”与“点得中”一致，
CPU 拾取目录在 **存在已 settle 的 vendor 快照** 时，把 `Opaque` 实体替换为一份**派生 analytic 几何**
（`SemanticGeometry::Compound`），但 `source` 仍是原实体的 `SelectionRef`：

- `Lines` → 开放 `Polyline`；`Segments` → 多条 `Line`；`Arc`/`Circle` → 对应 analytic 变体；
  `Text` → 其 origin 的 `Point`（只保证锚点可点，不伪造字形轮廓）；
  `Block` **不展开**（只有 block primitive 的 TCH 实体仍不可点，保留 `skipped = opaque`）。
- INSERT 内部成员沿用既有累计实例变换；图层可见性按源实体判断；`geometry_source = ProxyCache`。
- 点击、hover、`SELECTALL`、框选共用同一目录；`PickIndex` 缓存键包含 vendor 快照 revision，
  插件 settle 后索引会失效重建。
- **没有 vendor 快照 / 没有派生 primitive** 的 `Opaque` 行为不变：精细拾取
  `PickOutcome::Unsupported("opaque")`，并在 `PickReport.skipped` 里保留原因。不得把不可点实体
  算作已支持，也不得为其伪造 `EntityId`/handle。

> 注意：导入期的 `EntityCapability.pick` 对 `Opaque` 仍是 `Unsupported`（那是按 class 的静态
> 能力，且在 vendor pass 之前就已确定）；运行时可点性由 settle 后的 vendor 快照决定。两者不要
> 混写：能力报告仍说“该 class 的语义/拾取未建模”，交互拾取只是对“有派生表示”的实例生效。

**handle 语义**：可选中性以 **已提交进 `cad-db` 的实体** 为准，不以 source handle 为准。导入实体有
handle；API/编辑/COPY 创建并提交的实体有 `EntityId` 但 `source_handle = None`（属性面板显示
`handle = none`）。transient overlay（绘制/测量预览、框选、捕捉提示、grip marker、光标）只有合成
哈希 id，**没有**真实 `EntityId` 或 handle；`DrawingTransaction` 中 staged 未 commit 的实体不在
`cad-db`，也不可选中。这与 AutoCAD “已加入数据库即可选择、transient 不可选择”的行为一致，只是本项目
的 handle 额外承担“源文件出处”语义。

## 5.3 插件属性/命令提供者（ADR 0028，2026-11-05）

- `PROPERTIES` 仍是 chrome-local selector（不触发 `cad_command`、不写 DB）。chrome 在打开属性 dock 前
  询问已注册的 `properties` 提供者，传入**当前** `cad_event`/`status-changed` 的 `selection`：
  - 返回 panel descriptor → 在属性 dock 中显示宿主渲染的只读天正面板（插件只给分组 rows，不操作 DOM）；
  - 返回 `null` / 插件未就绪 / handler 抛错 → 继续原有 `PropertiesPalette`。
- panel 形状：`{ kind, handle, groups: [{ key, rows: [{ key, value }] }] }`。row key 是稳定机器键，
  宿主按 `properties.tch.<key>` 翻译；未知键显示原键。插件不硬编码中文。
- `command` 提供者在命令准入后、`cad_command` 之前被询问；天正插件对当前单选（且 handle 命中
  records 缓存）的 `MOVE/COPY/ERASE/ROTATE/SCALE/MIRROR/STRETCH/TRIM/EXTEND/OFFSET/EXPLODE/ARRAY/
  PEDIT/FILLET/CHAMFER` 返回 `{ handled: true, messageKey: 'vendor.editing.unsupported' }`，
  chrome 显示模态说明并**不发送** `cad_command`。普通图元编辑、未选中天正实体、普通绘制命令不受影响。
- **多选限制**：`cad_status.selection` 是多选**交集行**，没有逐实体列表；插件只对
  `count == 1` 且 `handle` 行非 mixed 的选择可靠接管。多选一律 pass-through；如需多选接管，必须先
  扩展 wire，不在当前范围。
- 插件仍是同权 module script（§6.3）；命令拦截只发生在 chrome 漏斗，**不是**安全边界；Rust 命令仍可被
  其他入口直接调用。

## 6. 已知缺口

0. **载体验证边界**：命令与事件在全部五个宿主上已按同一契约接线；但只有 Linux 做了运行/
   契约测试。Windows/macOS/Android 为编译证据，Web 为 wasm 编译 + adapter/单测证据。
   **worker（OffscreenCanvas）隔离载体没有页面侧的 vendor 通道**：该 carrier 的 wasm 实例
   在 Worker 内，页面 adapter 不访问它；这一限制必须在状态/文档中保持显式。
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
3. **插件隔离**：当前是普通 module script，和页面同权；不可信插件应改为 Worker/wasm
   或 sandboxed iframe。
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

7. **属性/命令接管边界（ADR 0028）**：多选没有逐实体列表，插件只接管 `count == 1` 且
   `handle` 行非 mixed 的选择；只有 block primitive 的 TCH 实体仍不可点；`Text` 只按 origin
   锚点可点，不伪造字形轮廓。插件 records 缓存随每次 `decode` 替换，解码失败即清空，因此 settle 前
   选中 TCH 只会走默认面板/放行命令。
8. **插件面板不是自由 UI**：插件只返回 `{kind, handle, groups, rows}` 只读 descriptor，由 React
   宿主渲染；插件不能注入 HTML/脚本、不能改选择、不能写 DB。面板 row 文案由宿主 i18n 提供，插件只给
   稳定的 `properties.tch.*` 机器键。

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
- 前端（ADR 0028）：`vendor-plugin-properties.test.ts`（合成 TCH_WALL records → `decode` →
  `register` → 属性行/未知 handle/多选/编辑命令接管）；`plugin-interop.test.ts`（提供者顺序、
  null 清理、异常放行、非法 panel/多选拒绝、notice store）；`plugin-command.test.ts`（`blocked`
  在 IPC 前返回、null 放行、chrome-local 不拦截、`blocked` 文案）。
- Rust 拾取（ADR 0028）：`crates/cad-app` 的拾取测试覆盖已提交/ staged/transient 身份、
  `Opaque` + vendor store 命中、无 store 时 `skipped = opaque`、vendor revision 使 `PickIndex`
  失效、`SELECTALL`/框选包含 vendor 派生实体。
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
