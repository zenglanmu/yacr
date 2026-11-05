# ADR 0029：厂商插件管理器（持久化后重启、单解码器、仅 `native-below`）

- 状态：**Accepted（2026-11-05）**；代码已落地。证据只到 **Linux 契约测试 + `app-linux`
  编译门禁**；Windows/macOS/Android 运行、Android 设备、真实 GUI/浏览器、GPU/像素全部
  **NOT VERIFIED / NOT RUN**（见第 6 节）。本文**不**宣称任何平台已完成产品验收。
- 日期：2026-11-05。
- 相关 ADR：[ADR-0010](0010-react-linux-host-replaces-slint.md)（React/Tauri 宿主）、
  [ADR-0023](0023-dom-on-top-cross-host-composition.md)（跨宿主合成；本文不改其合成/输入/单
  presenter 不变量）、[ADR-0024](0024-dom-canvas-wasm-cad-path.md)（`dom-canvas` 载体；本文新增的
  插件管理器**不在**该载体提供）、[ADR-0027](0027-android-native-below-carrier.md)（Android
  `native-below`；本文复用其共享命令注册表）。
- 影响：File 选项卡新增 **Plugins** 直达项，Options 新增 Plugins 页签；新增 `cad-platform::plugins::PluginStore`、
  `cad-ui-core::session::plugins` 纯模型、`cad-desktop-shell::plugins` 五个命令；四个 native
  `tauri.conf.json` 的 `script-src` 加入 `blob:`。**不**改变：单一 React 树、`cad-db` 单一权威、
  renderer-only wgpu、importer-only acadrust、`.native-input` 单一输入漏斗、弹窗模态围栏、
  无逐帧整幅回读。

---

## 1. 问题

厂商解码器（天正 `TCH_*`）此前只有**打包内置**的 `tch`：它随应用发布、只在 WebView 里以
`<script type="module">` 加载。用户无法安装另一个厂商的解码器，也无法停用内置解码器；没有
持久化策略、没有“当前启用的是谁”的可见状态，更没有跨启动一致的选择。

需要解决的是：

> **怎样让用户安装/切换/移除厂商解码器，同时不引入第二份文档权威、不热改当前会话、不在
> 不支持的载体上伪装成功？**

约束：

- 解码器是**不可信代码**，执行时与应用同权（现有内置插件也是如此）；
- 插件变更不能要求重启整个渲染/文档管线，但也不能在 pass 中途替换模块；
- `dom-canvas`（Linux X11/XWayland、Web）的 `cad-db` 权威在 wasm 侧，native 壳没有文档；
- 内置 `tch` 必须在没有插件目录、没有持久存储时仍能工作。

## 2. 决策

### D1 持久化后重启：立即写盘，下次启动生效

安装 / 启用 / 禁用 / 卸载都**立即**写 `<config-dir>/plugins.json`（以及插件目录），但**不修改
当前进程已加载的解码器**。状态返回
`restartRequired = activeDecoder != sessionActiveDecoder`，UI 只显示宿主返回的值。

- **没有热重载、没有 live unload**。当前会话继续使用启动时加载的模块；卸载只删除磁盘目录，
  不会从正在运行的 pass 中抽走模块。
- `accepted:true` 只表示宿主已保存，不表示已生效。
- 下次启动读取策略：`tch` → 内置 bundle；其他 id → 校验 manifest 后读入口并导入；`null` 或
  id 已不存在 → 不启用解码器，厂商 pass 以 `vendor.decoder_disabled` 形成真实的 unsupported
  结果，不挂起、不伪造几何。

**理由**：插件是 ES module，模块副作用与 `globalThis.__YACR_VENDOR_PLUGIN__` 是进程级的；
在 pass 中途替换会让 token/generation 与解码器版本错配。持久化后重启是最小、可解释的语义，
也避免“界面说换了，当前图还是旧解码器”的谎报。

### D2 唯一 active decoder（radio 语义）

策略只有一个字段：`activeDecoder: "tch" | "<id>" | null`。

