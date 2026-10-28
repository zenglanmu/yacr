# Android WebView「present 出图全空」缺陷：定位与修复（2026-10-28）

> 范围：`dom-canvas` wasm 会话在 **Android WebView** 上不出图（CAD 分区纯 `#121212`、
> 默认 framebuffer 读回全 `(0,0,0,0)`），而**同一 bundle** 在桌面 Chromium 141 正常。
> 本轮定位根因并修复；证据上限与 NOT VERIFIED 见 §7。**这不等于「Android 已验收」**。

## 1. 结论（根因）

**该 GLES 实现（`Android Emulator OpenGL ES Translator (Google SwiftShader)`，
WebView 124.0.6367.219）在顶点阶段把「动态数组下标」读成 0。**

`wgpu-hal 30.0.1` 的 **sRGB present 路径**（`crates/../wgpu-hal/src/gles/web.rs::Surface::present`
的 `swapchain.format.is_srgb()` 分支）为了把离屏 swapchain 纹理拷到 canvas 默认 framebuffer，
会画一个全屏三角形，其顶点阶段是：

```glsl
const vec2[3] TRIANGLE_POS = vec2[](vec2(0.0,-3.0), vec2(-3.0,1.0), vec2(3.0,1.0));
void main() { gl_Position = vec4(TRIANGLE_POS[gl_VertexID], 0.0, 1.0); }
```

在这台 WebView 上 `TRIANGLE_POS[gl_VertexID]` **每个元素都读成 `(0,0)`** ⇒ 三角形退化 ⇒
**一个片元都不产生**，`gl.getError() == 0`、`present()` 返回 `Ok` ⇒ 会话状态全绿
（`ok:true`、`presented` 递增、`error:null`、backing store 正确），但 canvas 保持初始的
**全透明黑**，屏幕上只剩 Activity 背景 `#121212`。桌面 Chromium/ANGLE 编译该 shader 正常，
所以同 bundle 在桌面上有图。

同一个模式还命中了**本仓库自己的**全屏 blit（`crates/cad-render-wgpu/src/blit.rs` 的
`array<vec2<f32>,3>` + `corners[vertex_index]`，经 naga → GLSL ES 300 后同样是动态下标），
即使 present 修好，帧也进不了 swapchain 纹理。

## 2. 逐假设判定（在设备上实测，A/B 对照）

设备内用 CDP 在**同一 WebGL2 上下文**里做的最小对照（脚本只做诊断，未入库）：

| # | 假设 | 判定 | 实测 |
|---|---|---|---|
| 1 | canvas context attributes（`alpha`/`premultipliedAlpha`/`preserveDrawingBuffer`…） | **排除** | `getContextAttributes()` = `alpha:true, premultipliedAlpha:true, preserveDrawingBuffer:false, antialias:false, depth:true`；`premultipliedAlpha`/`alpha` 与桌面一致，且 `gl.clear(red)` 能正常合成 |
| 2 | present 采样 float/non-filterable 纹理得 0 | **排除** | surface/frame 均为 `Rgba8UnormSrgb`（后为 `Rgba8Unorm`），无 float 纹理；把 canvas 先 clear 成绿色再 present，**绿色一字未改** ⇒ present 连一个片元都没写，不是「写入了 0」 |
| 3 | blit 机制（quad/blitFramebuffer/viewport/scissor/colorMask/blend/flush） | **部分成立，已解释** | 实测 present 路径确实执行了 `viewport(0,0,1082,2007)`、`bindFramebuffer(DRAW,null)`、`drawBuffers([BACK])`、`drawArrays(TRIANGLES,0,3)`，program `LINK_STATUS=true`、`getError()=0`；缩放/剪裁/混合/colorMask 全部正常。唯一的异常是**画不出片元** |
| 4 | wgpu WebGL 后端限制（格式/attachments/MSAA/半浮点） | **排除** | `format=Rgba8UnormSrgb`、`samples=0`、`RGBA8` 写入与 `blitFramebuffer` 对照实验均正常；`checkPresentFormat`/尺寸校验全部通过 |
| 5 | readback 时序（`preserveDrawingBuffer:false` 里读已合成后的缓冲） | **排除（且已改正判据）** | 在**同一个任务内**（present 之后立刻 `readPixels`）读回仍全 0，与合成无关；判据改为**截图**（`scripts/canvas-pixels.cjs`） |
| — | **顶点阶段动态数组下标** | **成立（根因）** | 同上下文最小对照见下表 |

最小对照（同一 canvas/context、同一着色器结构，`drawArrays(TRIANGLES,0,3)`，
统计写入像素数；全屏 = 2,171,574）：

| 顶点着色器取角方式 | 覆盖像素 | `getError()` |
|---|---|---|
| `const vec2[3] P = vec2[](…)` + `P[gl_VertexID]`（= wgpu-hal present） | **0** | 0 |
| `const vec2[3] P = vec2[3](…)` + `P[gl_VertexID]` | **0** | 0 |
| 局部数组 `vec2 p[3]` + `p[gl_VertexID]` | **0** | 0 |
| `if/else if` 按 `gl_VertexID` 选常量（无数组） | 2,171,574 | 0 |
| 顶点属性缓冲（`in vec2 pos`） | 2,171,574 | 0 |
| 算术 `float(gl_VertexID==1u)`（无数组）= 修复后形式 | 2,171,574 | 0 |

结论：**不是** `gl_VertexID` 错，也**不是**「无属性 draw 被跳过」，而是**动态数组下标**
（无论 `const`/局部）在该驱动上取值为 0。

