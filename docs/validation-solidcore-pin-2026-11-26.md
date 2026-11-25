# 验证记录：solidcore 依赖升版与适配（2026-11-26）

范围：`cad-kernel-solidcore` 的五个 git 依赖（`solidcore` / `solidcore-convert` /
`solidcore-ops` / `solidcore-record` / `solidcore-tess`）从
`55ae49ef919b606bf77901df626ed9bb543e5212` 升到上游 `main` 的
**`f17599cbfa99b420ea9bc24531ad2a808dbda34c`**（2026-11-25 16:22 UTC，pin 之间 **727** 个上游
提交）。仍固定 revision，无 `[patch.crates-io]` 键，`vendor/` 未动，`Cargo.lock` 只改这五个包
（`solidcore-ops` 新增上游依赖 `serde`）。

**这不是能力验收**：本轮只证明编译、既有契约测试与静态门禁通过。上游在 727 个提交里改了大量
几何与错误分类；本仓库的断言只覆盖 ADR-0036 已接线的操作。其它平台、真实窗口/GPU/像素与第三方
CAD 接受性一律没有新证据。

环境：Linux x86_64，Rust 1.99.1（`rust-toolchain.toml`），全部 `--locked`。

## 1. 适配改动

- `crates/cad-kernel-solidcore/Cargo.toml` / `Cargo.lock`：五个依赖 pin 更新；`UPSTREAM_REVISION`
  改为 `f17599cbfa99`、`UPSTREAM_VERSION` 改为 `1.0.1+f17599cbfa99`（上游 `solidcore` crate
  版本仍是 `1.0.1`）。
- `src/error.rs`：上游 `Error` 的两个签名变化——`CornerUnsupported` 由单元变体改为结构体变体、
  `TooLarge` 新增 `face` 字段。配套改动：`from_extrude_profile` 把 `Error::Status` 与该族的
  `Argument`/`NotSolid` 同类映射为 `Unsupported` + `solid.extrude_profile_failed`。原因是上游
  `#18402` 退役了 `sweep_statuses_pass_on`，未闭合轮廓的直线扫掠现在把内核状态
  `Status(1001)` 向上传递（以前被吞掉）；ADR-0036 的拒绝码表要求该输入仍是
  `solid.extrude_profile_failed`，不是泛化的 `solid.kernel_failed`。
- `tests/fillet.rs`：按新 pin 的产物更新**测量**（不是放宽断言）：
  - `Blend::Round` 的 record label 从零斜率 `Surface::Cone` 变为 **`Surface::Cylinder`**；
  - `Blend::Curvature` 从裸 `RationalSpline` 变为 **`Surface::Blend`（`Procedural`、order 2、
    常数半径）**，几何仍在 `Face.nurbs`；
  - 圆角环的起始 use 变了，环断言改为**按循环比较**（`canonical_cycle`），端面/相切面改为按形状
    查找而不是按固定 face 下标；
  - 逐字拒绝文本（pcurve 缺失）按新 pin 更新。
  - 圆角体的质量属性/布尔**闭式数字一个没改**，同一测试在新 pin 上通过。
- 文档：`docs/3d-solidcore-and-mesh-import.md`、`docs/adr/0036-…`（含 §13 的 2026-11-26 复测
  附记）、`docs/compatibility.md`、`THIRD_PARTY_NOTICES.md` 的 pin 与圆角 label 事实同步。

## 2. 已运行

| 命令 | 结果 |
|---|---|
| `cargo check -p cad-kernel-solidcore --all-targets --locked` | exit 0 |
| `cargo test -p cad-kernel-solidcore --locked -- --test-threads=1` | **137 passed**（69 + 3 + 22 + 8 + 8 + 7 + 6 + 2 + 5 + 7，另有 1 个空测试二进制 0 passed） |
| `cargo check -p app-linux --all-targets --locked` | exit 0（1m09s） |
| `cargo check -p app-web --target wasm32-unknown-unknown --locked` | exit 0（仅既存 `dead_code` 警告） |
| `cargo test -p cad-kernel-adapter --test tessellator_seam --locked` | 7 passed |
| `cargo test -p cad-cli-tools --test solid_options --test solid_command --test solid_boolean_multi_shell --locked` | 1 + 16 + 7 passed |
| `cargo test -p cad-app --lib --locked solids` | 62 passed |
| `cargo test -p cad-app --lib --locked extrude` | 7 passed |
| `cargo test -p cad-app --lib --locked press_pull` | 8 passed |
| `cargo test -p cad-app --lib --locked chamfer` | 56 passed |
| `cargo test -p cad-app --lib --locked offset` | 78 passed |
| `cargo test -p cad-app --lib --locked massprop` | 32 passed |
| `cargo clippy -p cad-kernel-solidcore --all-targets --locked -- -D warnings` | exit 0 |
| `cargo fmt --all -- --check` | exit 0 |
| `python3 scripts/check-architecture.py` | `Architecture OK: 38 packages` |
| `python3 scripts/check-file-size.py` | `file-size OK: 2881 code file(s)` |

`cad-app` 的 solid 相关用例使用契约夹具/注入模型器，不链接本内核；它们只证明升版没有破坏
命令层契约。真实内核行为由 `cad-kernel-solidcore` 的集成测试覆盖（Linux x86-64，本仓测试
profile）。

## 3. NOT RUN / NOT VERIFIED

- **全量门禁/测试**：`cargo test --workspace --locked -- --test-threads=1`、全量
  `cargo clippy --workspace …`、`check-tauri-capabilities.py`、`check-fixture-manifest.py`、
  `check-command-docs.py`、`check-i18n.py`、`check-workflows.py` **NOT RUN**。
- **其它载体**：Windows / macOS / Android 的编译或运行 **NOT RUN**；本轮只有 Linux 与 wasm
  目标检查。`native-below` 真实窗口、GPU、像素、性能 **NOT VERIFIED**。
- **上游几何差异**：727 个提交里的布尔/扫掠/圆角/记录格式变化没有逐用例 golden 对照；本仓只跑
  自己已接线的操作与既有断言。上游新增能力（例如 `Body::face_loops`）**未接线**。
- **第三方接受性**：ACIS 写出仍只有本仓库/acadrust 自读证据；本轮没有改变写出器。
- **依赖网络**：升版时 GitHub 直连不稳定，`cargo` 通过客户端 `url.insteadOf` 镜像取到 pin；
  `Cargo.lock` 记录的是 canonical GitHub 源，仓库状态不含镜像配置。

## 4. 边界结论

- “升版通过”只代表**编译 + 本仓既有内核契约 + 静态门禁**通过；不代表上游 727 个提交的几何行为
  与旧 pin 等价，也不代表圆角/G²/拉伸之外的新上游能力可用。
- G² 圆角仍不可承载：新 pin 给它 `Surface::Blend` 标签，读取端按名拒绝；它还需要中性模型没有的
  NURBS **边界曲线**族。圆角/抽壳操作本身仍不在 `SolidOperation` 里。
- 圆角 label 的变化（cone → cylinder）是上游事实，读取端保留零斜率 cone 的兼容映射供旧 record
  使用；`compatibility.md` 与 ADR-0036 §13 已按日期区分两个 pin。
