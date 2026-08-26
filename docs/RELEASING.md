# 发版与 Docker 镜像发布流程

本项目通过 GitHub Release 自动构建并发布 Docker 镜像到 Docker Hub。工作流文件为 `.github/workflows/publish-docker.yml`。

## 发布约定

- 版本遵循 SemVer，例如 `0.2.0`。
- Git tag 必须为对应版本加 `v` 前缀，例如 `v0.2.0`。
- `Cargo.toml` 中的 `package.version` 必须和 tag 去掉 `v` 后完全一致。
- Docker 镜像始终从 GitHub Release 所选 tag 指向的 commit 构建，而不是从默认分支的最新 commit 构建。
- 正式版会更新 `latest`；GitHub 中标记为 prerelease 的版本不会覆盖 `latest`。

## 一、准备发版 commit

先在准备发布的分支更新 `Cargo.toml`：

```toml
[package]
version = "0.2.0"
```

然后更新 `Cargo.lock` 并完成本地校验：

```bash
cargo check --locked
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

提交所有发版内容。这个 commit 就是最终用于构建 Docker 镜像的 commit：

```bash
git add Cargo.toml Cargo.lock
git add <本次发版的其他文件>
git commit -m "release: v0.2.0"
git push origin <当前分支>
```

建议先通过 Pull Request 将该 commit 合并到默认分支，再进行下一步。

## 二、为发版 commit 创建 tag

确认当前 HEAD 是要发布的 commit：

```bash
git log -1 --oneline
```

创建带说明的 tag 并推送：

```bash
git tag -a v0.2.0 -m "v0.2.0"
git push origin v0.2.0
```

如果需要为一个较早的指定 commit 发版，应明确指定 commit SHA：

```bash
git tag -a v0.2.0 <commit-sha> -m "v0.2.0"
git push origin v0.2.0
```

不要移动或覆盖已经发布的 tag。如果 tag 指错，应删除尚未发布的错误 tag，并创建一个新的正确版本号。

## 三、发布 GitHub Release

1. 打开仓库的 **Releases** 页面。
2. 点击 **Draft a new release**。
3. 在 **Choose a tag** 中选择刚推送的 `v0.2.0`。
4. 检查 Release 的 target commit 是否正是上一步确认的发版 commit。
5. 填写变更说明。
6. 正式版直接点击 **Publish release**；候选版需要勾选 **Set as a pre-release** 后再发布。

`published` 事件会启动 `Publish Docker image` 工作流。工作流首先验证：

1. checkout 的 commit 与 Release tag 指向的 commit 完全相同；
2. tag 去掉 `v` 后与 `Cargo.toml` 版本完全相同。

任一验证失败都不会发布镜像。

## 四、自动发布内容

假设 GitHub 仓库名为 `openlark-bitable-service`，发布 `v0.2.0` 后，镜像会推送到 Docker Hub 用户 `trihlp`：

```text
trihlp/openlark-bitable-service:0.2.0
trihlp/openlark-bitable-service:0.2
trihlp/openlark-bitable-service:latest
trihlp/openlark-bitable-service:sha-<短提交号>
```

主版本为 `0` 时不会发布含义过宽的 `:0` 标签；从 `v1.0.0` 开始会额外发布 `:1` 这样的主版本标签。工作流同时发布 `linux/amd64` 和 `linux/arm64`。候选版本只发布完整 SemVer 与 commit 标签，不更新 `latest`。

## 五、Docker Hub 准备

在 Docker Hub 中完成以下准备：

1. 在 Docker Hub 创建与 GitHub 仓库同名的 repository，例如 `openlark-bitable-service`。
2. 创建一个具有该 repository 写入权限的 Docker Hub Access Token。
3. 在 GitHub 仓库 **Settings → Secrets and variables → Actions** 中创建 Repository secret：

   ```text
   Name: DH_TOKEN
   Secret: <Docker Hub Access Token>
   ```

工作流固定使用 `trihlp` 作为 Docker Hub 用户名，并使用当前 GitHub 仓库名的小写形式作为 Docker Hub repository 名。Docker Hub repository 的公开或私有状态由 Docker Hub 中的 repository 设置决定。

工作流使用 GitHub Actions Cache 的 BuildKit 缓存（`scope=3h-reader`），并在 Dockerfile 中缓存 Cargo registry、Git 依赖和按目标平台隔离的 `target` 目录。Dockerfile 还会先单独编译依赖，再编译应用源码；因此 Cargo.lock 未变化时，后续发版通常只需重新编译本项目代码。首次构建或依赖升级仍可能较慢，多架构构建会分别维护 `amd64` 和 `arm64` 的 target 缓存。

工作流的 `GITHUB_TOKEN` 只具有 `contents: read` 权限，用于读取 Release 对应源码；镜像推送只使用 `DH_TOKEN`。

## 六、拉取和运行镜像

公开镜像可以直接拉取：

```bash
docker pull trihlp/<repository>:0.2.0
```

私有镜像需要先用 Docker Hub Access Token 登录：

```bash
echo "$DH_TOKEN" | docker login -u trihlp --password-stdin
```

运行时再注入业务凭据，不要把 `.env` 或任何密钥构建到镜像中：

```bash
docker run --rm \
  --name openlark-bitable-service \
  --env-file .env \
  -p 8080:8080 \
  trihlp/<repository>:0.2.0
```

验证服务：

```bash
curl http://127.0.0.1:8080/health
```

## 七、失败处理

在 GitHub 仓库的 **Actions → Publish Docker image** 中查看日志：

- **版本不一致**：修正 `Cargo.toml`，提交后使用新的版本号重新发版；不要覆盖已发布 tag。
- **Docker Hub 登录失败**：确认 GitHub Actions Secret 名称为 `DH_TOKEN`，并检查 token 是否仍有效。
- **Docker Hub push denied**：确认 `DOCKERHUB_USERNAME` 正确、目标 repository 已创建，并且 `DH_TOKEN` 具有写入权限。
- **Docker 构建失败**：先在本地运行 `docker build -t openlark-bitable-service:test .`。
- **某个平台构建失败**：查看 Buildx/QEMU 日志，确认依赖能在 `amd64` 与 `arm64` 上构建。
- **需要重试瞬时错误**：在失败的工作流页面选择 **Re-run failed jobs**；它仍然使用同一个 Release tag commit。
