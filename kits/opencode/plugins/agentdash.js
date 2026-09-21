// agentdash · opencode 插件(W7-004 + ADE attribution)
// tool.execute.after → agentdash hook posttooluse;session.status(idle) → stop。
// 与 claude-code/codex 两 kit 写同一份 <repo>/.agentdash/(台账一致性),
// 事件带 --host opencode 归属(多宿主混用可归因)。sessionID 写入载荷 session_id。
//
// 降级铁律:本插件任何失败(agentdash 缺失 / spawn 失败 / 写流失败)一律
// 静默吞掉,绝不阻塞宿主工具执行。

export const Agentdash = async ({ directory }) => {
  const feed = (event, payload) => {
    try {
      const body = JSON.stringify({
        hook_event_name: event,
        cwd: directory,
        ...payload,
      })
      const proc = Bun.spawn(
        ["agentdash", "hook", "--host", "opencode", event],
        { stdin: "pipe", stdout: "ignore", stderr: "ignore" },
      )
      proc.stdin.write(body)
      proc.stdin.end()
      proc.exited.catch(() => {})
    } catch {
      // 降级:静默,宿主零感知
    }
  }

  return {
    "tool.execute.after": async (input, output) => {
      feed("posttooluse", {
        session_id: input.sessionID,
        tool_name: input.tool,
        tool_input: output.args ?? {},
        tool_response: output.result ?? {},
      })
    },
    // session.idle 已废弃;idle/busy/retry 走 session.status,仅 idle 折叠门禁。
    "session.status": async (input) => {
      const typ = input?.status?.type || input?.status
      if (typ === "idle") {
        feed("stop", { session_id: input.sessionID })
      }
    },
  }
}
