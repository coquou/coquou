# 发版与自动更新

Coquou 用 Tauri 官方的 `tauri-plugin-updater` 自动更新：

- 应用启动 20 秒后检查一次，之后每 6 小时检查一次（主窗口 → 关于 →「自动检查更新」可关）。
- 检查地址：`https://github.com/coquou/coquou/releases/latest/download/latest.json`，即 GitHub 上最新一个正式 Release 里的 `latest.json`。
- 发现新版本时，宠物冒气泡提示，托盘出现「有新版本 x.y.z，查看更新…」，主窗口「关于」页显示更新说明。用户点「更新并重启」才会安装，**不会静默安装**。
- 下载的更新包要用 `tauri.conf.json` 里的公钥验签，签名不对就拒绝安装。

## 密钥

| 文件 | 说明 |
|---|---|
| `~/.tauri/coquou-updater.key` | **更新签名私钥**，只在发版用的 Mac 上，不进仓库。**一定要备份**：丢了它，已经装好的 Coquou 就再也收不到更新（只能让用户手动重装带新公钥的版本） |
| `~/.tauri/coquou-updater.key.pub` | 公钥，已写进 `tauri.conf.json` 的 `plugins.updater.pubkey` |
| 钥匙串里的 `Coquou Self-Signed` 证书和私钥 | macOS 应用签名身份（见下文「让钥匙串在更新后不再弹框」），只在发版用的 Mac 上。**也要备份**：丢了只能新建一个，所有用户更新到用新证书签名的版本时，都要再点一次钥匙串授权 |

私钥目前没有密码。也可以设一个密码（`npx @tauri-apps/cli@2 signer generate -w <路径> -p <密码>` 重新生成，同时更新公钥），发版时通过 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 传入。

## 发版步骤

前提：Mac 上有私钥；两台机器都装好 GitHub CLI（`gh`），并能写 `coquou/coquou`。上传和发布命令用对应账号的令牌运行，例如：

```bash
export GH_TOKEN=$(gh auth token -u continuationcs)
```

1. **Mac：改版本号**，提交并推送。

   ```bash
   python3 src/tools/release.py bump 0.1.1
   git commit -am "Release 0.1.1" && git push
   ```

2. **Mac：打包并上传。** 产物是 macOS 更新包（`.app.tar.gz` 和签名）和首次安装用的 DMG。上传后先是草稿 Release，用户还看不到。

   ```bash
   python3 src/tools/release.py build            # 加 --universal 同时支持 Intel Mac
   python3 src/tools/release.py upload --notes-file notes.md
   ```

3. **Windows：拉取、打包、上传 NSIS 安装包。** 不需要私钥。

   ```powershell
   git pull
   py src\tools\release.py build
   py src\tools\release.py upload
   ```

4. **Mac：签名并发布。** 给还没签名的安装包（Windows 的 setup.exe）签名，生成 `latest.json`，再把草稿改成正式的最新 Release。缺某个平台时默认不发布，确认只发部分平台时加 `--force`。

   ```bash
   python3 src/tools/release.py publish
   ```

产物都在 `out/<版本号>/`（不进仓库）。更新说明来自 Release 正文（`--notes` / `--notes-file`），会显示在主窗口「关于」页里。

## macOS：让钥匙串在更新后不再弹框

AI 接口的 key 存在钥匙串里。没有固定签名身份时，每次更新后应用的签名都会变，macOS 会把它当成别的应用，读 key 时又弹一次授权框。用一个固定的自签名证书签名就能解决（免费；也不需要 Apple 开发者账号）：

1. 打开「钥匙串访问」→ 菜单「钥匙串访问」→「证书助理」→「创建证书…」。
2. 名称填 `Coquou Self-Signed`，身份类型「自签名根证书」，证书类型选「**代码签名**」，创建。
3. `release.py build` 会自动用钥匙串里的 `Coquou Self-Signed` 给应用签名；第一次签名时 macOS 会问 `codesign` 能否使用这个私钥，选「始终允许」。钥匙串里找不到这张证书时 `build` 会停下，以免发出签名不一样的版本（那样所有用户更新后都要再授权一次）；确实要发未签名版本时加 `--unsigned`。用别的证书时设 `COQUOU_SIGN_IDENTITY="<证书名>"`。

4. 备份：在「钥匙串访问」的「我的证书」里找到 `Coquou Self-Signed`，展开确认下面带着私钥，右键 →「导出」，存成 `.p12` 并设一个密码，和更新签名私钥放在一起保管。只导出证书（`.cer`）没有用，签名需要私钥。

**换发版用的 Mac**：把 `~/.tauri/coquou-updater.key` 拷过去，在新 Mac 上双击 `.p12` 导入到「登录」钥匙串，`release.py` 就会自动用它。不要在新机器上重新创建同名证书：名字一样，身份却是新的，等于换了证书。

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
