# 应用域 UI 插槽与插件扩展

> 决策背景见 [ADR-0053](adr/0053-application-domains.md)；当前状态见
> [`docs/application-domains.md`](application-domains.md)。本文件只描述**实现契约**，
> 不重复平台证据表。

## 1. 三个槽

`apps/desktop-ui/src/domains/ui/` 把域界面拆成三个**数据槽**。React 树只有一棵，槽只决定
“这一块渲染什么”，不 fork 组件、不新增第二套 UI。

| 槽 | 数据 | 渲染位置 |
|---|---|---|
| `menu` | `RibbonTab[]`（一个 tab = 一个顶层菜单） | 顶部 `DomainMenuBar`（`concept`）或折叠 ribbon 的菜单栏（`cad2d`） |
| `toolbar` | `RibbonPanel[]`（一个 panel = 一个工具组） | 左侧 `DomainToolbar`（`concept` 的 Large Tool Set） |
| `tray` | `TrayPanel[]`（内置 body 的声明式面板） | 右侧 `DomainTray`（`concept` 的默认托盘） |

槽不引入新的派发通路：每个可点项仍是 `RibbonItem`，经
`itemEnabled` / `unavailable` / `runItem` → `shell.canCommand` → `cad_command` 或
`runAction`。没有命令、也没有 `pendingKey` 的项在注册时就会被拒绝。

## 2. 域注册

```ts
import { registerDomainUi } from '../domains/ui/registry.ts';

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

```ts
import { registerPluginUi, unregisterPluginUi } from '../plugins/ui.ts';

registerPluginUi({
  id: 'acme.roof-tools',
  domain: '*',                    // 或 'concept' / 'cad2d' / ...
  toolbar: [{
    id: 'concept.construction',   // 已有组：追加；新 id：整组追加
    titleKey: 'concept.toolbar.construction',
    items: [{ id: 'ACME_ROOF', labelKey: 'acme.roof', icon: 'line', command: 'LINE' }],
  }],
  tray: [{
    id: 'acme.roof-panel',
    titleKey: 'acme.roof.panel',
    kind: 'pending',
    pendingKey: 'acme.roof.panel.pending',
  }],
});
```

- 菜单/工具组按 id 合并：已有分组**追加**项，新分组追加到末尾；item id 冲突时**域自己的行优先**，
  patch 的重复项被跳过（避免 React key 冲突，也不允许插件遮蔽域命令）。
- 托盘按 id 合并：同 id 由 patch 覆盖，新 id 追加。
- `id` 必须唯一；卸载用同一个 id，重复卸载是 no-op（deactivation 幂等）。
- patch 只能填**数据**：不能注入 JSX/HTML，也不能新增内置托盘 body。新增 body 是
  `DomainTray.tsx` 的代码改动，保持写入路径可审。
- 当前插件通过 `apps/desktop-ui/src/plugins/ui.ts` 直接注册（same-privilege，ADR-0029/0030/0034）。
  `packages/cad-plugin-api` 的 `ui.register` 命名空间**尚未发布**；在描述符 JSON、宿主能力表
  与 WebView adapter 一起落地前，不要在任何能力表或 UI 里声称它 supported。

## 4. 验证

```bash
(cd apps/desktop-ui && node --test test/concept-domain.test.ts)
(cd apps/desktop-ui && npm test)   # tsc + 全部前端契约
python3 scripts/check-i18n.py
python3 scripts/check-file-size.py
```

视觉、真实 GPU、真机和各非 Linux 载体运行：**NOT RUN / NOT VERIFIED**。
