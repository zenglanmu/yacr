# DWG 打开与渲染测试指南

## 1. 测试顺序与结论

**Linux 原生离屏 wgpu + Mesa lavapipe 为主测试**，定位导入、表示、字体、相机、
几何与 GPU 问题；每轮修复先走这条路径，不依赖桌面显示服务、Slint 或浏览器。
最后使用 **WASM + Playwright headless 做一次第二层集成测试**，验证浏览器 File API、
宿主接线、字体加载和实际呈现。WebGL2 软件路径通过不等于 WebGPU/真实 GPU 通过。

必须分别记录三个结论：

1. **打开通过**：实际导入文件，实体数/空间/单位合理，没有解析失败。
2. **出图 smoke 通过**：真实后端提交且生成非空图像；浏览器必须呈现新文件，不能保留旧纹理。
3. **视觉验收通过**：与参考图核对后的几何/文字/布局完整性结论。

退出码 0、非空截图、`error=None` 均不能单独证明视觉正确。
`Partial` 允许其他内容继续查看，但不能报告整图完整。记录缺项而不是改为空成功。
已执行的 canteen 回归及缺陷证据见 [validation-dwg.md](validation-dwg.md)。

## 2. 样本与资源管理

默认外部语料目录：`~/sources/cad-test-files/`。当前批测脚本只扫描目录第一层的
`.dwg`（扩展名大小写不敏感），不递归；建议保持平铺、文件主名唯一。

```text
cad-test-files/
  anteen.dwg
  anteen-remove-color.jpg
  sample-02.dwg
  sample-02-reference.png
```

- 每个 DWG 尽量配对应参考图，注明模型/纸空间、相机、隐藏图层、字体和参考软件。
- 缺参考图只能做打开/出图 smoke，不算黄金图验收。
- 需要扩充时可从 <https://dwgmodels.com/> 的**免费图纸**选择建筑、机械、块/文字等样本。
  记录下载页 URL、日期、文件 SHA-256、许可/使用限制；免费下载不意味着允许再分发。
- 外部 DWG、参考图和第三方字体不提交仓库；只有授权明确的夹具才能加入
  `fixtures/manifest`。不要为了测试修改 acadrust 或添加 Cargo patch。
- 字体目录与图纸目录分开。原字体缺失时显式注明使用了哪个回退字体；
  不能把回退排版说成原字体正确。字体来源/授权见 [fonts.md](fonts.md)。

以下命令均从仓库根目录运行。每轮使用新的专用输出目录，避免旧 PNG/JSON 冒充本轮结果：

```bash
export CORPUS="$HOME/sources/cad-test-files"
export RUN="/tmp/opencode/yacr-dwg-$(date +%Y%m%d-%H%M%S)"
export FONT_DIR="$HOME/sources/cad-test-fonts"
mkdir -p "$RUN"
git rev-parse HEAD > "$RUN/revision.txt"
git diff --stat > "$RUN/working-tree.txt"
```

`RUN` 是可替换的测试产物路径；提交号与工作区差异都要记录，因为未提交修复也影响结果。
字体可复用本机已有资源。确认授权后，需获取项目 Web 字体集时运行：

```bash
scripts/fetch-web-fonts.sh "$FONT_DIR"
```

## 3. 第一层：原生快速反馈

前提：项目 Rust 工具链、Python 3、Mesa Vulkan lavapipe ICD。
通常 ICD 位于 `/usr/share/vulkan/icd.d/lvp_icd.json`，其他发行版可用 `--icd` 指定。

```bash
cargo build -p cad-cli-tools --release --locked
python3 scripts/check-dwg-native.py "$CORPUS" "$RUN/native" \
  --font "arial=$FONT_DIR/arial.woff" \
  --font "simplex=$FONT_DIR/simplex.shx" \
  --font "txt=$FONT_DIR/txt.shx" \
  --font "romans=$FONT_DIR/romans.shx"
```

这是 canteen 的显式回退组合，不是所有图纸通用的精确字体配置。按样本增减重复的
`--font name=path`。不传 `--font` 可隔离几何问题，但不包含文字渲染验收。

脚本依次独立执行 `scan` → `build-representation` → `render`，每阶段重新导入；
默认 2160×1520，可用 `--width 1080 --height 760` 降低调试分辨率。
强制并核查 Vulkan / CPU / llvmpipe；不需要启动 X11/Wayland。

