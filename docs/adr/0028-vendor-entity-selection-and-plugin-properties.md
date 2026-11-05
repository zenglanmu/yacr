# ADR 0028：天正图元拾取、handle 语义与插件属性/命令提供者

- 状态：**Accepted（2026-11-05，实现中）**。验证边界：Linux 契约/单元测试；Windows/macOS/Android/Web
  仍只到编译或契约级，真实 GPU/真机/像素 **NOT VERIFIED**。
- 日期：2026-11-05。
- 相关：[ADR-0024](0024-dom-canvas-wasm-cad-path.md)（`dom-canvas` 载体）、
  [ADR-0027](0027-android-native-below-carrier.md)（`native-below` 载体）、`docs/vendor-plugin.md`
  （现有 ABI v1 vendor pass）。
- 影响：`cad-app` 拾取目录、`cad-host` 点击/hover、`apps/desktop-ui` 属性 dock 与命令漏斗、
  `plugins/tch` 插件。**不**改变：`cad-db` 是唯一权威、vendor 派生表示不落库、单 React 树、
  `native-below`/`dom-canvas` 的文档所有权规则。

---

## 1. 问题

1. `TCH_*` / 无法解码的代理图元在 `cad-db` 里是 `SemanticGeometry::Opaque`。场景装配会把插件解出的
   派生 primitive 挂到源实体 id 上，所以它们**看得见**；但 CPU 拾取目录只读 `cad-db` 几何，
   `Opaque` 在精细拾取里返回 `PickOutcome::Unsupported("opaque")`，因此**点不中**。
2. 插件目前只有一个同步入口 `decode(records) -> result`，没有“启动时注册监听”的通道；属性命令
   `PROPERTIES` 是 chrome-local selector，从不经过 Rust 命令/事件，也没有插件拦截点。
3. 用户需要：天正图元可点击选中；选中后属性命令可由插件接管并显示天正自定义属性；插件可拒绝
   暂不支持的编辑操作（新增/移动/复制/删除等），未接管时继续走原属性面板。

## 2. 决策

### D1 可选中性以 `cad-db` 提交实体为准，不以 source handle 为准

- **可选中**：已经提交进 `cad-db` 的实体，身份是 `SelectionRef { document, entity: EntityId,
  instance, sub_element }`。`EntityId` 是唯一选择键。
- **不可选中**：transient overlay（绘制/测量预览、框选、捕捉提示、grip marker、光标）只携带
  `preview_source_id` 的合成哈希 id，**没有**真实 `EntityId`、**没有** handle；
  `DrawingTransaction` 里 staged 但未 commit 的实体不在 `cad-db`，也不可选中。
- `DbObject::source_handle: Option<String>` 是**来源出处**，不是选择身份：
  - 从 DWG/DXF 导入的实体有 handle（大写十六进制）；
  - API/编辑/COPY 创建并提交的实体有 `EntityId`，但 `source_handle = None`（属性面板显示
    `handle = none`，不伪造）；
  - 因此“带 handle 的都能选中”成立，但反过来“没有 handle 就不可选中”**不成立**——只要已提交
    到 `cad-db` 就可选中。这与 AutoCAD 的“已加入数据库即可选择、transient 不可选择”行为一致，
    只是本项目的 handle 字段额外承担“源文件出处”语义。
- 约束：不得为 transient/staged 实体伪造 handle 或 `EntityId`；不得把 GPU pick key 当持久身份。

### D2 vendor 派生几何进入拾取目录（替换 `Opaque`，不是写库）

- 当实体几何是 `SemanticGeometry::Opaque`，且 `VendorRepresentationStore` 按 `source_handle`
  能给出派生 primitive 时，拾取目录为这个实体提供一份**派生的 analytic 几何**
  （`SemanticGeometry::Compound`），`source` 仍是原实体的 `SelectionRef`，`transform` 仍是
  INSERT 展开的累计变换，`geometry_source = GeometrySource::ProxyCache`。
- 映射：`Lines` → `Polyline`；`Segments` → 多条 `Line`；`Arc`/`Circle` → 对应 analytic 变体；
  `Text` → 其 origin 的 `Point`（只保证锚点可点，不伪造字形轮廓）；`Block` 暂不展开
  （该 TCH 实体若只有 block primitive 则仍不可点，必须显式记录为限制，不得假装可点）。