## 3. 修复（`crates/cad-render-wgpu`，无 Cargo patch、不改 wgpu-hal）

1. **`blit.rs`**：全屏三角形的角改为 `vertex_index` **算术**（`-1 + 4*float(vi==1u)` 等），
   顶点阶段不再有任何数组；新增**线性的目标格式**片段阶段
   （`TEXTURE_SHADER_LINEAR_TARGET`，IEC 61966-2-1 精确分段曲线，与 wgpu-hal 同款常量），
   由目标格式选择（`texture_shader_for`）。
2. **`canvas/policy.rs`**：
   - `preferred_present_format` 改为优先选**线性**格式（canvas 广告 `Rgba8Unorm` +
     `Rgba8UnormSrgb` ⇒ 选 `Rgba8Unorm`）。线性 surface 走 wgpu-hal 的另一条 present 路径
     —— 纯 `blitFramebuffer` 拷贝，**不用任何 shader**，从而完全不碰上面那个坏 shader；
   - `check_present_format` 从「必须完全相等」放宽为「帧是 sRGB 格式，surface 是该格式**或**它
     的线性兄弟」；其他组合（需要 decode / 换通道序）仍按名拒绝。sRGB 编码因此落在**我们自己的**
     blit 里，呈现在画布上的字节与 sRGB surface 路径**等价**。
3. **契约测试**：`blit::tests`（顶点阶段无数组 + 两个变体 GLSL ES 300 反译成功 + 反译出的顶点
   stage **不含** `gl_VertexID]`）、`canvas::tests`（线性优先、sRGB 对比规则、拒绝 decode）。

为什么不是特例：没有任何「如果是 Android 就…」的分支。线性 present 目标对**所有** engine 都成立，
它只是把色彩编码位置从驱动搬到本仓库唯一的一处 blit 里。

## 4. Android 像素结果（修复前 / 后，同一 AVD、同 x86_64 release APK 流程）

CAD 分区（1080×2050，去掉顶部栏与底部命令栏）截图统计
（`/tmp/opencode/yacr-android-present-20261028T045524Z/pixel-measurement.txt`）：

| | 众数颜色 | 众数占比 | 亮像素（>128） | 颜色数 |
|---|---|---|---|---|
| 修复前 | `#121212`（18,18,18） | 97.9 % | **0** | 3（全是 chrome） |
| 修复后 | `#000000`（CAD 清屏色） | 97.8 % | **532** | 26 |

修复后：黑底 + **四条竖向白色合成线**可见；横屏 ↔ 竖屏旋转一轮后复测数值**完全一致**
（第二次 present 同样成立）。

## 5. 设备内 CDP 取证方式与边界

release APK 默认关闭 WebView 调试。本轮取数使用 **`--debug` 构建**（可调试应用默认开启
WebView DevTools），**未在生成工程里留任何 instrumentation**（`MainActivity.kt` 未改，
`git diff` 无相关改动）。最终验收 APK 是 `--release` 重建、临时自签（密钥在 `/tmp/opencode`，
**不入库**）。

## 6. 浏览器载体回归（非回归证据）

`bash scripts/build-web-ui.sh` 重建后
`node scripts/web-ui-smoke.cjs --url http://127.0.0.1:8090/ --verbose`：
**PASS 4/4 cases（82/82 checks）**。其中新增的**非空白像素断言**（`scripts/canvas-pixels.cjs`，
独立标签 `the presented CAD canvas is not a uniform colour`）在 390/768/1280、live resize 后、
resize 回转后均通过（非众数像素占比 0.27 %–0.42 %）。该断言的**负向对照**已验证：空白 canvas
报 `uniform=true`（失败），画一条线后报通过 —— 不是空断言。Chromium 侧 `status().format`
确认为 **`Rgba8Unorm`**（线性 surface 路径生效），present 后同任务内默认 framebuffer
674,086/674,086 像素非零。

> **2026-10-28 后续（断点提到 1024 之后）**：同一脚本已扩到 **7 个 case（141 checks）**——
> `390x844`、`768x1024`、**`914x412`（手机横屏 → mobile）**、**`1024x900`（→ mobile）**、
> **`1025x900`（→ desktop）**、`1280x800`，外加 live resize `1280 ↔ 390` **与新的
> `1025 ↔ 1024`（1 px 门槛两侧）**；本轮实测 **PASS 7/7 cases（141/141 checks），退出码 0**
> （SwiftShader，布局/DOM 证据）。上面的 4/4、82/82 是**该轮当时**的数字，保留不删。

## 7. NOT VERIFIED（必须照写）

- **真机**：未运行。模拟器 ≠ 真机（CPU/GPU/驱动/WebView 版本全不同）。
- **真实 GPU**：未验证。本轮全部是 `swiftshader_indirect` **软件渲染**，不得用作 GPU/性能结论。
- **其它 WebView 版本 / 系统 WebView**：只测 124.0.6367.219。该驱动缺陷是否存在于其它版本、
  是否只有模拟器的 GLES 翻译层有，**未验证**；修复不依赖它是否存在（线性 present 在所有
  engine 上都成立），但「别的版本上原本也没问题」这一点未验证。
- **色彩空间等价**：线性 surface + shader 编码与 sRGB surface + 硬件编码**按构造等价**
  （同一 IEC 曲线、同一常量）；但本仓库的合成夹具是**纯白线 + 纯黑底**，两端对编码不变，
  因此**没有**中灰像素的对比证据。
- **输入/手势/软键盘/生命周期**（`onPause`/`onResume`、进程重建）、分屏：未验证。
- **aarch64/armv7/i686 产物**：只构建并安装了 x86_64。