- 没有策略文件 = 首次运行 = 内置 `tch` active。
- 安装一个用户插件会把它设为 active；启用另一个会替换；禁用**当前 active** 写 `null`；禁用
  非 active 是幂等 no-op。
- `null` 是合法的“全部禁用”，不是文件损坏。
- `tch` 是保留 id：不能安装/覆盖，不能卸载；`cad_plugin_read_entry("tch")` 拒绝，因为内置
  bundle 由应用自己的 URL 提供。

**理由**：vendor pass 只有一个 `__YACR_VENDOR_PLUGIN__` 门面和一个 result 帧契约。允许同时
启用多个解码器会立刻产生“谁解哪条记录/谁负责 applied 计数”的歧义；radio 语义把这一点在 UI
和 wire 上都固定下来。禁用 active 后仍保留 `null`，而不是偷偷回退到 `tch`——UI 的 Disable
必须真的关闭厂商 pass。

### D3 目录布局、真实卸载与路径安全

```
<config-dir>/plugins.json
<config-dir>/plugins/<id>/plugin.js
<config-dir>/plugins/<id>/manifest.json
```

- **策略**：`{"version":1,"activeDecoder":"tch"|<id>|null}`；`version` 只接受 `1`，缺少
  `activeDecoder` 键是非法策略（不是静默禁用）。损坏的策略在状态里附
  `plugin.policy_invalid` 并回退到内置默认；**变更拒绝**，不覆盖看不懂的文件。
- **manifest**：由宿主合成，字段 `manifestVersion/id/name/version/abi/sourceFilename/entry/
  installedAt`；`abi`/`manifestVersion` 只接受 `1`，`entry` 必须是 `plugin.js`，`id` 必须与
  目录名一致。目录 id 取前端校验后模块提供的 `id`（合法时），缺失才由文件名派生；保留 id 与
  已存在目录显式拒绝，不覆盖。
- **id 语法**：`[a-z0-9][a-z0-9_-]{0,63}`；文件名 `.js`/`.mjs`（大小写不敏感）可派生出
  `[a-z0-9_-]` 的 slug。
- **真实卸载**：删除 `<id>/` 目录；若它是 active，策略同时写 `null`。不是隐藏、不是禁用。
- **路径安全**：id 先过语法；插件目录或入口/manifest 是 symlink 时拒绝（`plugin.unsafe_path`）；
  入口读取/写入有 4 MiB 上限；缺失目录是空扫描，不可读目录是 `plugin.scan_failed` 诊断。
- **原子性**：插件目录用同文件系统的临时目录 + rename；策略文件用临时文件 + rename。安装后
  策略写入失败时删除刚写入的目录，避免“已安装但无法激活”的半成品。

### D4 仅 `native-below`；`dom-canvas`/Web 显式拒绝

- `native-below`（Linux Wayland、Windows、macOS、Android）：`cad-desktop-shell` 注册五个命令。
- Linux X11/XWayland 的 `dom-canvas`：`cad_plugin_status` 返回
  `supported:false`、`carrier:"dom-canvas"`、诊断 `plugin.unsupported_carrier: dom-canvas`；
  安装/启用/卸载/读取一律 `accepted:false`（同一 reason）。命令仍路由到 native handler，是为了
  给出线格式拒绝，而不是 Tauri “command not found”。
- 纯 Web：`web-adapter` 对五个名字逐个抛 `unsupported-runtime: cad_plugin_<name>`；没有持久
  存储，也不伪造空列表。
- **两种载体的内置 `tch` 仍然 always-on**：不支持插件管理器不等于关闭厂商解析。

**理由**：`dom-canvas` 的插件字节若落在页面里，会让 native 壳持有与 wasm 文档无关的第二套
持久状态；而 Web 没有可信的跨启动目录。把管理器限定在**持有文档权威的原生宿主**上，边界最
清楚。Android 的 `filesDir` 在 `nativeAssetsReady` 之前不可解析，因此用惰性 store，握手前
返回稳定拒绝而不是缓存空路径。

