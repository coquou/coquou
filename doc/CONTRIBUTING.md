# 参与贡献

欢迎提 issue 和 PR！

## 开始之前

- 环境、常用命令、目录结构和原理见 [development.md](development.md)。
- 代码约定和提交前检查见根目录的 [AGENTS.md](../AGENTS.md)，对人和 AI 代理同样适用。
- 宠物包格式见 [pet-format.md](pet-format.md)；格式改动要同步更新它，并保持与 Codex 宠物包兼容。

## 提交前请确认

```bash
cargo clippy --all-targets && cargo test
```

改了 AI 孵化相关代码时，再跑一遍离线自测（不需要 API key，不花钱），见 [development.md](development.md)「AI 孵化」，日志里出现 `selftest PASS` 即通过。

## 授权

- 提交的代码以 [MIT](../LICENSE) 授权。
- 提交的宠物素材（预设宠物的图片等）以 [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) 授权，请确认你有权以此协议发布；不接受未经授权的他人角色或作品。宠物包不接受任何形式的脚本。