- 没有 vendor 快照 / 没有派生 primitive 的 `Opaque` 保持原行为：精细拾取 `Unsupported("opaque")`，
  并在 `PickReport.skipped` 里保留原因。
- 图层可见性仍按源实体判断；`SELECTALL`、框选、点击、hover 使用**同一**目录。
- `PickIndex` 缓存键必须包含 vendor 快照 revision（当前只含 `SceneIdentity` + document，vendor
  快照变化不改 DB revision，必须补上），否则插件 settle 后仍会命中旧索引。

### D3 插件提供者注册（ABI v1 可选扩展，不改二进制帧）

- `plugins/tch` 的 `abiVersion` 保持 **1**；records/result 帧布局不变。
- 新增可选导出：
  ```ts
  export function register(host: VendorPluginHost): void;
  ```
  宿主在启动时加载模块并调用一次。`register` 抛错/缺失都只是“没有提供者”，不得阻塞应用。
- 宿主 API（插件侧只依赖这个结构，不导入宿主源码）：
  ```ts
  interface VendorPluginHost {
    on(event: 'properties', handler: (selection: PluginSelection) => PluginPropertyPanel | null): void;
    on(event: 'command', handler: (request: PluginCommandRequest) => PluginCommandDecision | null): void;
    on(event: 'cad-status', handler: (status: PluginCadStatus) => void): void;
  }
  ```
- 事件语义：
  - `cad-status`：每次 `cad_event` 的 `status-changed` 快照（含 `selection`）。插件可缓存；
  - `properties`：用户执行 `PROPERTIES` 时由 chrome 调用，传入**当前** Rust 状态里的
    `selection`；返回 `null` = 不接管，继续默认属性面板；返回 panel descriptor = 接管；
  - `command`：命令漏斗在准入后、发送 `cad_command` 前调用；返回 `null` = 放行；
    返回 `{ handled: true, messageKey }` = 拦截并显示模态说明。
- 多个提供者按注册顺序询问，第一个 `handled` 生效；插件异常被宿主捕获并视为“不处理”，
  不得静默变成成功。
- 当前限制：`cad_status.selection` 是多选**交集行**，不含逐个实体列表；因此 `properties` 与
  `command` 的插件判定只在 `selection.count == 1` 且 `handle` 行非 mixed 时可靠。多选一律
  pass-through（显式限制，后续如需多选接管要先扩展 wire，不在本 ADR 范围）。

### D4 属性命令：复用现有 `cad_event`/`status-changed`，不新增 Rust 命令

- Rust 已经在每次 flush 推送 `cad_event { type: 'status-changed', status: { selection, ... } }`。
  本决策把这条既有状态流作为插件的“cad-status 事件”来源。
- `PROPERTIES` 仍是 chrome-local selector（不产生 `cad_command`、不写 DB）。chrome 在打开属性 dock
  前询问已注册的 `properties` 提供者：
  - 返回 panel → 在属性 dock 中渲染**宿主渲染的**插件面板（插件只给分组只读 rows，不操作 DOM）；
  - 返回 `null`/插件未就绪 → 打开原有 `PropertiesPalette`。
- 属性 dock 的归属不变：`native-below` 由 `palettesAvailable` 门控；`dom-canvas` 无宿主文档状态时
  不出现。插件输出是 display-only，不进入 `cad-db`、不参与保存/导出。
- 面板 row key 使用稳定机器键（如 `wall.height`），宿主按 `properties.tch.<key>` 翻译；未知键显示
  原键（与默认面板的 fallback 一致）。插件不得硬编码中文文案。

### D5 编辑拦截：只在当前选择含天正实体时拒绝

- 命令漏斗在 `cad_command` 之前询问 `command` 提供者。天正插件对
  `MOVE/COPY/ERASE/ROTATE/SCALE/MIRROR/STRETCH/TRIM/EXTEND/OFFSET/EXPLODE` 等编辑命令，
  且当前单选实体在插件 records 缓存中命中时，返回
  `{ handled: true, messageKey: 'vendor.editing.unsupported' }`。
