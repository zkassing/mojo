# 项目规则

## 发版流程（Mojo 桌面端）

发版 = 同步版本号 + 打 `v*` tag 并推送，其余由 GitHub Actions 自动完成
（构建三平台 → 签名更新包 → 生成 `latest.json` → 发布 GitHub Release）。

### 步骤

1. **同步版本号**（两处必须一致，tag 与此版本对应）：
   - `src-tauri/tauri.conf.json` 的 `version`
   - `package.json` 的 `version`
2. 提交并推送到 `main`。
3. 打 tag 并推送（版本号去掉前导 `v` 后须与上面一致）：

   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

4. CI 自动：构建、用 `TAURI_SIGNING_PRIVATE_KEY` 签名更新包、
   生成更新清单 `latest.json`、发布到 GitHub Releases。
   已安装的 App 启动时（及「服务与状态」页手动检查）即可检测并自动更新。
5. 预发布版本用带后缀的 tag：`v0.2.0-rc.1` / `-beta` / `-alpha`，
   工作流会自动标记为 prerelease。
6. 日常 push 到 `main` 只构建、产物存为 Actions Artifacts（保留 14 天），不发版。

### 更新机制要点

- 端点固定取最新正式版：
  `https://github.com/zkassing/mojo/releases/latest/download/latest.json`
  ，因此 Release 资产使用固定文件名（`mojo-macos-arm64.app.tar.gz` 等），
  改发版逻辑时不要改成带版本号的名字，否则更新器找不到。
- 更新签名密钥对：私钥存于 GitHub Secret `TAURI_SIGNING_PRIVATE_KEY`
  （本机备份 `~/.config/mojo/keys/mojo.key`，**严禁入库**）；
  公钥写在 `tauri.conf.json` 的 `plugins.updater.pubkey`。
  私钥/密码一旦丢失，后续更新将无法通过校验，需重新生成并让用户手动装一次。

### 两个前提（排查“为什么没更新/打不开”时先看这里）

1. **首个带更新插件的版本必须用户手动安装一次**：旧版本没有 updater 插件，
   检测不到更新属正常；从该版本起，之后的新版本才能自动更新。
2. **macOS 产物未做代码签名/公证**：自动更新替换 `.app` 后可能被 Gatekeeper
   拦截，需右键打开或 `xattr -dr com.apple.quarantine`。要做到无感需接入
   Apple Developer 签名（参考 `scripts/build-signed.sh`）。
