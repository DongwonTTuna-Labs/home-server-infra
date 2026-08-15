import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import { errorMessage } from "../shared/errors.js";
import { isRequestId } from "../shared/ids.js";
import type { Envelope } from "../shared/types.js";
import {
  InputError,
  LoginInterruptedError,
  LoginTimeoutError,
  Supervisor,
} from "../supervisor/run.js";
import {
  emptyPromptEnvelope,
  failedEnvelope,
  makeEnvelope,
  writeEnvelope,
} from "./envelope.js";
const COMMANDS = new Set([
  "run",
  "resume",
  "status",
  "cleanup",
  "release",
  "smoke",
  "login",
  "keepalive",
  "reap",
]);
export async function main(argv = process.argv.slice(2)): Promise<number> {
  const command = COMMANDS.has(argv[0] ?? "") ? argv.shift()! : "run";
  const requestEnvelopeCommand = command === "run" || command === "resume" || command === "release";
  let supervisor: Supervisor | null = null;
  const closeBeforeOutput = (): void => {
    const current = supervisor;
    supervisor = null;
    try {
      current?.close();
    } catch (error) {
      // The command result is already determined. Keep stdout machine-readable;
      // process exit will release the DB handle even if explicit close failed.
      process.stderr.write(`supervisor close failed: ${errorMessage(error)}\n`);
    }
  };
  const emitEnvelope = (envelope: Envelope, exitCode: number): number => {
    closeBeforeOutput();
    writeEnvelope(envelope);
    return exitCode;
  };
  const emitJson = (value: unknown, exitCode: number): number => {
    closeBeforeOutput();
    writeJson(value);
    return exitCode;
  };
  try {
    if (command === "run") {
      const parsed = await parseRun(argv);
      if (!parsed.prompt.trim()) {
        return emitEnvelope(emptyPromptEnvelope(), 0);
      }
      supervisor = await Supervisor.open();
      const envelope = await supervisor.run(parsed.prompt, parsed.files, parsed.timeoutSeconds);
      return emitEnvelope(envelope, envelope.hardFailure ? 1 : 0);
    }
    if (command === "resume") {
      const parsed = parseSessionCommand(argv, true);
      supervisor = await Supervisor.open();
      const envelope = await supervisor.resume(parsed.session, parsed.timeoutSeconds);
      return emitEnvelope(envelope, envelope.hardFailure ? 1 : 0);
    }
    if (command === "status") {
      if (argv.some((item) => item !== "--json")) throw new InputError("status accepts only --json");
      supervisor = await Supervisor.open();
      return emitJson(await supervisor.status(), 0);
    }
    if (command === "cleanup") {
      const apply = parseCleanup(argv);
      supervisor = await Supervisor.open();
      const report = await supervisor.cleanup(apply);
      return emitJson(report, 0);
    }
    if (command === "release") {
      const parsed = parseSessionCommand(argv, false);
      supervisor = await Supervisor.open();
      const envelope = await supervisor.release(parsed.session);
      return emitEnvelope(envelope, envelope.hardFailure ? 1 : 0);
    }
    if (command === "login") {
      const slot = parseLogin(argv);
      supervisor = await Supervisor.open();
      const controller = new AbortController();
      const abort = () => controller.abort();
      process.once("SIGINT", abort);
      process.once("SIGTERM", abort);
      try {
        const result = await supervisor.login(slot, {
          signal: controller.signal,
          onUrl: (url) => {
            process.stderr.write(`noVNC: ${url}\n로그인 대기 중...\n`);
          },
          onProgress: (elapsedMs, state) => {
            const elapsedSeconds = Math.floor(elapsedMs / 1_000);
            process.stderr.write(
              `로그인 대기 중... 경과 ${elapsedSeconds}초 (readiness: ${state})\n`,
            );
          },
        });
        return emitJson({
          ok: true,
          slot: result.slotId,
          state: result.state,
          novncUrl: result.url,
        }, 0);
      } catch (error) {
        if (error instanceof InputError) throw error;
        if (controller.signal.aborted || error instanceof LoginInterruptedError) {
          return emitJson({ ok: false, slot, errorKind: "login_aborted" }, 130);
        }
        if (error instanceof LoginTimeoutError) {
          return emitJson({
            ok: false,
            slot,
            state: "needs_login",
            errorKind: "login_timeout",
            message: error.message,
          }, 0);
        }
        return emitJson({
          ok: false,
          slot,
          errorKind: "daemon_unreachable",
          message: errorMessage(error),
        }, 0);
      } finally {
        process.removeListener("SIGINT", abort);
        process.removeListener("SIGTERM", abort);
      }
    }
    if (command === "keepalive") {
      if (argv.length > 0) throw new InputError("keepalive accepts no arguments");
      supervisor = await Supervisor.open();
      return emitJson(await supervisor.keepalive(), 0);
    }
    if (command === "reap") {
      const timeoutSeconds = parseReap(argv);
      supervisor = await Supervisor.open();
      return emitJson(await supervisor.reap(timeoutSeconds), 0);
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
      return emitJson(verified ? {
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
      }, envelope.hardFailure ? 1 : 0);
    }
    throw new InputError(`unknown command: ${command}`);
  } catch (error) {
    const input = error instanceof InputError;
    if (!requestEnvelopeCommand) {
      return emitJson({ ok: false, error: errorMessage(error) }, input ? 2 : 0);
    }
    const envelope: Envelope = input
      ? makeEnvelope("needs_user_action", { usageError: true, message: error.message })
      : failedEnvelope(null, "internal", errorMessage(error));
    return emitEnvelope(envelope, input ? 2 : 0);
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
function parseReap(argv: string[]): number {
  // reap은 한 tick에 한 후보만 고른다. 이 값은 그 후보의 poll 예산이며, 완료되지 않으면
  // 공정 순번을 뒤로 보낸 뒤 다음 timer tick에서 다른 요청을 잇는다.
  let timeoutSeconds = 120;
  for (let index = 0; index < argv.length; index += 1) {
    const item = argv[index]!;
    if (item === "--timeout-seconds") {
      timeoutSeconds = parseTimeout(requireValue(argv, ++index, "--timeout-seconds"));
    } else throw new InputError(`unexpected argument: ${item}`);
  }
  return timeoutSeconds;
}
function parseCleanup(argv: string[]): boolean {
  if (argv.length === 0 || (argv.length === 1 && argv[0] === "--dry-run")) return false;
  if (argv.length === 1 && argv[0] === "--apply") return true;
  throw new InputError("cleanup accepts exactly one of --dry-run or --apply");
}
function parseLogin(argv: string[]): string {
  if (argv.length !== 2 || argv[0] !== "--slot" || !argv[1]) {
    throw new InputError("login requires exactly --slot <id>");
  }
  return argv[1];
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
if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
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
}