- 普通图元编辑、以及未选中天正实体时的命令**不受影响**。普通绘制命令（`LINE` 等）不拦截：
  本应用没有“创建 TCH 实体”的命令，不能把普通绘图误判为天正编辑。
- 拦截只发生在 chrome 漏斗；Rust 命令仍可被其他入口直接调用。该边界必须写入文档，不能声称
  插件是安全边界。

### D6 权威与隔离

- 插件仍是同权 module script（`docs/vendor-plugin.md` §6.3 的已知缺口），本 ADR 不宣称沙箱。
- `cad-db` 唯一权威；vendor primitive、插件属性行、插件面板都是派生显示。插件不能写 DB、
  不能改选择、不能伪造 handle/EntityId。
- 跨载体：所有 React 宿主共用同一 `apps/desktop-ui` 与 `cad_event`；`native-below`/`dom-canvas`
  都通过各自 Rust 会话的 `status.selection` 驱动，不按构建目标分叉。

## 3. 数据契约

```ts
// 宿主 → 插件（结构类型，插件不 import 宿主源码）
interface PluginSelectionRow {
  key: string; value: string | null; group: string;
  mixed: boolean; editable: boolean; kind: string; options: string[];
}
interface PluginSelection {
  count: number; empty: boolean; rows: PluginSelectionRow[];
  // handle 由 rows 里的 "handle" 行给出；多选时通常 mixed。
}

// 插件 → 宿主
interface PluginPropertyPanel {
  kind: string;      // TCH_WALL / TCH_COLUMN / ...
  handle: string;    // uppercase hex, source provenance
  groups: Array<{
    key: string;     // identity | geometry | text | ...
    rows: Array<{ key: string; value: string }>; // properties.tch.<key>
  }>;
}
interface PluginCommandRequest { command: string; selection: PluginSelection }
type PluginCommandDecision = { handled: true; messageKey: string } | null;
```

`PluginPropertyPanel` 由宿主渲染；插件不提供任意 HTML/脚本。`messageKey` 必须存在于
`crates/cad-ui-core/i18n/{zh-CN,en}.json`。

## 4. 失败闭合

- 插件未加载/无 `register`/handler 抛错：`properties` 返回 `null`，默认面板；`command` 放行。
  错误写诊断（chrome banner），不伪造“已处理”。
- 插件返回未知 `messageKey`：宿主显示稳定的通用 key，并保留原 key 作为诊断；不静默吞掉。
- 插件对多选返回 panel：宿主拒绝并在诊断中记录 `vendor.plugin.multi_selection_unsupported`。
- vendor 派生几何为空/只有 block primitive：`Opaque` 仍不可点，`skipped` 保留 `opaque`；
  `SELECTALL`/框选不得把不可点实体算作“已支持”。
- 任何情况下都不得把插件结果写入 `cad-db`、不得修改 `source_handle`、不得生成第二份文档状态。

## 5. 测试与证据

- Rust：`cad-app` 拾取测试覆盖
  - 已提交实体可选中；staged 事务实体不在 DB/目录；API 创建实体 `source_handle=None`；
  - `Opaque` + vendor store 可点，无 store 时 `skipped = opaque`；
  - vendor revision 变化使 `PickIndex` 失效；
  - `SELECTALL`/框选包含 vendor 派生实体。
- 前端：插件 `register`/`properties`/`command` 的宿主单测；无插件/异常/多选 pass-through；
  `cad-command` 的 `blocked` 结果；面板 row 翻译；i18n 键集一致。
- 插件：合成 records 帧 → `decode` → `properties` 断言墙/柱/文字/洞口行；真实样本只在有
  `YACR_TEST_DWG` 时运行，跳过不算证据。
- 手工：Linux 真实 GUI 打开天正样本，点击墙/柱/文字，执行 `PROPERTIES`，确认插件面板；
  选中天正后执行 `MOVE`/`ERASE`，确认模态拒绝；未选中天正时普通编辑不受影响。
  **本 ADR 不把 Windows/macOS/Android/Web 的编译或契约结论写成运行验收。**
