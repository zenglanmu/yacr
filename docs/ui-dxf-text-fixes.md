# UI、DXF 与文字尺寸修复（2026-10-03）

本轮用户要求按 `docs/ui-spec/` 补齐图标/禁用状态/视图操作，支持 DXF，使用网络 DWG
对比文字；**明确不跑完整测试，只要求 Linux/Web 构建**，最后提交推送并发布 Cloudflare Pages。
这里只记录本轮的针对性契约与样本，不沿用上一轮完整门禁作为本轮结论。

## 实现范围

- 共享 Slint 浮动导航增加 Fit / Pan / Zoom in / Zoom out；缩放通过统一 `Zoom` 命令，
  平移模式取消未提交绘制/测量/批注，再把主键拖动路由为宿主中键导航；进入取点工具退出平移。
  触控画布命中排除整个四按钮区域，纯画布显隐仍由配置总开关控制。
- 文件、测量、批注、绘图编辑及导航使用项目绘制的 SVG 图标；编辑入口不再只显示文字。
  图标与文字显式分配布局区域，禁用按钮图标/文字灰显。2D 时三维标准视图/透视入口禁用；
  属性空选择清除入口、空图层恢复入口禁用。Linux 无路径的打开/侧车入口、未实现 Trim、
  后端切换禁用；核心 Unsupported 防线不删除，不以隐藏按钮代替命令校验。
- DXF 使用锁定的 acadrust 0.6.3 原版 `DxfReader`，按 ASCII SECTION/二进制签名分流，
  仍使用同一个 ImporterBuilder、数据库、语义转换和进度链；严格解析与流完成检查，
  不允许截断文件变成空成功。浏览器 file-input 接受 `.dwg,.dxf`。二进制及 ASCII 合成契约
  覆盖 LINE；复杂 DXF/vender 兼容仍未验收，不增加 DXF 写回承诺。
- 修复旧式 SHAPES SHX 度量：名称已经 strip，后续不能再寻找名称结束符。
  使用实际 above/below 基线度量归一化，而非遗留默认十单位。未修改 acadrust/未加 Cargo patch。
  不随意缩放全部文字，不将 SHX 修复推广为全部字体/MTEXT 排版正确。
- Web 遇缺失字体名时加载目录中的 arial/simplex 作为显式回退，保留 unresolved 名称；
  FontEngine 回退整形产生 `text.font_fallback` / Partial，而非假称原字体可用。
  Linux 支持重复 `--font NAME=PATH`，注册真实本地 CAD 字体并声明回退顺序。
- 更新授权 OFL UI 字体子集，新增导航中文文案不依赖宿主系统字体；第三方 CAD 字体不入库。

## 网络 DWG 对比（本轮实际执行）

来源：<https://cdn.jsdelivr.net/gh/mlightcad/cad-data@main/data/canteen.dwg>，
2026-10-03 网络下载（首次传输超时，Range 续传后完整校验）。大小 2,618,816 bytes；SHA-256：
`818f54cd3b413ce3ab00a6aa849bc29cd8cc8581a39fc31a723691f40141fdbc`。
DWG 使用限制没有获得再分发授权，仅存 `/tmp/opencode/yacr-ui-dxf-text/`，不提交或发布样本。
参考为仓库外既有 `~/sources/cad-test-files/anteen-remove-color.jpg`（dwgmodels 水印），
没有冒充本轮下载了新的参考图。网页搜索服务返回 403，猜测的页面不是对应图纸，均不作为来源证据。

使用同一网络 DWG 与同一 `times.shx` 显式回退组合，原生 debug CLI + Vulkan/lavapipe
输出 `legacy-before.png` / `legacy-after.png`，1080×760；软件 GPU、非真实 GPU。
旧版 TIMES 度量误取默认 10，实际为 120+20=140，字符尺度错误达 **14 倍**。
修复前立面标题、标注与右侧表格文字大面积互相覆盖；修复后恢复到与参考图比例合理的
小字，几何位置不需人为更换图纸。另保留 arial/simplex/txt/romans 组合 `before.png`
作为非旧式 SHX 对照，不能把该组合本来正常的结果说成修复成果。

