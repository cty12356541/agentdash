# bin/ — 预构建二进制占位(不入库)

本目录**只放这份说明,不放任何二进制**。agentdash 的预构建二进制一律经
**GitHub Releases** 分发,不提交进仓库,也**不内嵌进插件包**:插件
(`kits/claude-code/`)只携带 `hooks/hooks.json` 与 `skills/agentdash/`,
hook 运行时直调 PATH 上的 `agentdash`。

## 获取方式

| 平台 | 方式 |
|---|---|
| Windows | 从 Releases 下载对应 target triple 的压缩包解压进 PATH(**release 资产后续手动挂**;在此之前以 `cargo install --path .` 为准) |
| 类 Unix / macOS | Releases 对应 triple 压缩包,或直接 `cargo install --path .` |

## 本地摆放(可选)

把下载的二进制放进本目录(或任意 PATH 目录)并自检:

```powershell
.\bin\agentdash.exe --version
```

自检通过后,hook(`agentdash hook <event>`)即可消费会话事件。
