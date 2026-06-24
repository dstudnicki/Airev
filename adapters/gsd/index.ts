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

const GLOBAL_REGISTRATION_MARKER = "__airevGsdAdapterRegistered";

export default function (pi: ExtensionAPI) {
  const globalState = globalThis as Record<string, unknown>;
  if (globalState[GLOBAL_REGISTRATION_MARKER]) {
    return;
  }
  globalState[GLOBAL_REGISTRATION_MARKER] = true;

  let captureActive = false;

  pi.on("before_agent_start", async (event, ctx) => {
    const result = await runAirev(
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
      await runAirev(pi, ctx, ["touch", filePath], { quiet: true });
    }
  });

  pi.on("agent_end", async (event, ctx) => {
    if (!captureActive) return;

    const summary = summarizeMessages(event.messages);
    await runAirev(pi, ctx, ["turn", "end", "--summary", summary], { quiet: true });
    captureActive = false;
  });

  pi.registerCommand("airev-init", {
    description: "Initialize Airev storage for the current project.",
    handler: async (_args, ctx) => {
      const result = await runAirev(pi, ctx, ["init"], { quiet: true });
      notify(ctx, result.stdout || result.stderr || "Airev init completed.", result.code === 0 ? "success" : "error");
    },
  });

  pi.registerCommand("airev-status", {
    description: "Show local Airev revision status.",
    handler: async (_args, ctx) => {
      const result = await runAirev(pi, ctx, ["status"], { quiet: true });
      notify(ctx, result.stdout || result.stderr || "Airev status completed.", result.code === 0 ? "info" : "error");
    },
  });

  pi.registerCommand("airev-revisions", {
    description: "List local Airev revisions.",
    handler: async (_args, ctx) => {
      const result = await runAirev(pi, ctx, ["revisions"], { quiet: true });
      notify(ctx, result.stdout || result.stderr || "Airev revisions completed.", result.code === 0 ? "info" : "error");
    },
  });

  pi.registerCommand("airev-mission-start", {
    description: "Create a Mission Control run from project-prefixed text, e.g. `Airev: build X; CashPilot: fix Y`.",
    handler: async (args, ctx) => {
      const text = commandText(args).trim();
      if (!text) {
        notify(ctx, "Usage: /airev-mission-start Airev: build X; CashPilot: fix Y", "warning");
        return;
      }
      const result = await runAirev(pi, ctx, ["mission", "start", "--text", text], { quiet: true });
      notifyCommandResult(ctx, result, "Airev mission created.");
    },
  });

  pi.registerCommand("airev-mission-list", {
    description: "List stored Airev Mission Control runs.",
    handler: async (_args, ctx) => {
      const result = await runAirev(pi, ctx, ["mission", "list"], { quiet: true });
      notifyCommandResult(ctx, result, "Airev mission list completed.");
    },
  });

  pi.registerCommand("airev-mission-status", {
    description: "Show Mission Control status. Optional argument: mission id.",
    handler: async (args, ctx) => {
      const mission = commandText(args).trim();
      const command = mission ? ["mission", "status", mission] : ["mission", "status"];
      const result = await runAirev(pi, ctx, command, { quiet: true });
      notifyCommandResult(ctx, result, "Airev mission status completed.");
    },
  });

  pi.registerCommand("airev-mission-show", {
    description: "Show a Mission Control run with generated agent prompts. Argument: mission id.",
    handler: async (args, ctx) => {
      const mission = commandText(args).trim();
      if (!mission) {
        notify(ctx, "Usage: /airev-mission-show <mission-id>", "warning");
        return;
      }
      const result = await runAirev(pi, ctx, ["mission", "show", mission], { quiet: true });
      notifyCommandResult(ctx, result, "Airev mission show completed.");
    },
  });

  pi.registerCommand("airev-mission-update-agent", {
    description: "Update a mission agent. Syntax: <mission-id> <agent-id> [--status complete] [--summary text] [--revision 1] [--diff 1:src/main.rs].",
    handler: async (args, ctx) => {
      const tokens = tokenizeCommandText(commandText(args));
      if (tokens.length < 2) {
        notify(ctx, "Usage: /airev-mission-update-agent <mission-id> <agent-id> [--status complete] [--summary text] [--revision 1] [--diff 1:path]", "warning");
        return;
      }
      const result = await runAirev(pi, ctx, ["mission", "update-agent", ...tokens], { quiet: true });
      notifyCommandResult(ctx, result, "Airev mission agent updated.");
    },
  });
}

type RunOptions = { quiet?: boolean };

async function runAirev(pi: ExtensionAPI, ctx: ExtensionContext, args: string[], options: RunOptions = {}) {
  const invocation = airevInvocation(ctx.cwd);
  try {
    const result = await pi.exec(invocation.command, [...invocation.prefixArgs, ...args], {
      cwd: ctx.cwd,
      signal: ctx.signal,
      timeout: 30_000,
    });

    if (result.code !== 0 && !options.quiet) {
      notify(ctx, `Airev command failed: ${result.stderr || result.stdout}`.trim(), "error");
    }

    return result;
  } catch (error) {
    if (!options.quiet) {
      notify(ctx, `Airev integration error: ${error instanceof Error ? error.message : String(error)}`, "error");
    }
    return { stdout: "", stderr: String(error), code: 1, killed: false };
  }
}

function airevInvocation(cwd: string): { command: string; prefixArgs: string[] } {
  if (process.env.AIREV_BIN) {
    return { command: process.env.AIREV_BIN, prefixArgs: [] };
  }

  const debugBinary = path.join(cwd, "target", "debug", process.platform === "win32" ? "airev.exe" : "airev");
  if (fs.existsSync(debugBinary)) {
    return { command: debugBinary, prefixArgs: [] };
  }

  return { command: "airev", prefixArgs: [] };
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
