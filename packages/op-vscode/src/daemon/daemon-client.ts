// Wraps `op-host-web-server --serve-web --managed` as a child process: spawns
// it, reads the single-line handshake JSON under a bounded timeout, redacts the
// token from every forwarded log line, and shuts it down via the stdin-EOF
// parent-death lease. Node-only (no vscode import) so it is fully unit-testable.

import { type ChildProcessWithoutNullStreams, spawn } from "node:child_process";

export interface DaemonHandshake {
  port: number;
  token: string;
  version: string;
}

export interface DaemonLogger {
  info(line: string): void;
  error(line: string): void;
}

export interface SpawnOptions {
  /** Full command line — tests inject [process.execPath, fixturePath]; prod
   *  injects [binaryPath]. Daemon args are appended after this prefix. */
  command: string[];
  filePath?: string; // --file
  allowOrigin: string; // webview origin for --allow-origin
  logger: DaemonLogger;
  handshakeTimeoutMs?: number; // default 10_000
  /** Version the extension expects (from its own manifest metadata); a
   *  mismatching handshake version logs a warning (never a failure — dev
   *  builds drift), and the warning must NOT include the token. */
  expectedVersion?: string;
}

const DEFAULT_HANDSHAKE_TIMEOUT_MS = 10_000;
const DISPOSE_SIGKILL_DELAY_MS = 3_000;
const MALFORMED_HANDSHAKE_PREVIEW_BYTES = 32;

/** Wait for a child to exit, resolving immediately if it already has. */
function awaitExit(child: ChildProcessWithoutNullStreams): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve();
  return new Promise((resolve) => child.once("exit", () => resolve()));
}

export class DaemonClient {
  readonly handshake: DaemonHandshake;

  private readonly child: ChildProcessWithoutNullStreams;
  private readonly exitCallbacks: Array<(code: number | null) => void> = [];
  private exited = false;
  private disposing?: Promise<void>;

  private constructor(
    child: ChildProcessWithoutNullStreams,
    handshake: DaemonHandshake,
    logger: DaemonLogger,
  ) {
    this.child = child;
    this.handshake = handshake;

    child.once("exit", (code) => {
      this.exited = true;
      for (const cb of this.exitCallbacks) cb(code);
    });

    // Forward every post-handshake stdout/stderr line, redacting the token
    // first — daemon diagnostics can echo URLs/headers carrying it.
    this.forwardLines(child.stdout, (line) => logger.info(this.redact(line)));
    this.forwardLines(child.stderr, (line) => logger.error(this.redact(line)));
  }

  static async spawn(opts: SpawnOptions): Promise<DaemonClient> {
    const [exe, ...prefix] = opts.command;
    if (exe === undefined) {
      throw new Error("DaemonClient.spawn: opts.command must not be empty");
    }
    const args = [
      ...prefix,
      "--serve-web",
      "--managed",
      "--port",
      "0",
      ...(opts.filePath ? ["--file", opts.filePath] : []),
      "--allow-origin",
      opts.allowOrigin,
    ];

    const child = spawn(exe, args, { stdio: ["pipe", "pipe", "pipe"] });
    const timeoutMs = opts.handshakeTimeoutMs ?? DEFAULT_HANDSHAKE_TIMEOUT_MS;

    // Every rejection path funnels through cleanup() so a rejected spawn (which
    // returns no disposable client) can never leak a managed daemon.
    const cleanup = async (): Promise<void> => {
      if (child.exitCode === null && child.signalCode === null) {
        child.kill("SIGKILL");
      }
      await awaitExit(child);
    };

    try {
      const line = await readFirstLine(child, timeoutMs);
      const handshake = parseHandshake(line);
      if (opts.expectedVersion && opts.expectedVersion !== handshake.version) {
        opts.logger.info(
          `daemon version ${handshake.version} differs from expected ${opts.expectedVersion}`,
        );
      }
      return new DaemonClient(child, handshake, opts.logger);
    } catch (err) {
      await cleanup();
      // Redact any token-shaped content and cap length before surfacing.
      throw new Error(`daemon handshake failed: ${previewError(err)}`);
    }
  }

  get baseUrl(): string {
    return `http://127.0.0.1:${this.handshake.port}`;
  }

  get alive(): boolean {
    return !this.exited;
  }

  onExit(cb: (code: number | null) => void): void {
    this.exitCallbacks.push(cb);
  }

