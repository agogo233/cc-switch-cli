# Proxy/Claude File Size Reduction Design

## Goal

在不改变现有外部行为的前提下，先对 `proxy/Claude` 相关代码做一轮安全分批重构，尽快消除 `>1600` 行的热点文件，并为后续把主要文件进一步压到 `1200` 行左右建立清晰边界。

这轮设计不覆盖整个 `src-tauri`，也不等于“把整个 proxy 都整理一遍”。范围以明确文件清单为准，只有这些文件和它们拆分后新增的直接子模块在 scope 内。

## In Scope

本轮允许直接改动的现有文件只有这些：

- `src-tauri/src/proxy/forwarder.rs`
- `src-tauri/src/proxy/response.rs`
- `src-tauri/src/proxy/response_handler.rs`
- `src-tauri/src/proxy/provider_router.rs`
- `src-tauri/tests/proxy_claude_streaming.rs`
- `src-tauri/tests/proxy_claude_openai_chat.rs`

本轮允许新增的文件，只限于上面这些文件为了拆分而自然产生的子模块或测试辅助文件，例如：

- `src-tauri/src/proxy/forwarder/*.rs`
- `src-tauri/src/proxy/response/*.rs`
- `src-tauri/src/proxy/response_handler/*.rs`
- `src-tauri/src/proxy/provider_router/*.rs`
- `src-tauri/tests/proxy_claude_streaming/*.rs`
- `src-tauri/tests/proxy_claude_openai_chat/*.rs`

不在上面清单里的 `src-tauri/src/proxy/**` 文件，这轮都默认不动。若实现中发现必须触达，应该先回到设计/计划层重新确认，而不是顺手扩 scope。

## Current State

当前分支上最突出的超长文件有：

- `src-tauri/src/proxy/forwarder.rs`，3123 行
- `src-tauri/tests/proxy_claude_streaming.rs`，2710 行
- `src-tauri/tests/proxy_claude_openai_chat.rs`，1431 行

另外，`src-tauri/src/proxy/response.rs`、`src-tauri/src/proxy/response_handler.rs`、`src-tauri/src/proxy/provider_router.rs` 虽然还没有超到同样程度，但已经出现明显的职责混杂：主链路、纯 helper、错误摘要、测试代码、日志收尾逻辑都挤在同一个文件里。

## Hard Targets For This Round

这一轮的硬性处理目标按文件划分如下：

- `src-tauri/src/proxy/forwarder.rs`：必须处理；本轮结束后必须 `<=1600`，不要求一步到 `<=1200`
- `src-tauri/tests/proxy_claude_streaming.rs`：必须处理；本轮结束后必须 `<=1600`，并尽量压到 `<=1200`
- `src-tauri/tests/proxy_claude_openai_chat.rs`：必须处理；本轮结束后目标是 `<=1200`
- `src-tauri/src/proxy/response.rs`：应处理，但只限 helper/test 外提；若本轮不超阈值，不要求继续为了数字强拆
- `src-tauri/src/proxy/response_handler.rs`：应处理，但只限 helper/test 外提；若本轮不超阈值，不要求继续为了数字强拆
- `src-tauri/src/proxy/provider_router.rs`：可选处理；只有在 endpoint rewrite / tests 外提后边界仍然自然时才继续拆

换句话说，这轮的完成条件不是“所有 in-scope 文件都压到 1200 以下”，而是：

- 先把三份最明显的目标文件收口到约束内
- 让其余运行时文件的边界变清楚
- 不为了追求行数去冒语义回归风险

问题不只是“文件长”，而是边界弱：

- 测试基础设施和场景断言混在一起，补测和定位失败都慢
- 运行时代码里纯策略、纯构建逻辑和副作用编排混在一起，可读性差
- Proxy 热路径刚做完 upstream parity，对大拆大改的回归容忍度很低

## Constraints

这轮重构必须遵守下面几条：

1. 语义不变。对外接口、请求/响应行为、错误语义、timeout/failover/logging 结果都不应变化。
2. 优先安全分批，不追求一次把所有相关文件都压到 `1200` 以下。
3. 绝不允许在这轮之后还留下 `>1600` 行的 `proxy/Claude` 目标文件。
4. 只在确实有利于边界清晰时拆模块，不为了行数机械切片。
5. 先拆纯函数、纯策略、测试辅助，再考虑拆主编排链路。