### D5 前端 Blob 导入 + ABI 校验；CSP `blob:` 是明示取舍

安装流程：

1. 隐藏 file input 接受 `.js`/`.mjs`；扩展名与 **4 MiB** 上限在读取内容前预检。
2. 前端把源码放进 `Blob`，用 Blob URL 以 ES module 导入**一次**，要求
   `globalThis.__YACR_VENDOR_PLUGIN__.abiVersion === 1` 且 `decode` 是函数；导入前后保存/恢复
   该全局，Blob URL 无论成败都撤销。
3. 校验通过后 `cad_plugin_install({ filename, payload(base64), meta? })`；宿主再次检查文件名、
   base64、大小、保留 id，然后写目录并设置 active。

四个 native `tauri.conf.json` 的 `script-src` 加入 `blob:`；Web 的 CSP 不变。

**取舍**：`blob:` 对所有 native 用户页面放宽了脚本来源。接受它的前提是——安装流程**本来就**
要在页面里执行任意插件代码，而且没有 worker/sandbox；`blob:` 只是让“已安装插件”的加载路径
成立，并不使插件变可信。**已知未闭合**：插件隔离、签名/权限清单仍然缺失（第 7 节）。

### D6 分层：持久化缝 / 纯模型 / 薄命令，不产生第二文档权威

| 层 | 职责 |
|---|---|
| `cad-platform::plugins` | `PluginStore` trait：原始 JSON 文本 + 入口字节；路径解析、id 包含检查、symlink 拒绝、4 MiB 上限、原子写。`FsPluginStore` / `NonPersistentPluginStore` |
| `cad-ui-core::session::plugins` | 纯 wire 模型：policy/manifest/row/status/mutation、id 与文件名派生、base64 校验、稳定 reason。无文件系统/Tauri/GPU |
| `cad-desktop-shell::plugins` | 五个 `#[tauri::command]` 包装、启动时解码器选择、唯一状态投影 |
| `apps/desktop-ui/src/plugins` | 唯一前端 wire 调用点 + Blob 校验/安装流程；`options/plugins-view.ts` 是纯视图模型 |

- `PluginStore` 只传**原始** JSON/字节，解析在共享 UI 层；这样 `cad-ui-core` 不必触碰文件系统，
  也不产生 `cad-platform → cad-ui-core` 的依赖环。
- 插件不是 CAD 实体/显示表示，也不进入 `cad-db`。安装/切换不创建事务、不改文档 revision、
  不触发重解析；厂商 pass 的 `vendor` 通道与派生表示语义（§5.1）保持不变。
- 状态命令与每个变更结果走**同一个**投影，变更不会报告一个状态命令不认识的 registry 形状。

## 3. 备选方案与弃用理由

| 方案 | 弃用理由 |
|---|---|
| 热重载 / live unload | 模块全局、pass token/generation、`applied` 校验都是进程级；中途替换会静默错配。重启语义可解释、可测试 |
| 多解码器同时启用 | `__YACR_VENDOR_PLUGIN__` 只有一个；记录/`applied` 归属会变成第二套分发协议。radio 语义显式 |
| 每个插件独立 Worker / sandbox | 方向正确，但需要跨 worker 的帧协议与新的输入/事件通道；本轮不把“隔离”伪装成已完成 |
| 只存内存、不落盘 | 无法跨启动；重新打开应用就丢失选择，用户无法理解 |
| 卸载 = 标记 disabled | 会留下可执行的字节和“已卸载但仍能读”的矛盾；决策是真删目录 |
| 在 `dom-canvas`/Web 上也建插件目录 | 与文档权威分离，且 Web 无可信持久目录；显式拒绝比假 UI 诚实 |

## 4. 安全与威胁模型

- **信任边界**：插件是**不可信代码**，执行时与应用同权。安装/校验/加载都会运行它；UI 明示
  “只安装你信任的文件”。没有签名、没有权限清单、没有进程隔离。
