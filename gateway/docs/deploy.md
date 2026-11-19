# 部署教程

这份教程讲怎么在一台 Linux 服务器上部署 Baize：网关（Rust）、PostgreSQL、网页版，以及用 HTTPS 对外提供服务。桌面应用的构建见 `web/README.md`。

> 文中 `codex.example.com` 是示例域名，换成你自己的。所有命令默认在服务器上以 root 执行。

## 0. 你需要准备什么

| 项目 | 说明 |
|---|---|
| Linux 服务器 | Ubuntu 22.04 / 24.04 或同类发行版，2 核 4G 起步 |
| 域名 | 一个能解析到服务器的域名，用来获取 HTTPS 证书 |
| PostgreSQL 16 | 可以装在同一台机器上 |
| Rust 工具链 | 只在构建网关时需要，可以在别的机器上构建后拷贝二进制 |
| Node.js 20+ | 只在构建网页版时需要 |
| OpenSSH 客户端 | 使用 SSH 仓库时需要：`ssh`、`ssh-keygen`、`ssh-keyscan` |
| `git` | 代码会话需要 |
| `codex` 二进制 | 运行代码会话和聊天需要。获取方式见第 3 步 |
| Nginx 和 certbot | 反向代理和 HTTPS 证书 |

装好基础软件：

```sh
apt-get update
apt-get install -y postgresql nginx certbot python3-certbot-nginx git openssh-client curl
```

## 1. 准备 PostgreSQL

```sh
sudo -u postgres psql <<'SQL'
CREATE USER codex WITH PASSWORD 'CHANGE_ME_DB_PASSWORD';
CREATE DATABASE codex_gateway OWNER codex;
SQL
```

数据库连接串就是：

```
postgres://codex:CHANGE_ME_DB_PASSWORD@127.0.0.1:5432/codex_gateway
```

网关启动时会自动执行 `migrations/` 下的迁移，不需要手动建表。

## 2. 构建网关

在有 Rust 的机器上（服务器本身也可以）：

```sh
git clone <你的仓库地址> codex
cd codex/gateway
cargo build --release
```

产物是 `target/release/codex-gateway`。

## 3. 准备 `codex` 二进制（可选，但聊天和代码会话需要）

网关通过 `GATEWAY_CODEX_BIN` 启动 `codex app-server`。没有它，网关照常运行，但发送消息会报 "environment has no endpoint and local launching is disabled"。

1. 按 Codex 项目的说明构建或安装 `codex` 二进制，放到例如 `/opt/codex/bin/codex`。
2. 在环境变量里设置 `GATEWAY_CODEX_BIN=/opt/codex/bin/codex` 和 `GATEWAY_CODEX_HOME=/var/lib/codex-gateway/codex-home`。

网关会为每个环境在 `GATEWAY_CODEX_HOME` 下建目录，并在那里写 app-server 日志。

## 4. 安装网关

仓库里的 `gateway/deploy/install.sh` 会创建系统用户、安装二进制、写入 systemd 服务并设为开机启动：

```sh
cd codex/gateway
bash deploy/install.sh target/release/codex-gateway
```

脚本第一次运行时会把模板复制到 `/etc/codex-gateway/env`，然后提示你编辑它。

## 5. 配置环境变量

编辑 `/etc/codex-gateway/env`（权限 0600，只有 root 能读）。最少需要：

```sh
DATABASE_URL=postgres://codex:CHANGE_ME_DB_PASSWORD@127.0.0.1:5432/codex_gateway
GATEWAY_ADDR=127.0.0.1:8080
GATEWAY_PUBLIC_URL=https://codex.example.com
GATEWAY_SECRET_KEY=<64 位十六进制>
```

生成密钥：

```sh
openssl rand -hex 32
```

各项说明：

| 变量 | 作用 |
|---|---|
| `GATEWAY_ADDR` | 网关监听地址。只监听本机，由 Nginx 对外 |
| `GATEWAY_PUBLIC_URL` | 网站的公开地址，用于 OAuth 回调。必须是 https 地址 |
| `GATEWAY_DEV_LOGIN` | 不设置或设为 `0`。`1` 时任何人只凭邮箱就能登录 |
| `GATEWAY_SECRET_KEY` | 加密保存的令牌和 SSH 私钥。**丢失或更换后，已保存的 GitHub 连接、仓库令牌和 SSH 密钥都无法解密，需要重新连接和添加。请备份这个值。** |
| `GATEWAY_CODEX_BIN` / `GATEWAY_CODEX_HOME` | 第 3 步的 `codex` 路径和目录 |
| `GATEWAY_CODE_DIR` | 代码会话的克隆和工作区目录，默认 `code-data`（相对工作目录）。生产环境建议设为 `/var/lib/codex-gateway/code` |
| `GATEWAY_STORAGE` | `local`（默认，保存在 `GATEWAY_STORAGE_DIR`）或 `s3` |

可选项：