每图输出 `<name>-scan.json`、`<name>-build-representation.json`、
`<name>-render.json`、各阶段 stderr、`<name>.png`；`summary.json` 汇总 SHA、
墙钟耗时、实体数、Partial 原因、表示种类和帧统计。失败非零退出，其他样本继续测试。
墙钟耗时包含进程启动及该阶段全部工作，不是纯解析时间或统计性能基准。

### 参考图人工对照（忽略颜色）

1. 先核对整图方向、四周范围、各子图相对位置，排除上下镜像和 fit 错误。
2. 确认比较的是同一模型/布局、同样的图层可见性，裁去不同边距，按等比例缩放对齐。
3. 检查门窗/桌椅/轮廓/块实例、轴线、圆弧/曲线、标注、文字和表格内容。
4. 检查填充、孔洞、遮挡和绘制顺序；文字存在不等于内容/对齐/尺寸正确。
5. 不比较色相；参考图水印不算缺失图元。JPEG 压缩、抗锯齿、线宽与边距差异要单独注明。

如生成黑白前景对照图，先按原生图像与背景的 RGB 差异取前景，再将前景置黑、背景置白。
**这种二值化会把全部填充变黑**，只能辅助定位布局/轮廓，不能据此验收填充灰度或线宽。
没有对齐、掩码和容差约定，不输出或宣称 SSIM/IoU、图元完整率等自动保真分数。

修复渲染问题后运行串行契约，避免并发软件 GPU 测试互相干扰：

```bash
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  cargo test -p cad-render-wgpu -p cad-cli-tools --locked -- --test-threads=1
```

需要实际执行外部图纸的 CLI render 契约时，额外设置 `YACR_TEST_DWG` 为该 DWG 的绝对路径。
不设置时此项会跳过，不能将其计作真实样本证据。完整核心门见 `AGENTS.md`。

## 4. 第二层：WASM + Playwright headless

只在原生调试收敛后执行；前提为 wasm target、匹配锁文件的 wasm-bindgen CLI、
外部安装的 Playwright/Chromium。浏览器环境配置参考 [validation-web.md](validation-web.md)。

```bash
# build-web.sh 会重建 DIST；只能指向本轮专用输出目录，不能指向语料/字体目录。
DIST="$RUN/web-dist" WITH_FONTS=0 scripts/build-web.sh
cp -R "$FONT_DIR" "$RUN/web-dist/fonts"
# 在单独终端运行服务器，测试结束后 Ctrl-C 停止；端口占用时更换端口。
python3 scripts/serve-web.py --directory "$RUN/web-dist" --port 8106
```

另一个终端沿用相同 `RUN`/`CORPUS`，从原生 `<name>-scan.json` 读取 `entities`
作为最后一个参数（不是 `model_entities`）：

```bash
PLAYWRIGHT_MODULE="$HOME/.local/share/headless-browser/node_modules/playwright-core/index.js" \
  node scripts/check-web-dwg.mjs http://127.0.0.1:8106/ \
  "$CORPUS/anteen.dwg" "$RUN/browser" 29212
```

`PLAYWRIGHT_MODULE` 按本机安装路径替换；Playwright 不是项目运行依赖。
脚本使用 headless Chromium/SwiftShader，经真实 `#file-input` 打开文件，等待字体加载、
检查实体数/错误/**导入后 CAD 帧更新**，截取 CAD 区域并核查非空像素。
输出 `browser.json`、`browser-cad.png`；错误/超时非零退出。
默认操作超时 120 秒，可通过 `WEB_DWG_TIMEOUT`（毫秒）调整；不能靠无限等待隐藏失败。

必须人工确认截图是所测图纸而非演示图/上一文件。读取 `fonts` 中计划、注册、失败数：
未知字体可能被跳过，故 `failed=0` 不代表字体齐全，`registered=0` 的有字图纸不能判视觉通过。
记录实际后端；WebGPU 不可用而回退 WebGL2 的 warning 不表示 WebGPU 已验证。
CLI 显式回退字体与 Web 按目录自动加载的策略不同，应分别描述两端文字结果。

## 5. 交付记录

每轮在验证文档记录：代码版本/未提交差异、样本 SHA/来源、参考图、字体与回退、
命令、实际适配器、图像尺寸、三层结论、耗时范围、剩余缺项，以及证据目录。
保留失败/超时记录，修复后的证据用新目录；不要覆盖或引用旧 passed 冒充本轮通过。
新增可重现缺陷同步补契约测试与交接说明，不把单文件 smoke 推广为全 DWG 兼容。