- **元数据不是身份证明**：`name/version` 只用于显示；`id` 是安装身份，但宿主只能验证语法、保留字
  与目录占用，无法从字节证明它。没有来源证明。
- **路径**：id 语法 + 目录包含 + symlink 拒绝，阻止 `..`/绝对路径/链接逃逸。
- **资源**：入口 ≤4 MiB；records/result 帧的既有上限（§4）仍由 vendor 通道强制。
- **CSP**：`script-src blob:` 放宽 native 页面；这是为外部解码器导入付出的代价，不构成隔离。
- **回滚**：策略写失败会删除刚写入的插件目录；损坏策略不会被静默覆盖。

## 5. 载体矩阵

| 载体 | `surfaceMode` | 插件管理器 | 存储 / 拒绝 |
|---|---|---|---|
| Linux 原生 Wayland | `native-below` | 支持 | `$XDG_CONFIG_HOME/yacr`（或 `~/.config/yacr`） |
| Linux X11/XWayland | `dom-canvas` | 不支持 | `supported:false` + `plugin.unsupported_carrier: dom-canvas`；mutations 拒绝；内置 `tch` always-on |
| Windows | `native-below` | 契约支持 | `%APPDATA%\yacr`；`APPDATA` 缺失 → `NonPersistentPluginStore` |
| macOS | `native-below` | 契约支持 | `$HOME/Library/Application Support/yacr`；`HOME` 缺失 → 同上 |
| Android | `native-below` | 契约支持 | 应用私有 `filesDir`；`nativeAssetsReady` 前 `unsupported-runtime: plugin-store-unavailable` |
| Web | `dom-canvas` | 不支持 | adapter 按名拒绝 `unsupported-runtime: cad_plugin_*`；无持久存储 |

“契约支持”只表示命令/路径/拒绝语义已在共享层接线并有契约测试/静态契约，**不**表示该平台已
运行验证。

## 6. 证据

实现轮次（2026-11-05）已运行：

- `npm test`：**1096/1096 pass**。覆盖 `plugin-manager.test.ts`（wire 校验、Blob/ABI 校验、
  安装前扩展名/大小、`vendor-settled`）、`options-plugins.test.ts`（单 radio、行状态、内联确认、
  无热重载声明、i18n 键一致）、`web-adapter.test.ts`（五个名字按名拒绝）、
  `carrier-routing`/`host-adapter`（shell-only 路由）。
- `cargo test -p cad-ui-core -p cad-platform -p cad-desktop-shell --locked`：通过。覆盖
  policy/manifest 往返与拒绝、`FsPluginStore` 原子写/越界 id/symlink/超限、五个命令的
  install→status→read→uninstall、dom-canvas 拒绝、`restartRequired` 语义、内置 id 保留。
- `cargo check -p app-linux --all-targets --locked`：通过。
- `python3 scripts/check-architecture.py`、`scripts/check-file-size.py`、`scripts/check-i18n.py`：
  通过（实现轮次记录）。

**NOT RUN / NOT VERIFIED**：

- Windows/macOS/Android 真实目标运行、Android 设备/模拟器、目录落盘、插件执行；
- 真实 GUI 的 Options → Plugins 交互与像素、真实浏览器、GPU/像素、CSP 在真机 WebView 的行为；
- 热重载/热卸载**未实现**（设计），不是未验证的缺陷。

本轮只改文档；文档改动重跑了 `check-command-docs.py`、`check-i18n.py` 与 `git diff --check`。

## 7. 后果与未决

- **未决（明确）**：插件隔离（Worker/wasm/sandboxed iframe）、签名/权限清单、真正的热切换、
  Web/`dom-canvas` 的持久插件目录、插件依赖/版本解析。
- **债务**：`script-src blob:` 对所有 native 用户生效；`meta` 只作显示；损坏策略的恢复路径是
  “拒绝写 + 报诊断”，没有 UI 修复向导。
- **不变量**：插件管理器永远不是 CAD 文档权威；它不进入 `cad-db`、不创建事务、不改变当前
  会话；没有热重载/热卸载；不支持的载体显式拒绝。
