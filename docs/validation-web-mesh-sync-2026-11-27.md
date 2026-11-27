# dom-canvas 网格内核同步例外（2026-11-27）

本轮把 `dom-canvas`（Web 与 Linux X11/XWayland，均编译 `apps/app-web` 的 wasm 会话）从
“无 `MeshModeler`、按 `unsupported-runtime: mesh-modeler` 拒绝”改为**有界同步执行**。
这不是把 native 的 worker 模型搬到 wasm，而是 ADR-0061 D7 的显式例外：

- **注入**：`Live::install` / `Live::install_empty` 在创建 `CadSession` 后安装
  `cad_kernel_manifold::ManifoldMeshModeler`；只有 wasm target 链接该 crate，宿主测试为 no-op。
- **执行**：`cad-app::tasks::mesh::MeshJobManager::start` 在 `target_arch = "wasm32"` 下不
  `spawn`，直接 `run_job`，把 terminal 放进现有 `mpsc`；下一次 `HostCore::tick` 的
  `poll_mesh_job` 走既有陈旧检查/提交路径（一条事务、一步撤销）。
- **预算**：调用内核前检查总操作数三角面，超过
  `MESH_SYNC_MAX_TRIANGLES = 200_000` 返回稳定 `mesh.sync_budget`，不启动计算。
- **已知语义**：页面在计算期间阻塞（无重绘、无输入、无进度）；取消只在调用前/后检查，
  **没有中途取消**；命令诊断文案按载体改成“computed synchronously ...”，不再写 background。
- **未改**：`native-below` 的 worker + 看门线程 + `CancelToken`；`cad-mesh-edit` 的纯面编辑/
  细分/平滑/补洞；共享层仍不链接内核。

## 命令与结果

| 命令 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | PASS（exit 0） |
| `cargo test -p cad-app --lib tasks::mesh --locked` | PASS（2 passed：阈值边界/`mesh.sync_budget` 文案 + inline 路径进入现有 poll 契约） |
| `cargo test -p app-web --locked` | PASS（152 passed, 0 failed；宿主侧 no-op 注入） |
| `cargo check -p app-web --target wasm32-unknown-unknown --locked` | PASS（exit 0） |
| `cargo clippy -p cad-app --all-targets --locked -- -D warnings` | PASS（exit 0） |
| `cargo clippy -p cad-app --target wasm32-unknown-unknown --locked -- -D warnings` | PASS（exit 0；检查 wasm 同步分支） |
| `WITH_FONTS=0 bash scripts/build-web.sh` | PASS（exit 0；`app_web_bg.wasm` **13,971,455 B**，低于 20 MiB slim 预算；较此前 13,281,659 B 增加约 0.66 MiB） |
| `python3 scripts/check-architecture.py` | PASS（exit 0） |

## 证据边界

- **没有浏览器运行证据**：没有在 Chromium/WebKitGTK 里实际执行 `UNION`/`SUBTRACT`/`INTERSECT`/
  `MESHREPAIR`/`MESHREBUILD`；没有点击、没有画面、没有取消行为观察。
- **没有 GPU/像素/性能/耗时证据**：`200_000` 三角面是 liveness 上限，不是基准结果；浏览器
  可接受的冻结时长、WebKitGTK 的 responsiveness 行为均 **NOT VERIFIED**。
- **没有真实大样本**：只用合成契约与预算单测；`mesh.sync_budget` 的触发也只由单测覆盖，
  没有浏览器端点击验证。
- **没有 Web Worker**：非阻塞/可取消路径仍未实现；本轮明确接受页面阻塞和无中途取消。
- **X11 与 Web 共享同一份 `app-web`**：X11 `full-formats` 与 Web slim 都会带内核；本轮只构建
  并测量了 slim Web bundle，X11 `build-canvas-wasm.sh` 的产物与 WebKitGTK 运行时同样
  **NOT VERIFIED**。
- 构建成功只证明 bundle 存在且尺寸在预算内，不是运行时、GPU、像素或产品验收。
