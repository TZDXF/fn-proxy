# 版本发布

当前发布目标为 Windows x64 NSIS 安装包，同时生成 Tauri updater 签名和 latest.json。更新检查读取本仓库最新正式 GitHub Release；草稿与预发布版本不作为更新候选。设置页可通过签名验证的应用内更新下载并安装，Windows 使用 passive 模式，无需手动卸载旧版本。

## 发布前

1. 同步 package.json、package-lock.json（根版本及 packages[""]）、src-tauri/Cargo.toml、src-tauri/Cargo.lock 与 src-tauri/tauri.conf.json 中的应用版本。
2. 在 docs/releases/vX.Y.Z.md 中编写发布说明（不包含真实账号、凭据或内部网络信息）。
3. 执行 npm run release:check、npm run check、npm run test:rust、npm run rust:check，并完成必要的真实桌面验证。
4. 提交所有待发布文件，确保构建来源与版本标签一致。

## 触发发布

创建并推送对应版本标签，例如：

```powershell
git tag -a v0.1.0 -m "FN Proxy v0.1.0"
git push origin main
git push origin v0.1.0
```

Release Windows 工作流会验证版本与标签、运行前后端检查，构建 NSIS 安装包，生成 updater 签名、latest.json 和 SHA256SUMS.txt，然后创建草稿、上传附件，最后发布为正式版本。只有上传全部成功后才公开发布。latest.json 的下载地址指向当前 tag 下已规范化的安装包附件。

可通过 workflow_dispatch 输入已存在的 tag 重试失败构建。工作流拒绝覆盖已经公开的 Release；附件上传失败时保留草稿。不要移动已发布标签。

所用 Actions 均固定提交 SHA。工作流仅授予发布作业 contents:write；GH_TOKEN 来自仓库内置 GITHUB_TOKEN，不需要额外保存个人 Token。

应用内更新使用 Tauri minisign 公钥验证；公钥保存在 src-tauri/tauri.conf.json。构建与发布需要仓库 Secret TAURI_SIGNING_PRIVATE_KEY，以及可为空的 TAURI_SIGNING_PRIVATE_KEY_PASSWORD。私钥不得提交、写入日志或配置文件；泄露后必须轮换密钥并重新发布完整安装包。Windows 安装包仍未做 Authenticode 代码签名，用户可能看到 SmartScreen 提示。
