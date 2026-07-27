import { readFile } from "node:fs/promises";

import { errorMessage } from "../shared/errors.js";
import { isRequestId } from "../shared/ids.js";
import type { Envelope } from "../shared/types.js";
import { InputError, Supervisor } from "../supervisor/run.js";
import {
  emptyPromptEnvelope,
  failedEnvelope,
  makeEnvelope,
  writeEnvelope,
} from "./envelope.js";

const COMMANDS = new Set(["run", "resume", "status", "cleanup", "release", "smoke"]);

async function main(argv = process.argv.slice(2)): Promise<number> {
  const command = COMMANDS.has(argv[0] ?? "") ? argv.shift()! : "run";
  const requestEnvelopeCommand = command === "run" || command === "resume" || command === "release";
  let supervisor: Supervisor | null = null;
  try {
    if (command === "run") {
      const parsed = await parseRun(argv);
      if (!parsed.prompt.trim()) {
        writeEnvelope(emptyPromptEnvelope());
        return 0;
      }
      supervisor = await Supervisor.open();
      const envelope = await supervisor.run(parsed.prompt, parsed.files, parsed.timeoutSeconds);
      writeEnvelope(envelope);
      return envelope.hardFailure ? 1 : 0;
    }

    if (command === "resume") {
      const parsed = parseSessionCommand(argv, true);
      supervisor = await Supervisor.open();
      const envelope = await supervisor.resume(parsed.session, parsed.timeoutSeconds);
      writeEnvelope(envelope);
      return envelope.hardFailure ? 1 : 0;
    }

    if (command === "status") {
      if (argv.some((item) => item !== "--json")) throw new InputError("status accepts only --json");
      supervisor = await Supervisor.open();
      writeJson(await supervisor.status());
      return 0;
    }

    if (command === "cleanup") {
      const apply = parseCleanup(argv);
      supervisor = await Supervisor.open();
      const report = await supervisor.cleanup(apply);
      writeJson(report);
      return 0;
    }

    if (command === "release") {
      const parsed = parseSessionCommand(argv, false);
      supervisor = await Supervisor.open();
      const envelope = await supervisor.release(parsed.session);
      writeEnvelope(envelope);
      return envelope.hardFailure ? 1 : 0;
    }

    if (command === "smoke") {
      if (argv.length > 0) throw new InputError("smoke accepts no arguments");
      if (process.env.GWP_LIVE !== "1") throw new InputError("smoke requires GWP_LIVE=1");
      supervisor = await Supervisor.open();
      const envelope = await supervisor.run(
        "Reply with exactly GWP_SMOKE_OK",
        [],
        defaultTimeoutSeconds(),
      );
      const verified = envelope.status === "complete" && envelope.answer?.trim() === "GWP_SMOKE_OK";
      writeJson(verified ? {
        ok: true,
        sessionId: envelope.sessionId,
        answer: envelope.answer,
        answerSha256: envelope.answerSha256,
        artifacts: envelope.artifacts,
      } : {
        ok: false,
        sessionId: envelope.sessionId,
        status: envelope.status,
        errorKind: envelope.errorKind ?? "internal",
        message: envelope.message ?? "live smoke response did not match GWP_SMOKE_OK",
      });
      return envelope.hardFailure ? 1 : 0;
    }

    throw new InputError(`unknown command: ${command}`);
  } catch (error) {
    const input = error instanceof InputError;
    if (!requestEnvelopeCommand) {
      writeJson({ ok: false, error: errorMessage(error) });
      return input ? 2 : 0;
    }
    const envelope: Envelope = input
      ? makeEnvelope("needs_user_action", { usageError: true, message: error.message })
      : failedEnvelope(null, "internal", errorMessage(error));
    writeEnvelope(envelope);
    return input ? 2 : 0;
  } finally {
    supervisor?.close();
  }
}

function writeJson(value: unknown): void {
  process.stdout.write(`${JSON.stringify(value)}\n`);
}

async function parseRun(argv: string[]): Promise<{
  prompt: string;
  files: string[];
  timeoutSeconds: number;
}> {
  let promptFile: string | null = null;
  let timeoutSeconds = defaultTimeoutSeconds();
  const files: string[] = [];
  const promptParts: string[] = [];
  for (let index = 0; index < argv.length; index += 1) {
    const item = argv[index]!;
    if (item === "--file") {
      files.push(requireValue(argv, ++index, "--file"));
    } else if (item === "--prompt-file") {
      promptFile = requireValue(argv, ++index, "--prompt-file");
    } else if (item === "--timeout-seconds") {
      timeoutSeconds = parseTimeout(requireValue(argv, ++index, "--timeout-seconds"));
    } else if (item.startsWith("--")) {
      throw new InputError(`unknown run option: ${item}`);
    } else {
      promptParts.push(item);
    }
  }
  if (promptFile && promptParts.length > 0) {
    throw new InputError("--prompt-file cannot be combined with prompt arguments");
  }
  let prompt: string;
  if (promptFile) prompt = await readFile(promptFile, "utf8");
  else if (promptParts.length > 0) prompt = promptParts.join(" ");
  else if (!process.stdin.isTTY) prompt = await readStdin();
  else prompt = "";
  return { prompt, files, timeoutSeconds };
}

function parseSessionCommand(
  argv: string[],
  allowTimeout: boolean,
): { session: string; timeoutSeconds: number } {
  let session = "";
  let timeoutSeconds = defaultTimeoutSeconds();
  for (let index = 0; index < argv.length; index += 1) {
    const item = argv[index]!;
    if (item === "--session") session = requireValue(argv, ++index, "--session");
    else if (item === "--timeout-seconds" && allowTimeout) {
      timeoutSeconds = parseTimeout(requireValue(argv, ++index, "--timeout-seconds"));
    } else throw new InputError(`unexpected argument: ${item}`);
  }
  if (!session || !isRequestId(session)) throw new InputError("--session req_... is required");
  return { session, timeoutSeconds };
}

function parseCleanup(argv: string[]): boolean {
  if (argv.length === 0 || (argv.length === 1 && argv[0] === "--dry-run")) return false;
  if (argv.length === 1 && argv[0] === "--apply") return true;
  throw new InputError("cleanup accepts exactly one of --dry-run or --apply");
}

function requireValue(argv: string[], index: number, option: string): string {
  const value = argv[index];
  if (!value) throw new InputError(`${option} requires a value`);
  return value;
}

function defaultTimeoutSeconds(): number {
  return parseTimeout(process.env.GPTPRO_TIMEOUT ?? "10800");
}

function parseTimeout(value: string): number {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed < 0) {
    throw new InputError("timeout-seconds must be a non-negative number");
  }
  return parsed;
}

async function readStdin(): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of process.stdin) {
    chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(String(chunk)));
  }
  return Buffer.concat(chunks).toString("utf8");
}

main().then((exitCode) => {
  process.exitCode = exitCode;
}).catch((error) => {
  try {
    writeEnvelope(failedEnvelope(null, "internal", errorMessage(error)));
    process.exitCode = 0;
  } catch {
    process.exitCode = 70;
  }
});