  /** Closes stdin (parent-death lease), resolving when the process exits;
   *  SIGKILL fallback after 3s. Idempotent. */
  dispose(): Promise<void> {
    if (this.disposing) return this.disposing;
    this.disposing = (async () => {
      if (this.exited) return;
      const exited = awaitExit(this.child);
      this.child.stdin.end();
      const timer = setTimeout(() => {
        if (!this.exited) this.child.kill("SIGKILL");
      }, DISPOSE_SIGKILL_DELAY_MS);
      try {
        await exited;
      } finally {
        clearTimeout(timer);
      }
    })();
    return this.disposing;
  }

  private redact(line: string): string {
    return line.split(this.handshake.token).join("<redacted>");
  }

  private forwardLines(
    stream: NodeJS.ReadableStream,
    onLine: (line: string) => void,
  ): void {
    let buffer = "";
    stream.setEncoding("utf8");
    stream.on("data", (chunk: string) => {
      buffer += chunk;
      let idx: number;
      while ((idx = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, idx);
        buffer = buffer.slice(idx + 1);
        if (line.length > 0) onLine(line);
      }
    });
    stream.on("end", () => {
      if (buffer.length > 0) onLine(buffer);
    });
  }
}

/** Read the first newline-terminated line from stdout under a bounded timeout.
 *  Rejects on timeout, on the stream ending before a line arrives, or on a
 *  stream error. Detaches its own listeners on settle so the client's own
 *  forwarders take over cleanly. */
function readFirstLine(
  child: ChildProcessWithoutNullStreams,
  timeoutMs: number,
): Promise<string> {
  return new Promise<string>((resolve, reject) => {
    const stdout = child.stdout;
    let buffer = "";
    let settled = false;

    const cleanup = () => {
      clearTimeout(timer);
      stdout.removeListener("data", onData);
      stdout.removeListener("end", onEnd);
      stdout.removeListener("error", onError);
      child.removeListener("exit", onExit);
    };
    const done = (fn: () => void) => {
      if (settled) return;
      settled = true;
      cleanup();
      // Leave the leftover bytes for the client's forwarder by pausing here;
      // the forwarder re-attaches after spawn() resolves.
      stdout.pause();
      fn();
    };

    const timer = setTimeout(
      () => done(() => reject(new Error("handshake timeout"))),
      timeoutMs,
    );
    const onData = (chunk: string) => {
      buffer += chunk;
      const idx = buffer.indexOf("\n");
      if (idx >= 0) {
        const line = buffer.slice(0, idx);
        done(() => resolve(line));
      }
    };
    const onEnd = () =>
      done(() => reject(new Error("stdout closed before handshake")));
    const onError = (err: Error) => done(() => reject(err));
    const onExit = () =>
      done(() => reject(new Error("process exited before handshake")));

    stdout.setEncoding("utf8");
    stdout.on("data", onData);
    stdout.once("end", onEnd);
    stdout.once("error", onError);
    child.once("exit", onExit);
    stdout.resume();
  });
}

function parseHandshake(line: string): DaemonHandshake {
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch {
    throw new Error("handshake is not JSON");
  }
  if (typeof value !== "object" || value === null) {
    throw new Error("handshake is not an object");
  }
  const rec = value as Record<string, unknown>;
  const { ok, port, token, version } = rec;
  if (ok !== true) throw new Error('handshake missing "ok":true');
  if (typeof port !== "number" || !Number.isInteger(port) || port < 0 || port > 65535) {
    throw new Error("handshake port invalid");
  }
  if (typeof token !== "string" || token.length === 0) {
    throw new Error("handshake token invalid");
  }
  if (typeof version !== "string") {
    throw new Error("handshake version invalid");
  }
  return { port, token, version };
}

/** Redact token-shaped runs and cap length so a raw handshake line can never
 *  reach a log via an error message. We do not know the token on the failure
 *  path (parse failed), so cap by bytes — the preview cannot contain a full
 *  secret because a malformed handshake never produced a usable token. */
function previewError(err: unknown): string {
  const message = err instanceof Error ? err.message : String(err);
  if (message.length <= MALFORMED_HANDSHAKE_PREVIEW_BYTES) return message;
  return `${message.slice(0, MALFORMED_HANDSHAKE_PREVIEW_BYTES)}…`;
}