- **GitHub 账号登录**：在 GitHub 注册 OAuth App，回调地址填 `https://codex.example.com/api/v1/github/callback`，然后设置 `GATEWAY_GITHUB_CLIENT_ID` 和 `GATEWAY_GITHUB_CLIENT_SECRET`。不设置时，GitHub 连接按钮会报"未配置"。
- **网页版地址**：授权完成后跳转回的地址。不设置时用 `GATEWAY_PUBLIC_URL`，这正是同一域名部署时想要的。
- **S3 存储**：设置 `GATEWAY_STORAGE=s3`、`GATEWAY_S3_BUCKET`，并按需设置 `GATEWAY_S3_ENDPOINT`、`GATEWAY_S3_REGION`、`GATEWAY_S3_FORCE_PATH_STYLE`。AWS 凭据用标准的环境变量或实例角色。
- **SSH 仓库**：不需要额外变量，只需要服务器上装好 OpenSSH 客户端。
- **代码会话的页面预览**：设置 `GATEWAY_SCREENSHOT_CMD="node /opt/codex/gateway/tools/screenshot.cjs"`。服务器上需要 Node.js、Playwright 和 Chromium（`npx playwright install chromium`）。`tools/screenshot.cjs` 需要随网关一起拷到服务器。不设置时，预览按钮会报"预览未启用"。预览只能打开会话工作区里的 HTML 文件，资源路径要是相对的。
- **文件远程**：`GATEWAY_ALLOW_FILE_REMOTES` 必须保持不设置。开启后，任何能添加仓库的人都能让网关读取本机文件。

保存后启动：

```sh
systemctl start codex-gateway
systemctl status codex-gateway --no-pager
journalctl -u codex-gateway -n 50 --no-pager
```

看到 `gateway listening` 字样说明启动成功。迁移会在这时自动执行。

验证：

```sh
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8080/api/v1/me
```

返回 `401` 是正常的（没有登录令牌）。返回 `000` 说明没有在监听，去看日志。

## 6. 构建网页版

在有 Node.js 的机器上：

```sh
cd codex/web
npm ci
npm run build
```

产物在 `web/dist/`。把它拷到服务器，例如 `/var/www/codex`：

```sh
mkdir -p /var/www/codex
rsync -a --delete dist/ root@服务器:/var/www/codex/
```

网页版默认访问当前页面所在的域名，所以不需要在构建时设置网关地址。只有当网页版和网关不在同一个域名下时，才需要设置 `VITE_GATEWAY_URL`。

## 7. 配置 Nginx 和 HTTPS

创建 `/etc/nginx/sites-available/codex`：

```nginx
server {
    listen 80;
    server_name codex.example.com;

    root /var/www/codex;
    index index.html;

    # 网页版的单页应用：找不到的路径都回到 index.html
    location / {
        try_files $uri /index.html;
    }

    # 网关接口
    location /api/ {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        client_max_body_size 100m;   # 附件上传的大小
        proxy_read_timeout 300s;     # 长回合不能被提前断开
    }

    # 实时连接
    location /ws {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_read_timeout 3600s;
    }
}
```

启用并签发证书：

```sh
ln -s /etc/nginx/sites-available/codex /etc/nginx/sites-enabled/codex
nginx -t && systemctl reload nginx
certbot --nginx -d codex.example.com
```

`certbot` 会把站点改成 HTTPS 并设置自动续期。之后 `GATEWAY_PUBLIC_URL` 必须是 `https://codex.example.com`。

打开 `https://codex.example.com`，应该看到登录页。

## 8. 创建第一个账号

网页版的注册入口在登录页的 "Create one"。每个注册的账号都会得到自己的个人工作区，成为它的所有者。关闭开发登录后，只能用用户名和密码注册和登录。

## 9. 升级

```sh
cd codex && git pull
cd gateway && cargo build --release
systemctl stop codex-gateway
install -m 0755 target/release/codex-gateway /opt/codex/gateway/codex-gateway
systemctl start codex-gateway
cd ../web && npm ci && npm run build && rsync -a --delete dist/ /var/www/codex/
```

迁移是向前的，启动时自动执行。升级前备份数据库：

```sh
sudo -u postgres pg_dump codex_gateway | gzip > /var/backups/codex_gateway-$(date +%F).sql.gz
```

## 10. 备份和需要保留的东西

| 内容 | 位置 | 说明 |
|---|---|---|
| 数据库 | `codex_gateway` | 所有聊天、工作区、仓库记录 |
| `GATEWAY_SECRET_KEY` | `/etc/codex-gateway/env` | 没有它，加密的令牌都无法使用 |
| 附件 | `GATEWAY_STORAGE_DIR`，或 S3 桶 | 聊天里的文件 |
| 代码克隆和工作区 | `GATEWAY_CODE_DIR` | 可以重新克隆，但未提交的改动只在这里 |
| `codex` 数据 | `GATEWAY_CODEX_HOME` | 运行时状态 |

## 11. 常见问题

- **发送消息报 "environment has no endpoint and local launching is disabled"**：没有设置 `GATEWAY_CODEX_BIN`，或者路径不对。见第 3 步。
- **GitHub 连接后回到页面显示"无法连接"**：检查 `GATEWAY_PUBLIC_URL` 是否是 https 地址，以及 GitHub OAuth App 的回调地址是否完全一致。
- **SSH 仓库检查失败，提示 Permission denied**：部署密钥没有加到仓库上，或者没有勾选写权限。
- **SSH 仓库报 host key 变化**：服务器换了主机密钥。在仓库上点"检查主机密钥"，核对新指纹后确认。不要直接跳过。
- **启动后 `401` 以外的状态码**：看 `journalctl -u codex-gateway`。数据库连接失败最常见，检查 `DATABASE_URL` 和 PostgreSQL 是否在运行。
- **上传大文件失败**：调大 Nginx 的 `client_max_body_size`。

## 12. 还没有在真实环境里验证的部分

以下步骤只在本地模拟环境里测过，正式部署前请在你自己的环境里再走一遍：

- GitHub OAuth 的完整流程（真实 GitHub 账号）。
- GitLab 等 git 服务的 HTTPS 令牌和 SSH 部署密钥（真实服务器）。
- 配合真实 `codex` 二进制的聊天和代码会话（自动提交、工作日志）。
- 桌面应用（Tauri）的构建和安装，见 `web/README.md`。
