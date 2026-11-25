# 应用域 UI 插槽与插件扩展

> 决策背景见 [ADR-0053](adr/0053-application-domains.md)；当前状态见
> [`docs/application-domains.md`](application-domains.md)。本文件只描述**实现契约**，
> 不重复平台证据表。

## 1. 四个槽

`apps/desktop-ui/src/domains/ui/` 把域界面拆成四个**数据槽**。React 树只有一棵，槽只决定
“这一块渲染什么”，不 fork 组件、不新增第二套 UI。

| 槽 | 数据 | 渲染位置 |
|---|---|---|
| `ribbon` | `RibbonTab[]`（一个 tab = 一个 ribbon 选项卡） | 顶部 `DomainRibbonBand`（`bim`） |
| `menu` | `RibbonTab[]`（一个 tab = 一个顶层菜单） | 顶部 `DomainMenuBar`（`concept`）或折叠 ribbon 的菜单栏（`cad2d`） |
| `toolbar` | `RibbonPanel[]`（一个 panel = 一个工具组） | 左侧 `DomainToolbar`（`concept` 的 Large Tool Set） |
| `tray` | `TrayPanel[]`（内置 body 的声明式面板） | `DomainTray`，侧别由 profile 的 `traySide` 决定（`bim` 左 / `concept` 右）；`SHELL_ACTIONS.domainTray` 显示/隐藏整列，托盘标题行的关闭按钮走同一转换 |

`ribbon` 与 `menu` 的数据形状相同（都是 `RibbonTab[]`），但渲染器不同：`ribbon` 是保留
92px 面板带的真实功能区，`menu` 是下拉菜单栏。`cad2d` 的 profile 用内建 `Ribbon`，不读取槽；
只有 `surface: 'domain-ribbon'` 的域才从槽渲染。

托盘槽的数据与侧别无关：同一份 `TrayPanel[]` 在左、右两侧渲染同样的面板与 body。侧别是
profile 的表现事实（`DomainProfile.traySide`，`bim` 左 / `concept` 右），`Workspace` 据此决定
`DomainTray` 放在 `.workspace__canvas` 之前还是之后；托盘组件只按 `side` 翻转朝向画布的边框，
并在左侧时不再为右缘导航坞预留 `--navrail-w`。

槽不引入新的派发通路：每个可点项仍是 `RibbonItem`，经
`itemEnabled` / `unavailable` / `runItem` → `shell.canCommand` → `cad_command` 或
`shell.runStandardView` / `shell.runSetView` 或 `runAction`。`viewPreset` 是
`cad_set_view { preset }` 的派发族（`bim` 的“BIM 视图”面板）；没有派发 token、也没有
`pendingKey` 的项在注册时就会被拒绝。

## 2. 域注册

```ts
import { registerDomainUi } from '../domains/ui/registry.ts';

registerDomainUi('bim', {
  ribbon: BIM_RIBBON_TABS,
  tray: BIM_TRAY,
});

registerDomainUi('concept', {
  menu: CONCEPT_MENUS,
  toolbar: CONCEPT_TOOLBAR,
  tray: CONCEPT_TRAY,
});
```

- 重复注册同一个域名会**替换**该域布局（后加载者胜），供测试或定制构建覆盖 profile。
- 空槽是 `undefined`；渲染器画 `null`，不会生成空条/空托盘。
- `RibbonPanel.pending: true` 的组必须 `items: []`；`RibbonPendingItem` 必须带
  `pendingKey`，这是“保留参考位置但不假装已实现”的唯一形状。

## 3. 插件增量 patch

公开的插件作者面是 `@yacr/plugin-api` 的 `api.ui.register`（客户端本地，无描述符行）；
宿主侧落到同一个 `registerPluginUi` 合并路径。插件只提供**数据**，不能注入 JSX/HTML/自定义
托盘 body；每个可点项仍是 `RibbonItem`，经 `runItem` → `canCommand` → `cad_command`/`runAction`。

```ts
api.ui.register({
  id: 'acme.roof-tools',
  domain: '*',                    // 或 'concept' / 'cad2d' / ...
  menu: [{ id: 'acme.roof', title: 'Roof', items: [
    { id: 'ROOF', label: 'Roof', command: 'LINE' },
  ] }],
  toolbar: [{
    id: 'concept.construction',   // 已有组：追加；新 id：整组追加
    title: 'Roof',
    items: [
      { id: 'ACME_ROOF', label: 'Roof', icon: 'line', command: 'LINE' },
      { id: 'ACME_DORMER', label: 'Dormer', pendingReason: 'no dormer kernel op' },
    ],
  }],
  tray: [{
    id: 'acme.roof-panel',
    title: 'Acme',
    pendingReason: 'no panel body yet',
  }],
});
```

- `label` / `title` / `pendingReason` 是插件提供的**字面文本**（宿主模型里的
  `RibbonItem.label`、`RibbonPanel.title`、`RibbonTab.label`、`TrayPanel.title` /
  `pendingText`），渲染时优先于 i18n 目录；`labelKey`/`titleKey` 只是稳定合成键，不显示。
  插件不能在宿主 i18n 目录里加行，因此语言切换需插件在注册前自行处理（v1 限制）。
- `command` 与 `pendingReason` **恰好一个**：前者经共享准入（不可用就 disabled），后者渲染为
  disabled 行和插件自己的说明，不假装可运行。
- 菜单/工具组按 id 合并：已有分组**追加**项，新分组追加到末尾；item id 冲突时域自己的行优先。
  托盘按 id 合并：同 id 由 patch 覆盖，新 id 追加。
- `id` 是卸载句柄，客户端内唯一；`api.ui.unregister(id)` 显式移除，`deactivate` 时 loader
  自动移除该插件注册的全部贡献。宿主没有 `onUiRegister` 钩子时
  `api.ui.register` 以 `unsupported-runtime: cad_api.Ui.register` 拒绝。
- 插件仍是 same-privilege 模块（ADR-0029/0030/0034）；这不是沙箱。`ui.register` 是
  **client-local**，不在 `crates/cad-ui-core/api/plugin-api.json` 的能力表里，不要写成
  `supported` 描述符操作。

## 4. 验证

```bash
(cd apps/desktop-ui && node --test test/bim-domain.test.ts test/concept-domain.test.ts)
(cd apps/desktop-ui && npm test)   # tsc + 全部前端契约
python3 scripts/check-i18n.py
python3 scripts/check-file-size.py
```

视觉、真实 GPU、真机和各非 Linux 载体运行：**NOT RUN / NOT VERIFIED**。
