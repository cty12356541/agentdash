// agentdash · opencode 插件(W7-004)
// tool.execute.after → agentdash hook posttooluse;session.idle → stop。
// 与 claude-code/codex 两 kit 写同一份 <repo>/.agentdash/(台账一致性),
// 事件带 --host opencode 归属(多宿主混用可归因)。
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
    // 工具执行后:opencode 的 input.tool 为小写名("bash"/"read"/…),
    // agentdash 的 gate 匹配对大小写容忍,无需归一
    "tool.execute.after": async (input, output) => {
      feed("posttooluse", {
        tool_name: input.tool,
        tool_input: output.args ?? {},
        tool_response: output.result ?? {},
      })
    },
    // turn 结束(session.idle):折叠在途 gate(与宿主 Stop 事件同语义)
    "session.idle": async () => {
      feed("stop", {})
    },
  }
}
