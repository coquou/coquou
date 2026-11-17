# 发版与自动更新

Coquou 用 Tauri 官方的 `tauri-plugin-updater` 自动更新：

- 应用启动 20 秒后检查一次，之后每 6 小时检查一次（主窗口 → 关于 →「自动检查更新」可关）。
- 检查地址：`https://github.com/coquou/coquou/releases/latest/download/latest.json`，即 GitHub 上最新一个正式 Release 里的 `latest.json`。
- 发现新版本时，宠物冒气泡提示，托盘出现「有新版本 x.y.z，查看更新…」，主窗口「关于」页显示更新说明。用户点「更新并重启」才会安装，**不会静默安装**。
- 下载的更新包要用 `tauri.conf.json` 里的公钥验签，签名不对就拒绝安装。

## 密钥

| 东西 | 在哪 | 说明 |
|---|---|---|
| 更新签名私钥 | `~/.tauri/coquou-updater.key`（发版用的 Mac，已备份）；GitHub 机密 `TAURI_SIGNING_PRIVATE_KEY` | **丢了它，已经装好的 Coquou 就再也收不到更新**（只能让用户手动重装带新公钥的版本） |
| 更新签名公钥 | `~/.tauri/coquou-updater.key.pub` | 已写进 `tauri.conf.json` 的 `plugins.updater.pubkey` |
| `Coquou Self-Signed` 证书和私钥 | 发版用的 Mac 的钥匙串，备份的 `.p12`；GitHub 机密 `MACOS_CERTIFICATE`（`.p12` 的 base64）和 `MACOS_CERTIFICATE_PASSWORD` | macOS 应用签名身份（见下文「让钥匙串在更新后不再弹框」）。丢了只能新建一个，所有用户更新到用新证书签名的版本时，都要再点一次钥匙串授权 |

私钥目前没有密码。也可以设一个密码（`npx @tauri-apps/cli@2 signer generate -w <路径> -p <密码>` 重新生成，同时更新公钥），再在工作流里通过 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 传入。

### 设置 GitHub 机密（只需一次）

在发版用的 Mac 上，用能管理 `coquou/coquou` 的账号运行（`.p12` 换成备份文件的路径，第三条会提示输入导出 `.p12` 时设的密码）：

```bash
export GH_TOKEN=$(gh auth token -u continuationcs)
gh secret set TAURI_SIGNING_PRIVATE_KEY --repo coquou/coquou < ~/.tauri/coquou-updater.key
base64 -i Coquou-Self-Signed.p12 | gh secret set MACOS_CERTIFICATE --repo coquou/coquou
gh secret set MACOS_CERTIFICATE_PASSWORD --repo coquou/coquou
```

换了私钥或证书时重新设置对应的机密。

## 发版步骤

发版由 GitHub Actions 的 **Deploy** 工作流（`.github/workflows/deploy.yml`）自动完成：推送 `v1.2.3` 这样的标签后，它在 macOS 上打通用版（Intel + Apple 芯片）的 DMG 和更新包，在 Windows 上打 NSIS 安装包，都用更新私钥签名，然后上传、生成 `latest.json`，发布为最新的正式 Release。

1. **改版本号**，提交并推送：

   ```bash
   python3 src/tools/release.py bump 0.1.1
   git commit -am "Version 0.1.1" && git push
   ```

2. **打标签并推送。** 标签的说明就是更新说明，会显示在 Release 页面和主窗口「关于」页里：

   ```bash
   git tag -a v0.1.1 -m "这一版的更新说明"
   git push origin v0.1.1
   ```

3. 在 GitHub 的 Actions 页看 Deploy 跑完（大约 15–20 分钟）。标签和 `tauri.conf.json` 里的版本号不一致时会直接失败；任何一个平台失败都不会发布。

工作流跑的就是 `src/tools/release.py` 的 `build`、`upload`、`publish`：产物先放在 `out/<版本号>/`，再上传到草稿 Release，`publish` 检查每个更新包都有签名、写出 `latest.json` 后才把草稿改成正式版。工作流失败时，修好后删掉远端标签（`git push origin :v0.1.1`）和草稿 Release，重新打标签推送即可。

## macOS：让钥匙串在更新后不再弹框