结论分别为：**打开通过 / 原生出图 smoke 通过 / 巨大文字缺陷修复得到对比证据**。
完整视觉保真仍未验收：本图原 GOST 字体缺失，TIMES 是显式回退，部分字符及复杂排版、
填充/遮挡与参考不同；不宣称原字体、所有字形、所有 DWG 兼容。

## 构建、针对性验证与发布

针对性验证：ASCII + 二进制 DXF（含截断拒绝）、旧式 SHX 合成度量、显式字体回退、
实际 Linux 宿主平移/缩放入口与禁用/触控排除契约均通过；没有运行完整 workspace 测试。
Linux release 与 Web release 含字体 bundle 已构建通过。Linux 实际合成 DXF 出图及导航通过
（`linux-dxf/`，24 画布像素变化），合成 UI 通过（`linux-ui/`，4553 像素变化）。
截图抽查发现浮动 Fit 按钮默认居中与 Pan 重叠，补显式 `y: 0px`，最终包重新构建，不沿用该图
冒充修复后的 UI 验收。

另尝试实际 Linux App 打开网络 canteen 并加载 TIMES 字体（`linux-network-dwg/`），
**240 秒超时，没有出图通过证据**。已有原生 CLI 对比图有效，但不能推广为大图 Slint 宿主
性能通过。本轮不为了绕过超时改成空成功；大图大量 outline/batch 的性能仍待处理。
最终 `linux-confirm/initial.png` 已抽查：Fit/Pan/+/− 四图标分行、不再重叠，Linux 未接线
打开/侧车按钮灰显。实际浏览器四按钮点击与 Pan 拖动验证通过，缩放改变 world-per-pixel、
平移改变相机中心、Fit 恢复中心（`web-navigation.json/png`）；第一次临时脚本漏加宿主 canvas
DOM 偏移导致误点，纠正坐标后重跑，未为此修改核心缩放语义。

最终构建：`cargo build -p app-linux --bin yacr-linux --release --locked`；
`DIST=/tmp/opencode/yacr-ui-dxf-text/web-deploy WITH_FONTS=1 bash scripts/build-web.sh`，
wasm **17,422,222 bytes**，SHA-256
`f86f94e4af45c244f33aa48c208bd9c7913570e13f94867b041cd549b0d1f485`。
发布包不含外部 DWG/参考图，字体按已有同源字体发布流程收集，第三方字体未提交 Git。
fmt、i18n、架构检查通过；OFL UI 子集覆盖全部当前中文目录字形。

WASM 真实 File API DXF 抽查通过：`browser-dxf/browser.json`，Chromium 153 / SwiftShader WebGL2，
1 LINE，CAD 帧 1→2，32 颜色，无 console/page error，诊断 Partial 保留；这只是合成 DXF
浏览器 smoke，不是复杂 DXF/真 GPU 验收。
Cloudflare Pages 已发布生产部署 `a6ea4bfe-9c00-40ae-8133-76535a242983`，API 确认
`production / deploy success`，关联实现提交 `d269140b86fab47ff458092172dd564de0a4d45c`。
生产地址 <https://yacr-examples.pages.dev/>，不可变地址
<https://a6ea4bfe.yacr-examples.pages.dev/>，现有域名 <https://yacr-examples.snakeheartgo.top/>。
生产 URL 已重跑真实 File API 合成 DXF smoke 通过（`production-dxf/browser.json`），
无 console/page error，1 LINE、2 CAD 帧、32 颜色，Partial 诊断保留。
Python 直接下载生产 wasm 校验 hash 遇 HTTP 403，线上二进制 hash **未验证**；
不能把本地 hash 当线上下载证据。线上浏览器为无头 SwiftShader，不是用户真机/真实 GPU。
本轮 **完整 workspace 测试、完整 clippy 门禁 NOT RUN（用户明确要求）**。
