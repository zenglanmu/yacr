# 真实 DWG 打开与渲染回归（2026-10-03）

通用测试步骤、环境准备与通过判据见 [testing-dwg.md](testing-dwg.md)；本文仅记录本轮证据。

## 样本与结论范围

用户提供 `/home/zenglanmu/sources/cad-test-files/anteen.dwg` 与
`anteen-remove-color.jpg`（1080×760，dwgmodels 水印）。DWG SHA-256：
`818f54cd3b413ce3ab00a6aa849bc29cd8cc8581a39fc31a723691f40141fdbc`。
未下载额外图纸；这一样本已经暴露真实缺陷。DWG、参考图和第三方字体均不入库，
“免费下载”不表示允许再分发。单样本不能用于宣布 DWG 版本/工业兼容性。

**原生与浏览器打开/非空出图链路通过，视觉保真未通过；Web 仍缺文字。**

主证据目录：`/tmp/opencode/yacr-canteen-validation/`。

## 第一层：Linux 原生离屏（主测试）

release CLI，真实 wgpu/Vulkan，强制
`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`。
实际适配器：CPU / llvmpipe，Mesa 26.0.8，LLVM 21.1.8。
无 X11/Wayland、无 Slint、无浏览器。

| 项目 | 实测 |
|---|---|
| 导入 | 29,212 entities，25,123 model entities，108 blocks，14 layers，毫米 |
| `scan`（含进程启动/解析/转换） | 0.412 秒 |
| `build-representation`（另一次导入+表示） | 0.698 秒 |
| 2160×1520 `render`（另一次导入+构建+GPU+回读+PNG） | 25.530 秒 |
| 绘制 | 205,988 draws，651,424 vertices，2,902 triangles |
| 有字体表示 | 205,786 lines，202 meshes，0 未整形 Text，0 build failures |
| 像素 | 331,060 非背景像素，10.08% coverage |

字体使用测试机已有 `arial.woff`、`simplex.shx`、`txt.shx`、`romans.shx`；
GOST 原字体不在目录，采用显式回退，**不是原字体排版验收**。
时间是单次墙钟测量（不是纯解析时间、统计基准或交互性能达标）。
大量虚线片段形成大量独立 batch/draw，是后续性能调查入口。

### 已修复且有回归测试

1. `Renderer::render` 原来的负 Y 比例把 CAD 整图上下镜像。
   wgpu clip space 已是 Y-up，viewport 自行转为图像坐标，取消重复翻转。
   新 GPU 像素契约使用非对称上半屏线段、非零 batch/camera origin，并验证平移。
   原 mesh 测试的 CW 数据是适配错误投影的测试数据，改为真实 +Z/CCW；
   透明合成、渐变、三角形测试均继续执行。
2. CLI `render` / `plot` 没有使用 `--font`，只传 `None`，导致文字不入 GPU。
   两入口现在加载并注入 FontEngine；坏字体验证发生在 GPU 工作前。
3. `CadView::request_redraw` 仅安排 Slint 零时长 timer；浏览器 File API/字体
   回调发生在空闲事件循环外时，timer 本身不唤醒 winit。通过
   `slint::invoke_from_event_loop` 发布唤醒事件，CPU 准备仍在 timer、GPU 仍在
   renderer 内。四线段浏览器回归无鼠标移动就从 CAD 帧 1 更新为 3，并实际显示
   四线段（`browser-small-final/`）。单纯额外 window.request_redraw 的尝试未解决，
   其失败证据仍保留在 `browser-small-wake.log`。

### 参考图检查（不比较颜色）

`native-before.png`：立面跑到下面，平面跑到上面，文字与表格内容缺失。
修复后 `native/anteen.png`、`native-monochrome.png`：四幅立面在上，平面在下，
表格在右；主要门窗、桌椅、房间及轴线可见，文字与表格内容已绘出。
`reference-vs-native.png` 为左参考/右原生的人工检查图。
原生单色图按与背景的 RGB 差异生成黑色前景/白色背景，忽略原始色相；
这种二值化会把所有填充变黑，不宜用于填充灰度或线宽的定量验收。

剩余差异：立面填充观感/遮挡关系和参考不同，字体、字符尺寸与轴号近似，
复杂线型中的 shape/text 未支持；3DSOLID/WIPEOUT 等仍在导入 Partial 范围。
参考是带水印的 JPEG，视口边距/抗锯齿/线宽也不同，未建立对齐黄金像素指标，
没有宣称 IoU/SSIM 或图元完整率。导入 Partial 文案是保守能力报告，
不能把其中列出的每个类型都推断为本图实际漏画的对象数。