AI 接口的 key 存在钥匙串里。没有固定签名身份时，每次更新后应用的签名都会变，macOS 会把它当成别的应用，读 key 时又弹一次授权框。用一个固定的自签名证书签名就能解决（免费；也不需要 Apple 开发者账号）：

1. 打开「钥匙串访问」→ 菜单「钥匙串访问」→「证书助理」→「创建证书…」。
2. 名称填 `Coquou Self-Signed`，身份类型「自签名根证书」，证书类型选「**代码签名**」，创建。
3. `release.py build` 会自动用钥匙串里的 `Coquou Self-Signed` 给应用签名（Deploy 工作流先把 `MACOS_CERTIFICATE` 导入一个临时钥匙串）；在本机第一次签名时 macOS 会问 `codesign` 能否使用这个私钥，选「始终允许」。找不到这张证书时 `build` 会停下，以免发出签名不一样的版本（那样所有用户更新后都要再授权一次）；确实要发未签名版本时加 `--unsigned`。用别的证书时设 `COQUOU_SIGN_IDENTITY="<证书名>"`。

4. 备份：在「钥匙串访问」的「我的证书」里找到 `Coquou Self-Signed`，展开确认下面带着私钥，右键 →「导出」，存成 `.p12` 并设一个密码，和更新签名私钥放在一起保管。只导出证书（`.cer`）没有用，签名需要私钥。这个 `.p12` 也是 GitHub 机密 `MACOS_CERTIFICATE` 的来源。

**换发版用的 Mac**：发版在 GitHub 上进行，不依赖哪台 Mac。要在新 Mac 上本地打签名版时，把 `~/.tauri/coquou-updater.key` 拷过去，双击 `.p12` 导入到「登录」钥匙串，`release.py` 就会自动用它。不要在新机器上重新创建同名证书：名字一样，身份却是新的，等于换了证书。

用户换 Mac 不受影响：证书不用装到用户电脑上。用户在新电脑上装好 Coquou、重新填一次 key（或用迁移助理连钥匙串一起搬过去）就行。

这个证书只解决钥匙串问题。首次安装时 Gatekeeper 仍会拦（要右键 →「打开」）；要彻底解决，需要 Apple 开发者账号签名并公证。

## Windows 注意事项

- 没有代码签名证书时，首次安装会弹 SmartScreen「Windows 已保护你的电脑」，要点「更多信息 →仍要运行」。之后的自动更新走应用内安装（`installMode: passive`，只显示进度条）。
- 安装包是按当前用户安装的 NSIS，不需要管理员权限。

## 本地测试更新流程

不用发 Release，也能在本机模拟一次完整更新（只在调试版可用：允许 http 地址，并能自动安装）：

```bash
# 1) 打一个当前版本（0.0.1）的调试包，复制到别处当「已安装」的版本
npx @tauri-apps/cli@2 build --debug --bundles app
cp -R target/debug/bundle/macos/Coquou.app /tmp/coquou-e2e/
# 2) 打一个带签名的 0.0.2 更新包
TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/coquou-updater.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
  npx @tauri-apps/cli@2 build --debug --bundles app --config '{"version":"0.0.2","bundle":{"createUpdaterArtifacts":true}}'
# 3) 用 Coquou.app.tar.gz 和 .sig 写一个 latest.json，用 python3 -m http.server 提供出来，然后：
COQUOU_UPDATE_URL=http://127.0.0.1:8899/latest.json COQUOU_UPDATE_AUTOINSTALL=1 \
  /tmp/coquou-e2e/Coquou.app/Contents/MacOS/Coquou
```

日志里依次出现 `update available: 0.0.1 → 0.0.2`、`update installed, restarting`，`/tmp/coquou-e2e/Coquou.app` 的版本变成 0.0.2。把更新包改坏后再试，应当出现 `signature verification failed`，并且不会安装。

## 仓库与旧名

仓库在 [coquou/coquou](https://github.com/coquou/coquou)，更新检查地址、发版脚本里的 `REPO`、主窗口里的「项目主页 / 反馈问题 / 发布记录」链接都指向这里。

改名前的仓库 `continuationcs/perchy` 是另一个独立仓库（不是迁移过来的，没有地址重定向）。它没发布过任何 Release，所以不会有已安装的版本去那里检查更新；建议在 GitHub 上把它归档（Archive），README 指向新仓库。
