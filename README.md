# OpenLark Bitable Service

一个使用 Salvo 与 OpenLark 0.20.0 构建的只读 Web 服务。它读取指定飞书多维表格的全部记录，将原始 JSON 永久缓存在内存中，并用 AES-256-GCM 加密后返回。

## 配置

复制 `.env.example` 为 `.env`，至少设置：

```dotenv
authToken=请替换为至少16字符的随机令牌
noencrypt_authToken=请替换为另一个至少16字符的随机令牌
encryptToken=请替换为加密密钥材料
APP_ID=cli_xxx
APP_KEY=xxx
```

OpenLark 通过 Git tag 固定在 `v0.20.0`。依赖启用了用户要求的 `auth`、`docs-full`，以及接收飞书长连接事件所必需的 `websocket` feature。

## 启动

```bash
cargo run --release
```

默认监听 `0.0.0.0:8080`。健康检查为 `GET /health`。

也可以使用 Docker：

```bash
docker build -t openlark-bitable-service .
docker run --rm --env-file .env -p 8080:8080 openlark-bitable-service
```

## 调用

```bash
curl "http://127.0.0.1:8080/bitable/US52wnvfniIRC0kAiyVcaltLnrh?table=tblQH0TVkKNoJ2Ef" \
  -H "Authorization: Bearer $authToken"
```

也兼容 `auth: <authToken>` 请求头。成功响应：

```json
{"data":"base64url..."}
```

`data` 的二进制格式为 `nonce(12 bytes) || ciphertext || GCM tag(16 bytes)`，整体使用无填充 Base64 URL 编码。AES-256 密钥为 `SHA-256(encryptToken UTF-8 bytes)`。解密后的内容是扁平记录数组 JSON：每条记录直接以字段名为键，例如 `{"视频ID":"7675309357916572974","点赞量":28}`；飞书字段值中的 `text`、`type` 包装会被移除。

使用 `noencrypt_authToken` 时，`data` 不加密，直接返回相同的扁平 JSON：

```bash
curl "http://127.0.0.1:8080/bitable/US52wnvfniIRC0kAiyVcaltLnrh?table=tblQH0TVkKNoJ2Ef" \
  -H "Authorization: Bearer $noencrypt_authToken"
```

```json
{"data":[{"视频ID":"7675309357916572974","点赞量":28}]}
```

`authToken` 与 `noencrypt_authToken` 都至少需要 16 个字符且不能相同。未加密 Token 拥有读取原始业务数据的能力，必须按敏感凭据管理。

服务允许任意域名、方法和请求头跨域，但读取接口始终校验认证头。不要在不可信的浏览器前端代码中暴露 `authToken` 或 `encryptToken`。

## 缓存与飞书事件

- 缓存键为 `(bitable app_token, table_id)`，无 TTL；同一表首次并发读取会通过逐键锁合并为一次飞书 API 请求。
- 每个 bitable 的首次请求会先通过 OpenLark 查询云文档事件订阅状态；未订阅时调用订阅接口，随后在内存订阅管理表中登记该 bitable 及其 table。订阅检查失败只记录告警，不阻断多维表格读取；服务运行期间同一 bitable 不会重复调用订阅接口。
- 服务通过 OpenLark WebSocket 接收 `drive.file.bitable_record_changed_v1`。
- 从收到首个记录变更开始收集受影响的 bitable/table；每次新事件都会重新计算静默窗口。
- 连续 120 秒没有新变更后，批量清除一次相关缓存。若事件载荷无法解析或缺少 bitable token，为避免返回陈旧数据会安全地清空全部缓存。
- WebSocket 断开后按 2、4、8……最长 60 秒退避重连。

还需要在飞书开发者后台为应用开通读取多维表格所需权限，将应用添加为目标多维表格的协作者，并在“事件与回调”中选择长连接订阅 `drive.file.bitable_record_changed_v1`。

## 验证

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

## 发版

发布 GitHub Release 后，GitHub Actions 会从该 Release tag 指向的 commit 构建 `linux/amd64`、`linux/arm64` 镜像，并使用仓库 Secret `DH_TOKEN` 推送到 Docker Hub 用户 `trihlp`。完整步骤见 [`docs/RELEASING.md`](docs/RELEASING.md)。