## 第二层：WASM + Playwright headless

基于本次 Rust 源码重新 release 构建 wasm；同源复制已有 99 字体目录。
Chromium 153.0.8010.12 / SwiftShader，实际 CAD 后端 WebGL2（非 WebGPU）。
通过真实 `#file-input` File API 导入，而不是只调用核心 parser。

真实文件导入实体数与原生一致（29,212），但最初只检查实体数/非空截图会
**误判成功**：截图实际仍为演示圆，而不是图纸。
修正脚本要求导入前已有 CAD 帧，导入后 CAD 帧计数必须增加，禁止接受旧纹理。
修复前 `browser-frame-verified.log` 记录 120 秒等待超时，`cad_frames=1`，
`error=None`，故这次浏览器出图不能算通过。四线段夹具也出现相同停滞；
`probe-wake.log` 证明移动鼠标即可让 CAD 帧从 1 增至 2，定位到外部 File API
回调与空闲事件循环的唤醒路径，而非单纯大图绘制耗时。
此前 `browser/browser.json` 的 passed 是已识别的错误判据，仅保留用于审计，
**不得用它作为验收证据**。

最终 `browser-final/browser.json` 与 `browser-final/browser-cad.png`：实体数
29,212，CAD 帧 **1 → 4**，真实 CAD 区域 10 种颜色，`error=None`，无 console/page error；
截图显示四幅立面、平面与表格，方向与原生一致，不再显示演示圆。
**最终浏览器集成 smoke 通过**，但表格只有框线、字体内容缺失，视觉验收仍不通过。
四线段辅助隔离回归 `browser-small-final/` 也通过（不是代替真实图纸的测试）。
WebGPU 探测没有可用适配器的 warning 是正常回退证据，不是 WebGPU 通过。

字体加载：`catalog=99 requested=646 planned=0 registered=0 failed=0`。
requested 是逐实体收集的引用次数，不是 646 个不同字体。未知名称被跳过而
未计为 failed；即使目录完备，本图仍无可用 CAD 字体，是单独的 Web 缺口。

## 复现

```bash
cargo build -p cad-cli-tools --release --locked
python3 scripts/check-dwg-native.py ~/sources/cad-test-files \
  /tmp/opencode/yacr-canteen-validation/native \
  --font arial=/tmp/opencode/yacr-pages-release/fonts/arial.woff \
  --font simplex=/tmp/opencode/yacr-pages-release/fonts/simplex.shx \
  --font txt=/tmp/opencode/yacr-pages-release/fonts/txt.shx \
  --font romans=/tmp/opencode/yacr-pages-release/fonts/romans.shx

# DIST 必须是新的专用产物目录，build-web.sh 会重建该目录。
DIST=/tmp/opencode/yacr-canteen-validation/web-event-wake WITH_FONTS=0 scripts/build-web.sh
cp -R /tmp/opencode/yacr-pages-release/fonts /tmp/opencode/yacr-canteen-validation/web-event-wake/fonts
python3 scripts/serve-web.py --directory /tmp/opencode/yacr-canteen-validation/web-event-wake --port 8106
PLAYWRIGHT_MODULE=$HOME/.local/share/headless-browser/node_modules/playwright-core/index.js \
  node scripts/check-web-dwg.mjs http://127.0.0.1:8106/ \
  ~/sources/cad-test-files/anteen.dwg /tmp/opencode/yacr-canteen-validation/browser-final 29212
```

`check-dwg-native.py` 输出每图的 SHA、独立阶段耗时、导入/表示/帧 JSON、stderr 和 PNG；
验证实际 CPU Vulkan/llvmpipe 和非空帧，明确区分 smoke 与人工保真。
`check-web-dwg.mjs` 在帧未更新、错误、实体数不匹配或空帧时非零退出。
Playwright 与字体是测试环境资源，不新增仓库运行依赖。

## 验证门

- lavapipe 核心串行测试：**965 passed / 0 failed / 1 ignored**。
- 其中 render/CLI 定向契约 **85 passed**（默认外部样本门没有文件时跳过）；
  额外设置 `YACR_TEST_DWG` 后真实图纸 CLI render 契约实际执行通过。
- release CLI/WASM 构建、fmt、两修改 crate 的严格 clippy、架构检查通过。
- UI wasm 测试编译通过（不是执行）；UI wasm 严格 clippy 另报既有
  `responsive.rs` 文档 lint 与 `adapter.rs` 的 `clone_on_copy` 等，未通过，
  未为本轮图纸测试顺便修改这些无关位置（见 `ui-wasm-clippy.log`）。
- 本轮没有 Android、真实 GPU、WebGPU 或生产部署证据。
