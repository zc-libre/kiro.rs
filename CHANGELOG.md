# Changelog

## refactor/v2 (2026-04-26)

### Breaking Changes
- 移除 `countTokensApiUrl` / `countTokensApiKey` / `countTokensAuthType` 配置项。
  `count_tokens` 端点改为内置 token 估算，不再支持外部代理。
  旧字段会被 serde 静默忽略（向前兼容），但下次保存配置时会从文件中消失。

### Features
- Hexagonal 架构重构：`domain` / `infra` / `service` / `interface` 分层。
- 错误体系：`thiserror` 分层取代 `anyhow`，HTTP 响应映射通过结构化 enum 完成（`KiroError` → `axum::Response`）。
- 凭据池 single-flight 刷新：同一凭据并发刷新串行化，避免浪费 refresh_token。
- `DisabledReason` 区分 `Manual` / `TooManyFailures` / `TooManyRefreshFailures` / `QuotaExceeded` / `InvalidRefreshToken` / `InvalidConfig`。
- 启动期重复 credential id 检测：拒绝启动以避免凭据被静默覆盖。

### Improvements
- Stats 30s debounce：避免高并发下每请求落盘；进程退出前显式 `flush_stats` 保证最后一窗口不丢。
- Priority 模式候选顺序稳定化：tied priority 按 id 二级升序，杜绝 HashMap 顺序非确定性。
- 启动时 `current_id` 初始化为最低 priority 启用凭据。
- Token 即将过期（< 10min）时记录 `tracing::warn`，便于运维提前感知。
- `400 + CONTENT_LENGTH_EXCEEDS_THRESHOLD` 与 `400 + Input is too long` 在 `infra` 层识别为 `ProviderError::ContextWindowFull` / `InputTooLong`，HTTP 响应统一为 `400 invalid_request_error`。
- API Key 启动日志仅显示前 4 字符 + 长度，避免半个 key 落入日志。
