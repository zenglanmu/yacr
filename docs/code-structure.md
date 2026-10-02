# 宿主代码拆分

本轮仅重组 Web 宿主与共享渲染桥，不改变领域模型、crate 依赖方向、命令协议或 wasm
导出名称。核心 CAD 算法的大文件仍可按各自领域继续拆分，不把本轮描述为全仓库完成。

## Web Rust 宿主

`apps/app-web/src/browser.rs` 是组合根：选择后端、组装控制器/UI/视图，持有唯一运行时。
`browser/` 下按职责拆分：

| 文件 | 职责 |
|---|---|
| `documents.rs` | 打开图纸、未保存决策、安装新文档 |
| `annotations.rs` | sidecar 导入/导出确认、显式恢复/丢弃 |
| `fonts.rs` | Fetch 字体、注册、过期结果拒绝与诊断 |
| `input.rs` | UI 命令分派、平移/缩放、后端切换前保护批注 |
| `persistence.rs` | 浏览器下载/选择器、localStorage 恢复槽 |

组合根通过 re-export 保留调用路径；子模块只在宿主内部共享运行时，不新增数据库副本。
保存仍是准备 → 宿主写入 → 确认准确 revision，失败不标记保存。

## Web JavaScript

`apps/app-web/web/main.js` 只负责启动与模块组装，`host/` 包含：

- `i18n.js`：共享 catalog、DOM/状态本地化及语言持久化。
- `files.js`：File API 选择器、未保存提示、下载与导出确认。
- `renderer.js`：可见性感知轮询、退避及首次就绪消息。
- `runtime.js`：WebGL 单采样兼容及 winit 控制流异常识别。

文件/语言模块通过显式参数接收 wasm 与状态接口，不依赖入口的可变全局变量。
`window.yacr` 的既有诊断接口保留；`build-web.sh` 复制整个 `host/`，CI 与部署脚本
检查所有模块存在，防止只发布入口导致 HTTP 404。

## Slint 渲染桥

`crates/cad-ui-slint/src/bridge.rs` 保留共享 GPU 状态、`CadView` 与 Slint 生命周期；
`bridge/scene.rs` 负责从权威数据库派生场景与批注叠加，`bridge/camera.rs` 负责相机
适应与应用/渲染参数映射，`bridge/tests.rs` 保留原有契约测试。
公共函数仍由 `bridge` re-export，宿主和已有集成测试不需要迁移调用路径。

## 回归入口

```bash
node --test scripts/test-web-host.mjs
scripts/build-web.sh
node scripts/check-web-ui.mjs http://127.0.0.1:8090/ /tmp/opencode/yacr-refactor-web.png
```

模块契约测试使用 DOM/wasm stub，只验证模块边界，不算真实浏览器或 GPU 验收。
Playwright 使用真实 wasm、WebGL2/SwiftShader：检查模块加载、非空画面、缩放变化、
双语切换/重载持久化、真实批注下载与 sidecar 回导，以及轮询不覆盖操作状态。
运行证据与明确未验证项见 `validation-web.md`。
