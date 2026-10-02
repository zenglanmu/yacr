# CAD 场景 / GPU runtime / Slint 呈现边界

保留 ADR 0002/0003：Slint 是唯一呈现协调者，提供共享 Device/Queue，CAD 绘制离屏
纹理供 Slint 合成。没有第二个窗口/呈现循环，也没有逐帧 CPU 像素回读。

## 职责

- `crates/cad-app/src/render_scene/`：`CadSceneController` 管理 CPU 场景准备、版本与
  诊断。底图与批注分别缓存；批注 revision/显隐变化不重新构建底图。不可变文档
  Arc 的身份在接收时计算并保留，平移/缩放不重复遍历数据库 bounds；不同打开即使
  id/revision/bounds 相同也重新准备。字体替换使用单调 resource revision。
- `crates/cad-ui-slint/src/bridge/view.rs`：接收完整 `ViewSnapshot`，验证空间和相机
  后一次替换。没有独立写入二维/三维相机或模式的公共 setter。宿主写入合并到一次
  event-loop preparation task；准备成功后请求 Slint 绘制。
- `bridge/runtime.rs`：`CadRenderRuntime` 管理设备 epoch、上传、目标与画面失效。
  `BeforeRendering` 只同步已准备场景、按需绘制并返回纹理。UI-only 重绘复用纹理；
  相机改变不上传几何。`Detached/Ready/Lost/Failed` 明确区分；Lost 不上传/绘制，
  必须等待 Slint 提供设备，不能以重复 redraw 假装恢复。
- `bridge/presenter.rs`：导入纹理与绑定 Image。绑定 key 为设备 epoch + renderer
  texture revision + 尺寸；同尺寸的新附件也重新绑定。导入失败不推进 key，错误
  可诊断并允许重试。画布尺寸由 UiHandle 提供，而非整个窗口尺寸。
- `cad-render-wgpu`：`prepare_upload` 暂存 GPU 批次；`commit_upload` 成功后替换
  活动批次。runtime 此后才推进已应用场景版本。底图批次作为 prefix，批注替换
  suffix；失败不会先 clear 旧场景。已准备资源不能提交到不同/丢失的设备。

`IncomingDocument` 是当前不可变数据库的共享引用，**不是待消费消息槽**。
`None` 表示没有打开文档，准备并发布空场景，不遗留旧图。任务携带真实 viewport
DocumentId、数据库 revision 与场景准备 generation，不再写死 DocumentId(0)。

## 正确性与剩余限制

- 场景准备已移出绘制回调，但目前仍在 UI event loop 上执行，不是 Worker/后台
  线程。大图 CPU 准备仍可能阻塞事件循环；本次不宣称完成后台任务、取消或帧预算。
- 上传是同步暂存/提交：在分配前验证索引、有限坐标与 GPU buffer 限制。
  wgpu 的异步驱动/验证失败仍依赖渲染 error scope 和设备生命周期；不能把同步
  `Ok` 当作 GPU 异步成功的证据。分批上传预算仍待实现。
- CPU 准备失败保留上一份 ready 场景，诊断明确其更新失败；GPU 上传失败不推进
  applied revision，下一个呈现机会可重试。不能把旧场景当作新版本成功。
- 底图/批注组已独立，但不是完整的逐实体 ChangeSet 增量更新。
- 设备丢失状态与重挂接契约已实现；真实浏览器/Android 设备丢失及自动恢复仍待验收。

## 验证入口

```bash
cargo test -p cad-app render_scene --locked
cargo test -p cad-render-wgpu --test render_effects --locked
cargo test -p cad-ui-slint --lib --target wasm32-unknown-unknown --no-run --locked
PLAYWRIGHT_MODULE=/path/to/playwright-core/index.js node scripts/check-web-mobile.mjs URL
PLAYWRIGHT_MODULE=/path/to/playwright-core/index.js node scripts/check-web-ui.mjs URL
```

GPU 契约覆盖失败保留旧场景、overlay prefix 更新、丢失设备拒绝提交和 texture
revision。CPU 契约覆盖 Some→Some 字体失效、批注复用底图、关闭文档、准备失败保留
版本及同身份不同打开。Web 报告的 `cad_frames` 用于验证语言/UI-only 重绘不绘制 CAD。
Slint 原生测试受宿主 fontconfig/pkg-config 环境限制；wasm `--no-run` 仅代表编译。

本轮最终核心串行测试为 875 passed / 0 failed / 1 ignored；严格 clippy（cad-app、
cad-render-wgpu 全目标与 app-web wasm 独立检查）通过。UI wasm clippy 仍有 32 项
既有 adapter/responsive lint，没有把它声明为严格通过。核心默认并行测试一次在
GPU integration test 进程遭遇驱动 SIGSEGV；同一套测试 `--test-threads=1` 全部通过。
浏览器证据：`/tmp/opencode/yacr-bridge-final-mobile/report.json`、
`/tmp/opencode/yacr-bridge-final-desktop.json`。桌面缩放像素发生变化，移动端语言切换
复用 CAD 画面、真实文件选择器导入 synthetic DWG、旋转保留文档；console/page errors=0。
Android `aarch64-linux-android` 的 cad-ui-slint/app-android 编译检查通过；本轮未重新
运行 APK，不把编译检查算作设备运行验收。
