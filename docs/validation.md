# 验证与运行证据

本文件记录**实际执行过**的测试与构建。没有样本、没有真机的项目在此显式标注为未执行。

## 环境

- Rust 1.98.1（`rust-toolchain.toml` 固定），`wasm32-unknown-unknown`、
  `aarch64/armv7/i686/x86_64-linux-android` target 已安装。
- JDK 17（Temurin），Android SDK build-tools 34.0.0 与 30.0.3，
  platform android-34 / android-30，NDK 27.0.12077973。
- cargo-apk 0.10.0。
- 锁定依赖：acadrust 0.5.5、slint 1.18.1、wgpu 30.0.1（见 Cargo.lock）。

## 已执行的测试

纯核心 crate 单元/契约测试（`cargo test -p ...`）全部通过，含：

- cad-db：事务原子性、revision 递增、空提交不推进 revision、失败不留部分状态、
  Builder 引用校验（悬空图层拒绝）、包围盒。
- cad-geometry：弧长离散容差、bulge 半圆顶点、样条端点、面积/自交/非共面拒绝、
  变换、射线与工作平面求交、局部交点。
- cad-proxy：metafile 边界、未知 opcode 保守降级、缺失缓存不伪造、raw DWG 不与
  graphic_data 混用、超限拒绝。
- cad-import-acadrust：垃圾输入不 panic、超限拒绝、取消生效、DWG 签名检查。
- cad-representation / cad-spatial / cad-scene：提供器选择与歧义拒绝、网格查询、
  相对原点精度、stamp 过期拒绝、变更集局部失效、预算淘汰。
- cad-measure：3D 距离、三点角、非有限拒绝、自交面积拒绝、纸空间拒绝。
- cad-annotations：命令经事务、JSON 往返、未知字段保留、指纹不匹配拒绝、高版本拒绝。
- cad-query：分页、Mixed/Unset、文档切换过期、变更集修订检测。
- cad-app：模式权限在命令层、创建/撤销/重做、过期文档拒绝、图层覆盖不改底图、
  未保存批注离开需决策。
- cad-resources / cad-dependencies / cad-history：路径策略、依赖传播、元数据不触发
  重绘、撤销合并与预算、恢复日志。

`cad-ui-slint` 不在主机运行测试（Linux 宿主缺 fontconfig/freetype 开发头与
pkg-config，本环境无 sudo）；其 Slint 编译由 Android target 检查覆盖。

## 已执行的构建

- `cargo check --target aarch64-linux-android -p cad-ui-slint`：通过。
- `cargo check --target aarch64-linux-android -p app-android`：通过。
- 探针 APK（Slint Android 后端 + `unstable-wgpu-30`）：`cargo apk build` 产出
  `slintprobe.apk`（arm64-v8a，debug 258 MB），证明 Slint+Android 打包路径。
- 本仓库 APK：`cargo apk build -p app-android --target aarch64-linux-android --lib`
  （见 `docs/build.md`）。

## 未执行（明确标注）

- 真机/模拟器安装与运行：本环境无 adb/emulator，**未运行**。
- 浏览器运行：无 Wasm 导出与 JS 宿主，**未运行**。
- 桌面宿主：**未构建**（依赖缺失）。
- 真实 DWG 样本、黄金图、跨后端对照、性能基准：`fixtures/manifest` 为空，
  **未执行**；任何实体兼容性声明都不成立。
