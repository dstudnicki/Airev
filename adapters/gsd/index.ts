import fs from "node:fs";
import path from "node:path";

type ExecResult = {
  stdout: string;
  stderr: string;
  code: number;
  killed?: boolean;
};

type ExtensionContext = {
  cwd: string;
  signal?: AbortSignal;
  hasUI?: boolean;
  ui?: {
    notify(message: string, level: "info" | "warning" | "error" | "success"): void;
  };
};

type ExtensionAPI = {
  on(event: "before_agent_start", handler: (event: { prompt?: string }, ctx: ExtensionContext) => Promise<void>): void;
  on(event: "tool_call", handler: (event: ToolCallEvent, ctx: ExtensionContext) => Promise<void>): void;
  on(event: "agent_end", handler: (event: { messages: unknown[] }, ctx: ExtensionContext) => Promise<void>): void;
  registerCommand(
    name: string,
    options: {
      description: string;
      handler: (args: unknown, ctx: ExtensionContext) => Promise<void>;
    },
  ): void;
  exec(command: string, args: string[], options: { cwd: string; signal?: AbortSignal; timeout?: number }): Promise<ExecResult>;
};

type ToolCallEvent = {
  toolName: string;
  input: Record<string, unknown>;
};

const GLOBAL_REGISTRATION_MARKER = "__patchbayGsdAdapterRegistered";

