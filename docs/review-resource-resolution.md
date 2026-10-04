# Rust 审查：字体目录与 URL 路径段（2026-10-04）

## 范围与依据

仅修改 `crates/cad-resources` 与本文档。依据需求 §7.3：资源引用为逻辑 key，
不允许不可信路径自动访问外部资源。已阅读 `AGENTS.md`、需求、
`docs/review-resources.md` 和 `docs/review-branches.md`；未重复修改 resolver
替换计费、错误传播或图片像素计数溢出的既有修复。

## 已修复的源码缺陷

1. **字体目录接受路径而将其原样放入 URL**：例如 `../outside.shx` 可匹配
   绘图字体名，两个规划 API 都会生成含上级路径的 URL。现在非空目录文件项必须
   为安全、无需清洗的裸文件名；目录分隔符、绝对路径、点段、控制字符及会被
   清洗的首尾空白/引号返回已有 `CadError::InvalidInput`，不静默丢弃或截断。
   既有缺失/空白文件项跳过行为保持不变。
2. **文件名只编码空格，改变实际请求目标**：`A #?%2F宋.shx` 原先会引入
   fragment/query 或预编码分隔符，主机请求的文件与目录声明不一致。现在按 UTF-8
   字节编码所有非 RFC 3986 unreserved 字符，字面 `%` 编码为 `%25`，防止一次
   URL 解码把目录文件名当成路径或预编码指令。两个规划 API 共用此逻辑。
   对绕过目录解析、直接构造的公开 `FontFace`，URL 构造先取裸文件名，点段变为
   `unnamed`，作为兼容现有返回类型的纵深防御；这不是解析成功或字体可用的声明。

## 新增合成契约测试（NOT RUN）

- `font_catalog_rejects_paths_and_lossy_file_names`：合法项之后出现路径、点段、
  控制字符或会改变 key 的清洗输入，整个解析显式失败。
- `font_urls_encode_reserved_characters_and_utf8_as_a_single_segment`：空格、
  fragment/query、字面百分号、中文 UTF-8、预编码 traversal 字面串，以及两个规划 API。
- `manually_constructed_font_faces_cannot_inject_url_path_segments`：公开构造入口的
  上级路径、反斜线、点段和百分号编码。

## 执行证据与限制

本子代理按任务约束仅写代码、测试和文档：**编译、测试、格式化、clippy、静态门禁
全部 NOT RUN**；未提交或推送，交由主控合并后执行 debug 编译/检查。
未添加依赖或新的用户可见诊断目录键，未修改其它 crate、共享 handoff 或 acadrust。

主机仍需提供可信、无 query/fragment 的目录 base URL，并自行实施网络授权、重定向、
域名和响应大小策略；此 crate 不发起网络请求。编码不能保证拒绝服务端的重复解码，
也不替代主机授权。非空不合法文件项现在使目录整体失败，是有意的输入契约收紧。
缺失/空白文件项仍沿用跳过规则；别名冲突、资源版本/跨文档隔离和完整目录大小限制
不在本轮修复范围。真实字体、真实 DWG、GPU、浏览器/真机及视觉验收均未运行，
不提升 BigFont、图片或 Xref 支持等级。