## Non-Goals

这轮不做下面这些事：

- 不改 proxy 的上游对齐策略
- 不改 provider failover、circuit breaker、timeout 语义
- 不做整个 `src-tauri` 范围的普遍性瘦身
- 不把热路径一次性“重写成更漂亮的架构”

## Design Choice

这次采用“安全分批混合拆分”的方案。

原因很简单：

- 只拆测试，收益不够，`forwarder.rs` 仍然会远超阈值
- 直接主攻 `forwarder.rs` 主循环，风险太高，和刚收口的 Claude parity 冲突
- 先把测试、内嵌测试、纯 helper、纯策略抽出来，再收口 `forwarder.rs` 里低风险段落，能在行为风险和收益之间取到更好的平衡

## Target Architecture

### 1. `forwarder.rs` 变成薄入口

`src-tauri/src/proxy/forwarder.rs` 最终应该只保留：

- 对外公开类型
- 主入口方法
- 少量串联编排逻辑

下面这些内容应优先外提到子模块：

- 请求构建与 header/body 预处理
- 请求发送与响应接收的共用流程
- 错误分类与重试结果整理
- `#[cfg(test)]` 测试模块

第一批不强行拆散 provider failover 主循环本身，除非拆分点已经非常明确且能做到零语义漂移。

建议的模块边界：

- `src-tauri/src/proxy/forwarder/request_builder.rs`
- `src-tauri/src/proxy/forwarder/error_classification.rs`
- `src-tauri/src/proxy/forwarder/send.rs`
- `src-tauri/src/proxy/forwarder/tests.rs`

### 2. Claude 测试按“基础设施”和“场景组”分离

`src-tauri/tests/proxy_claude_streaming.rs` 与 `src-tauri/tests/proxy_claude_openai_chat.rs` 的问题，主要不是断言本身复杂，而是 helper、假上游 handler、等待函数、数据库校验、业务场景全在一个文件里。

这轮应该把它们拆成：

- 公用 helper / fake upstream / fixture
- 按场景分组的测试模块

推荐分法：

- `src-tauri/tests/proxy_claude_streaming/helpers.rs`
- `src-tauri/tests/proxy_claude_streaming/success_cases.rs`
- `src-tauri/tests/proxy_claude_streaming/error_cases.rs`
- `src-tauri/tests/proxy_claude_streaming/logging_cases.rs`

以及：

- `src-tauri/tests/proxy_claude_openai_chat/helpers.rs`
- `src-tauri/tests/proxy_claude_openai_chat/transform_cases.rs`
- `src-tauri/tests/proxy_claude_openai_chat/error_cases.rs`
- `src-tauri/tests/proxy_claude_openai_chat/logging_cases.rs`

实际命名可以根据 Rust integration tests 的组织方式稍作调整，但原则不变：helper 和场景断言分开，场景按行为组织，而不是按历史追加顺序组织。

### 3. `response.rs` 与 `response_handler.rs` 只保留主逻辑

这两个文件现在最大的问题，是 helper、收尾逻辑和测试块都和主路径混在一起。

这轮拆分的重点是：

- 把错误摘要 helper、header copy、timeout/read helper 等纯逻辑外提
- 把 `#[cfg(test)]` 大块测试移出主文件
- 保留 `PreparedResponse`、主 builder 入口、最终收尾入口在当前文件，避免过度打散调用关系

建议的模块边界：

- `src-tauri/src/proxy/response/error_summary.rs`
- `src-tauri/src/proxy/response/tests.rs`
- `src-tauri/src/proxy/response_handler/stream_recorder.rs`
- `src-tauri/src/proxy/response_handler/tests.rs`

### 4. `provider_router.rs` 只做路由和状态编排

`provider_router.rs` 当前包含：provider 选择、breaker registry、upstream endpoint rewrite、current-provider 读取、测试。

这轮建议先抽两个最稳的边界：

- endpoint rewrite
- `#[cfg(test)]` 测试模块

breaker registry 与 provider 选择逻辑只在边界很清晰时再拆，不作为第一优先级。

## Execution Strategy

### Batch 1: 先处理低风险体积

第一批目标是快速消灭最明显的超长点，并让主逻辑显形。

范围：

- 拆 `proxy_claude_streaming` 测试
- 拆 `proxy_claude_openai_chat` 测试
- 把 `forwarder.rs`、`response.rs`、`response_handler.rs`、`provider_router.rs` 的 `#[cfg(test)]` 模块外移