export default function (pi: ExtensionAPI) {
  const globalState = globalThis as Record<string, unknown>;
  if (globalState[GLOBAL_REGISTRATION_MARKER]) {
    return;
  }
  globalState[GLOBAL_REGISTRATION_MARKER] = true;

  let captureActive = false;

  pi.on("before_agent_start", async (event, ctx) => {
    const result = await runPatchbay(
      pi,
      ctx,
      ["turn", "begin", "--force", "--snapshot-baseline", "--prompt", event.prompt ?? ""],
      { quiet: true },
    );
    captureActive = result.code === 0;
  });

  pi.on("tool_call", async (event, ctx) => {
    if (!captureActive) return;

    const paths = pathsFromToolCall(event);
    for (const filePath of paths) {
      await runPatchbay(pi, ctx, ["touch", filePath], { quiet: true });
    }
  });

  pi.on("agent_end", async (event, ctx) => {
    if (!captureActive) return;

    const summary = summarizeMessages(event.messages);
    await runPatchbay(pi, ctx, ["turn", "end", "--summary", summary], { quiet: true });
    captureActive = false;
  });

  pi.registerCommand("pb-init", {
    description: "Initialize Patchbay storage for the current project.",
    handler: async (_args, ctx) => {
      const result = await runPatchbay(pi, ctx, ["init"], { quiet: true });
      notify(ctx, result.stdout || result.stderr || "Patchbay init completed.", result.code === 0 ? "success" : "error");
    },
  });

  pi.registerCommand("pb-status", {
    description: "Show local Patchbay revision status.",
    handler: async (_args, ctx) => {
      const result = await runPatchbay(pi, ctx, ["status"], { quiet: true });
      notify(ctx, result.stdout || result.stderr || "Patchbay status completed.", result.code === 0 ? "info" : "error");
    },
  });

  pi.registerCommand("pb-revisions", {
    description: "List local Patchbay revisions.",
    handler: async (_args, ctx) => {
      const result = await runPatchbay(pi, ctx, ["revisions"], { quiet: true });
      notify(ctx, result.stdout || result.stderr || "Patchbay revisions completed.", result.code === 0 ? "info" : "error");
    },
  });

  pi.registerCommand("pb-compose", {
    description: "Compose a Patchbay mission from natural-language text. Prefix with --run or --fast when desired.",
    handler: async (args, ctx) => {
      const text = commandText(args).trim();
      if (!text) {
        notify(ctx, "Usage: /pb-compose [--run] [--fast] CashPilot: onboarding, billing", "warning");
        return;
      }
      const tokens = shellishSplit(text);
      const run = tokens.includes("--run");
      const fast = tokens.includes("--fast");
      const request = tokens.filter((token) => token !== "--run" && token !== "--fast").join(" ");
      const command = ["compose", "--text", request];
      if (run) command.push("--run");
      if (fast) command.push("--fast");
      const result = await runPatchbay(pi, ctx, command, { quiet: true });
      notifyCommandResult(ctx, result, "Patchbay compose completed.");
    },
  });

  pi.registerCommand("pb-mission-start", {
    description: "Create a Mission Control run from project-prefixed text, e.g. `Patchbay: build X; CashPilot: fix Y`.",
    handler: async (args, ctx) => {
      const text = commandText(args).trim();
      if (!text) {
        notify(ctx, "Usage: /pb-mission-start Patchbay: build X; CashPilot: fix Y", "warning");
        return;
      }
      const result = await runPatchbay(pi, ctx, ["mission", "start", "--text", text], { quiet: true });
      notifyCommandResult(ctx, result, "Patchbay mission created.");
    },
  });

  pi.registerCommand("pb-mission-list", {
    description: "List stored Patchbay Mission Control runs.",
    handler: async (_args, ctx) => {
      const result = await runPatchbay(pi, ctx, ["mission", "list"], { quiet: true });
      notifyCommandResult(ctx, result, "Patchbay mission list completed.");
    },
  });

  pi.registerCommand("pb-mission-status", {
    description: "Show Mission Control status. Optional argument: mission id.",
    handler: async (args, ctx) => {
      const mission = commandText(args).trim();
      const command = mission ? ["mission", "status", mission] : ["mission", "status"];
      const result = await runPatchbay(pi, ctx, command, { quiet: true });
      notifyCommandResult(ctx, result, "Patchbay mission status completed.");
    },
  });

  pi.registerCommand("pb-mission-show", {
    description: "Show a Mission Control run with generated agent prompts. Argument: mission id.",
    handler: async (args, ctx) => {
      const mission = commandText(args).trim();
      if (!mission) {
        notify(ctx, "Usage: /pb-mission-show <mission-id>", "warning");
        return;
      }
      const result = await runPatchbay(pi, ctx, ["mission", "show", mission], { quiet: true });
      notifyCommandResult(ctx, result, "Patchbay mission show completed.");
    },
  });

  pi.registerCommand("pb-mission-run", {
    description: "Run a Patchbay mission agent loop. Usage: <mission-id> [agent-id] [--all] [--fast] [--profile model].",
    handler: async (args, ctx) => {
      const tokens = shellishSplit(commandText(args));
      const mission = tokens.find((token) => !token.startsWith("--"));
      if (!mission) {
        notify(ctx, "Usage: /pb-mission-run <mission-id> [agent-id] [--all] [--fast]", "warning");
        return;
      }
      const rest = tokens.filter((token) => token !== mission);
      const command = ["mission", "run", mission];
      const agent = rest.find((token) => !token.startsWith("--") && token !== "fast");
      if (agent) command.push(agent);
      if (rest.includes("--all")) command.push("--all");
      if (rest.includes("--fast")) command.push("--fast");
      const profileIndex = rest.indexOf("--profile");
      if (profileIndex >= 0 && rest[profileIndex + 1]) command.push("--profile", rest[profileIndex + 1]);
      const result = await runPatchbay(pi, ctx, command, { quiet: true });
      notifyCommandResult(ctx, result, "Patchbay mission run completed.");
    },
  });

  pi.registerCommand("pb-mission-add-agent", {
    description: "Add a root or child agent. Usage: <mission-id> --project name --task text [--parent agent-id] [--fast].",
    handler: async (args, ctx) => {
      const tokens = shellishSplit(commandText(args));
      const mission = tokens.find((token) => !token.startsWith("--"));
      const project = valueAfter(tokens, "--project");
      const task = valueAfter(tokens, "--task");
      if (!mission || !project || !task) {
        notify(ctx, "Usage: /pb-mission-add-agent <mission-id> --project name --task text [--parent agent-id]", "warning");
        return;
      }
      const command = ["mission", "add-agent", mission, "--project", project, "--task", task];
      const parent = valueAfter(tokens, "--parent");
      if (parent) command.push("--parent", parent);
      if (tokens.includes("--fast")) command.push("--fast");
      const result = await runPatchbay(pi, ctx, command, { quiet: true });
      notifyCommandResult(ctx, result, "Patchbay child agent added.");
    },
  });

  pi.registerCommand("pb-mission-update-agent", {
    description: "Update a mission agent. Syntax: <mission-id> <agent-id> [--status complete] [--summary text] [--revision 1] [--diff 1:src/main.rs].",
    handler: async (args, ctx) => {
      const tokens = tokenizeCommandText(commandText(args));
      if (tokens.length < 2) {
        notify(ctx, "Usage: /pb-mission-update-agent <mission-id> <agent-id> [--status complete] [--summary text] [--revision 1] [--diff 1:path]", "warning");
        return;
      }
      const result = await runPatchbay(pi, ctx, ["mission", "update-agent", ...tokens], { quiet: true });
      notifyCommandResult(ctx, result, "Patchbay mission agent updated.");
    },
  });
}

