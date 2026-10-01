# 宿主文件闭环（F01/F09/F12）

把已合并的应用层 API（`HostController::open_bytes_leaving`、`UnsavedFlow`、
原子导出、`RecoverySnapshot`）接到两个真实宿主上，使打开/离开/保存/恢复不再默认放行。
核心逻辑可测，宿主真机/浏览器运行未验证。

## 共享逻辑（`crates/cad-app/src/host_files.rs`）

- `plan_leave(dirty, decision)` → `LeavePlan`：把"是否有未保存工作 + 用户决定"变成宿主可
  执行的动作（直接继续 / 先原子导出 / 先写恢复快照 / 取消）。**取消保留当前文档**。
- `export_annotations_atomically(controller, write)`：先 `prepare` 得到字节与 revision，
  由宿主写入，成功后再 `confirm`；**写入失败不标记已保存**，也不替换文档。
- `recovery_bytes` / `parse_recovery`：恢复快照的编码/解码；解码失败显式报错（非空快照）。
- `parse_decision` / `decision_name`：宿主 UI 字符串 ↔ `UnsavedDecision` 的唯一映射。
- `UnsavedSignal` / `LeaveResolution`：把决定暴露给宿主，禁止静默默认。

## Web（`apps/app-web`）

- 打开新文件时先走 `plan_leave`：脏文档弹出决定（保存 / 保留恢复 / 丢弃 / 取消），
  取消保留当前文档；保存走原子导出路径，失败则中止替换。
- 导出批注：编码 → 下载（浏览器）→ 确认 revision；下载失败不报"已保存"。
- 恢复：经 `cad-platform::Persistence`/本地存储可用时保存快照，启动时可恢复；
  不可用则显式提示，不伪造。

## Android（`apps/app-android`）

- 同上流程，导出/保存走 `FileAccess`（SAF/content URI）与原子写入；
  恢复快照经持久化契约保存，缺少存储时显式 `NotImplemented`。
- 已接入宿主连接器所需的决定回调；未接线的部分显示明确状态而非空成功。

## 验证

```bash
cargo test -p cad-app --locked                                  # 166 passed（含 host_files 逻辑）
cargo check --workspace --lib --target wasm32-unknown-unknown --locked   # app-web
ANDROID_HOME=... JAVA_HOME=... \
  cargo check --target aarch64-linux-android -p app-android --locked     # app-android
```

## 未完成（显式）

- **真机/浏览器运行未执行**：本环境无 adb/模拟器与浏览器宿主；宿主行为仅编译级验证，
  无安装运行、无真实下载/SAF 交互证据。
- **自动保存/崩溃恢复的完整策略**未闭环（何时写恢复、保留多久、清理）。
- **Android 网络字体取字节**仍为资产路径；HTTP 未实现（见 `docs/font-hot-loading.md`）。
- 未保存决定对话框的最终视觉与 U09 交互细节仍待设备验证。