完成后，应该至少做到：

- `src-tauri/tests/proxy_claude_streaming.rs` 不再超过 `1600`
- `src-tauri/tests/proxy_claude_openai_chat.rs` 不再超过 `1200`
- `forwarder.rs` 先通过移出测试显著降体积，并让后续 request builder / error classification 外提不需要再碰测试块
- `response.rs`、`response_handler.rs` 至少完成测试外移或纯 helper 外提中的一项，让主文件更接近只保留主逻辑

### Batch 2: 拆 `forwarder.rs` 的低风险纯逻辑

第二批只处理低语义风险段：

- request builder
- 错误分类
- 可复用 send helper

这里的目标不是把 `forwarder.rs` 一次拆到完美，而是让主文件只剩编排骨架，继续向 `1200` 逼近，同时不碰最脆的 failover 主循环语义。

### Batch 3: 如有必要，再拆中风险编排块

只有在前两批做完后，`forwarder.rs` 仍明显超过阈值，才考虑第三批：

- 进一步抽 streaming / buffered 共用流程
- 收口 permit 生命周期处理
- 细化 provider dispatch 相关逻辑

这一批必须建立在前两批测试绿且 review 通过的基础上，不默认强行执行。

## Error Handling And Semantic Safety

为了保证“只是重构，不是改行为”，每一批都要遵守下面的安全线：

- 不改变 public function 签名，除非只是模块路径移动带来的内部可见性调整
- 不改变现有错误类型和状态码映射
- 不改变 timeout budget 的计算与传递方式
- 不改变 `streaming` / `buffered` / `non-SSE fallback` 的行为分叉
- 不改变 provider success sync、request logging、usage logging 的收尾时机

如果某个拆分必须同时改逻辑判断，说明边界选错了，应该退回上一步重选拆分点。

## Testing Strategy

验证策略按“改动半径”分层：

### 每个批次都必须通过的通用 gate

- `cargo test --no-run`
- 该批次直接触达文件对应的 unit / integration tests

如果某个批次触达了通用 proxy 文件，但改动不只是“测试外移”或“纯 helper 外提”，就不能只跑 Claude 定向测试，必须补充至少一层更通用的 smoke gate，避免把范围悄悄扩大却没有对应验证。

### 每个小批次的最低验证

- 对应的 unit tests
- `cargo test --test proxy_claude_openai_chat`
- `cargo test --test proxy_claude_streaming`

### 涉及 `forwarder.rs` 时的额外验证

- `cargo test --test proxy_claude_forwarder_alignment`
- `cargo test --test proxy_claude_response_parity`
- `cargo test --test proxy_claude_model_mapping`
- `cargo test --test proxy_upstream_error_summary`

### 完成阶段性收口后的最终验证

- `cargo test proxy::response::tests`
- `cargo test proxy::usage::parser::tests`
- 上面全部 Claude 定向集成测试

### 通用 proxy 文件的额外限制

如果本轮改到 `response.rs`、`response_handler.rs`、`provider_router.rs`：

- 允许的首选改动是测试外移、纯 helper 外提、模块重组
- 如果出现跨 provider 的行为判断变化，说明已经越过这轮边界，应停止并回到设计/计划层
- 除非补了对应的非-Claude smoke coverage，否则不接受“顺手整理通用路径”的实现

## Expected Outcome

这轮完成后，`proxy/Claude` 范围内应达到下面的状态：

- 不再有目标文件超过 `1600` 行
- 测试文件和运行时文件的职责边界比现在清楚得多
- `forwarder.rs` 主文件显著变薄，主链路更容易读
- 后续若继续把文件压到 `1200` 左右，可以在更小、更稳的边界上继续推进，而不是每次都从一个三千行文件里动刀

## Open Trade-Off

这份设计明确接受一个现实：第一轮不保证把 `forwarder.rs` 一步压到 `1200` 以下。

如果为了这个数字强行把 failover 主循环、timeout/streaming 分支、permit 生命周期一次全部打散，行为回归风险会高很多，也不符合“语义不变优先”的前提。

所以这轮的成功标准是：

- 先把最危险的超长测试文件和最容易外提的运行时段落拆开
- 先消灭 `>1600`
- 再在 review 和验证结果允许的前提下，继续推进第二批