type RunOptions = { quiet?: boolean };

async function runPatchbay(pi: ExtensionAPI, ctx: ExtensionContext, args: string[], options: RunOptions = {}) {
  const invocation = patchbayInvocation(ctx.cwd);
  try {
    const result = await pi.exec(invocation.command, [...invocation.prefixArgs, ...args], {
      cwd: ctx.cwd,
      signal: ctx.signal,
      timeout: 30_000,
    });

    if (result.code !== 0 && !options.quiet) {
      notify(ctx, `Patchbay command failed: ${result.stderr || result.stdout}`.trim(), "error");
    }

    return result;
  } catch (error) {
    if (!options.quiet) {
      notify(ctx, `Patchbay integration error: ${error instanceof Error ? error.message : String(error)}`, "error");
    }
    return { stdout: "", stderr: String(error), code: 1, killed: false };
  }
}

function patchbayInvocation(cwd: string): { command: string; prefixArgs: string[] } {
  if (process.env.PATCHBAY_BIN) {
    return { command: process.env.PATCHBAY_BIN, prefixArgs: [] };
  }

  const debugBinary = path.join(cwd, "target", "debug", process.platform === "win32" ? "pb.exe" : "pb");
  if (fs.existsSync(debugBinary)) {
    return { command: debugBinary, prefixArgs: [] };
  }

  return { command: "pb", prefixArgs: [] };
}

function pathsFromToolCall(event: ToolCallEvent): string[] {
  if (event.toolName === "write" || event.toolName === "edit" || event.toolName === "read") {
    const maybePath = event.input.path;
    return typeof maybePath === "string" ? [stripAtPathPrefix(maybePath)] : [];
  }

  return [];
}

function stripAtPathPrefix(filePath: string): string {
  return filePath.startsWith("@") ? filePath.slice(1) : filePath;
}

function summarizeMessages(messages: unknown[]): string {
  const lastAssistant = [...messages].reverse().find((message: any) => message.role === "assistant");
  if (!lastAssistant) return "";
  return messageText(lastAssistant).slice(0, 8_000);
}

function messageText(message: any): string {
  const content = message?.content;
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .map((part) => {
        if (typeof part === "string") return part;
        if (typeof part?.text === "string") return part.text;
        if (typeof part?.content === "string") return part.content;
        return "";
      })
      .filter(Boolean)
      .join("\n");
  }
  if (typeof message?.text === "string") return message.text;
  return "";
}

function commandText(args: unknown): string {
  if (args == null) return "";
  if (typeof args === "string") return args;
  if (Array.isArray(args)) return args.map(String).join(" ");
  if (typeof args === "object") {
    const record = args as Record<string, unknown>;
    for (const key of ["text", "input", "args", "argument", "value"]) {
      if (typeof record[key] === "string") return record[key] as string;
    }
  }
  return String(args);
}

function tokenizeCommandText(text: string): string[] {
  const tokens: string[] = [];
  let current = "";
  let quote: '"' | "'" | null = null;

  for (const char of text) {
    if (quote) {
      if (char === quote) {
        quote = null;
      } else {
        current += char;
      }
      continue;
    }

    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }
    if (/\s/.test(char)) {
      if (current) {
        tokens.push(current);
        current = "";
      }
      continue;
    }
    current += char;
  }

  if (current) tokens.push(current);
  return tokens;
}

function shellishSplit(text: string): string[] {
  return tokenizeCommandText(text);
}

function valueAfter(tokens: string[], flag: string): string | undefined {
  const index = tokens.indexOf(flag);
  return index >= 0 ? tokens[index + 1] : undefined;
}

function notifyCommandResult(ctx: ExtensionContext, result: ExecResult, fallback: string) {
  notify(ctx, result.stdout || result.stderr || fallback, result.code === 0 ? "info" : "error");
}

function notify(ctx: ExtensionContext, message: string, level: "info" | "warning" | "error" | "success") {
  if (!ctx.hasUI || !ctx.ui) return;
  ctx.ui.notify(truncate(message, 1_500), level);
}

function truncate(value: string, max: number): string {
  return value.length > max ? `${value.slice(0, max)}…` : value;
}
