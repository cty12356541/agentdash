# bin/ — 预构建二进制占位(不入库)

本目录**只放这份说明,不放任何二进制**。agentdash 的预构建二进制一律经
**GitHub Releases** 分发,不提交进仓库,也**不内嵌进插件包**:插件
(`kits/claude-code/`)只携带 `hooks/hooks.json` 与 `skills/agentdash/`,
hook 运行时直调 PATH 上的 `agentdash`。

## 获取方式

Release 资产由 `.github/workflows/release.yml`(W4-006)在推送 `v*` tag 时
**自动构建并挂载**,覆盖四个 target triple:

| 平台 | target triple | 资产 |
|---|---|---|
| Linux x64 | `x86_64-unknown-linux-gnu` | `agentdash-v<x>-x86_64-unknown-linux-gnu.tar.gz` |
| macOS (Intel) | `x86_64-apple-darwin` | `agentdash-v<x>-x86_64-apple-darwin.tar.gz` |
| macOS (Apple Silicon) | `aarch64-apple-darwin` | `agentdash-v<x>-aarch64-apple-darwin.tar.gz` |
| Windows x64 | `x86_64-pc-windows-msvc` | `agentdash-v<x>-x86_64-pc-windows-msvc.zip` |

在 Releases 页按平台下载解压,放入 PATH 即可;`agentdash --version` 自检。
在此之前(仓库首个 Release 前)以 `cargo install --path .` 为准。

## 本地摆放(可选)

把下载的二进制放进本目录(或任意 PATH 目录)并自检:

```powershell
.\bin\agentdash.exe --version
```

自检通过后,hook(`agentdash hook <event>`)即可消费会话事件。
