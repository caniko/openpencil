import { test, expect } from "bun:test";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { DaemonClient, type DaemonLogger, type SpawnOptions } from "./daemon-client";

const here = dirname(fileURLToPath(import.meta.url));
const FIXTURE = join(here, "..", "..", "test", "fixtures", "fake-daemon.mjs");

interface CapturingLogger extends DaemonLogger {
  lines: string[];
}
function capturingLogger(): CapturingLogger {
  const lines: string[] = [];
  return {
    lines,
    info: (l) => lines.push(`info:${l}`),
    error: (l) => lines.push(`error:${l}`),
  };
}

function opts(extra: string[], override: Partial<SpawnOptions> = {}): SpawnOptions {
  return {
    command: [process.execPath, FIXTURE, ...extra],
    allowOrigin: "vscode-webview://test",
    logger: capturingLogger(),
    handshakeTimeoutMs: 2_000,
    ...override,
  };
}

async function expectSpawnRejects(o: SpawnOptions): Promise<void> {
  await expect(DaemonClient.spawn(o)).rejects.toThrow();
}

test("parses the handshake into port/token/version", async () => {
  const client = await DaemonClient.spawn(
    opts(["--fake-port", "45001", "--fake-token", "abc123", "--fake-version", "1.2.3"]),
  );
  expect(client.handshake.port).toBe(45001);
  expect(client.handshake.token).toBe("abc123");
  expect(client.handshake.version).toBe("1.2.3");
  expect(client.baseUrl).toBe("http://127.0.0.1:45001");
  expect(client.alive).toBe(true);
  await client.dispose();
});

test("forwards --serve-web/--managed/--file/--allow-origin verbatim, in order", async () => {
  const logger = capturingLogger();
  const client = await DaemonClient.spawn(
    opts(["--fake-port", "45002", "--echo-argv"], {
      logger,
      filePath: "/tmp/design.op",
      allowOrigin: "vscode-webview://xyz",
    }),
  );
  await new Promise((r) => setTimeout(r, 200));
  const argvLine = logger.lines.find((l) => l.includes("argv "));
  expect(argvLine).toBeDefined();
  // The fixture echoes everything after its own prefix flags; the daemon flags
  // the client appended must appear in the documented order.
  const daemonArgs = argvLine!.slice(argvLine!.indexOf("--serve-web"));
  expect(daemonArgs).toBe(
    "--serve-web --managed --port 0 --file /tmp/design.op --allow-origin vscode-webview://xyz",
  );
  await client.dispose();
});

test("omits --file when no filePath is given", async () => {
  const logger = capturingLogger();
  const client = await DaemonClient.spawn(
    opts(["--fake-port", "45003", "--echo-argv"], { logger, allowOrigin: "vscode-webview://nofile" }),
  );
  await new Promise((r) => setTimeout(r, 200));
  const argvLine = logger.lines.find((l) => l.includes("argv "))!;
  const daemonArgs = argvLine.slice(argvLine.indexOf("--serve-web"));
  expect(daemonArgs).toBe("--serve-web --managed --port 0 --allow-origin vscode-webview://nofile");
  await client.dispose();
});

// Every reject path funnels through spawn()'s cleanup(), which kills the child
// and AWAITS its exit before the rejection surfaces. So a settled rejection is
// itself proof the child was reaped — no separate leak assertion is possible
// (spawn returns no client on failure), and the awaited cleanup guarantees no
// orphan survives the rejection.
test("rejects on handshake timeout (child reaped in cleanup)", async () => {
  await expectSpawnRejects(opts(["--no-handshake"], { handshakeTimeoutMs: 300 }));
});

test("rejects on garbage handshake (child reaped in cleanup)", async () => {
  await expectSpawnRejects(opts(["--garbage-handshake"]));
});

test("rejects on early exit before handshake (child already gone)", async () => {
  await expectSpawnRejects(opts(["--early-exit"]));
});

// A daemon that half-writes then closes stdout while staying ALIVE does not
// deliver a parent-side EOF under this runtime (the live child keeps the pipe's
// write end referenced), so the handshake timeout is the backstop that catches
// it — and the timeout path runs the same cleanup(), so the child is still
// reaped. The client keeps defensive stdout end/error handlers regardless, so a
// runtime that DOES deliver EOF fast-fails instead of waiting for the timeout.
test("rejects a half-written-then-closed stdout via the timeout backstop", async () => {
  await expectSpawnRejects(opts(["--close-stdout"], { handshakeTimeoutMs: 400 }));
});

test("redacts the token from forwarded log lines", async () => {
  const logger = capturingLogger();
  const client = await DaemonClient.spawn(
    opts(["--fake-token", "SECRETTOKEN", "--echo-log"], { logger }),
  );
  // Give the post-handshake stderr line time to arrive.
  await new Promise((r) => setTimeout(r, 200));
  const all = logger.lines.join("\n");
  expect(all).not.toContain("SECRETTOKEN");
  expect(all).toContain("<redacted>");
  await client.dispose();
});

test("never logs the token in plaintext across any line", async () => {
  const logger = capturingLogger();
  const client = await DaemonClient.spawn(
    opts(["--fake-token", "PLAINTEXTLEAK", "--echo-log"], { logger }),
  );
  await new Promise((r) => setTimeout(r, 200));
  for (const line of logger.lines) expect(line).not.toContain("PLAINTEXTLEAK");
  await client.dispose();
});

test("dispose closes stdin so the daemon self-exits cleanly (code 0, not killed)", async () => {
  const client = await DaemonClient.spawn(opts(["--fake-port", "45010"]));
  let exitCode: number | null = -1;
  client.onExit((code) => {
    exitCode = code;
  });
  await client.dispose();
  expect(client.alive).toBe(false);
  expect(exitCode).toBe(0); // stdin-EOF lease → graceful exit, not SIGKILL
});

test("logs a version-skew warning without failing", async () => {
  const logger = capturingLogger();
  const client = await DaemonClient.spawn(
    opts(["--fake-version", "0.0.1"], { logger, expectedVersion: "9.9.9" }),
  );
  expect(client.handshake.version).toBe("0.0.1");
  expect(logger.lines.some((l) => l.includes("differs from expected"))).toBe(true);
  await client.dispose();
});
