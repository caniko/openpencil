// Cross-language integration smoke: drives the REAL op-host-web-server binary
// through the extension's own DaemonClient → DaemonHttp → McpProxy stack. It
// does not load VS Code — it exercises the daemon contract end to end.
//
// Requires `cargo build -p op-host-web-server`. Binary missing → the test
// FAILS (never skips) so a broken build is visible.

import { test, expect, afterEach } from "bun:test";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { DaemonClient, type DaemonLogger } from "../../src/daemon/daemon-client";
import { DaemonHttp } from "../../src/daemon/daemon-http";
import { McpProxy } from "../../src/mcp/mcp-proxy";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..", "..", "..", ".."); // packages/op-vscode/test/integration → repo root
const BINARY = join(repoRoot, "target", "debug", "op-host-web-server");
const BUNDLE_DIR = join(repoRoot, "crates", "op-host-web", "pkg");

const silent: DaemonLogger = { info: () => {}, error: () => {} };
const cleanups: Array<() => Promise<void> | void> = [];

afterEach(async () => {
  for (const c of cleanups.splice(0)) await c();
});

test("real daemon: handshake → version → mcp-via-proxy → auth/origin defenses → clean exit", async () => {
  expect(
    existsSync(BINARY),
    `op-host-web-server not built — run: cargo build -p op-host-web-server`,
  ).toBe(true);

  // A minimal .op file to open.
  const fixture = join(
    repoRoot,
    "vendor",
    "jian",
    "crates",
    "jian-ops-schema",
    "tests",
    "corpus",
    "nested-frame.op",
  );
  const client = await DaemonClient.spawn({
    command: [BINARY],
    filePath: existsSync(fixture) ? fixture : undefined,
    allowOrigin: "vscode-webview://smoke",
    logger: silent,
  });
  cleanups.push(() => client.dispose());

  // Handshake produced a usable base URL + token.
  expect(client.baseUrl).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/);
  expect(client.handshake.token.length).toBeGreaterThan(0);

  const http = new DaemonHttp(client.baseUrl, client.handshake.token);
  const version = await http.version();
  expect(typeof version).toBe("number");
  expect(version).toBeGreaterThanOrEqual(0);

  // ready(): true only when the wasm bundle is present. Report honestly.
  const ready = await http.ready();
  if (existsSync(BUNDLE_DIR)) {
    expect(ready).toBe(true);
  } else {
    expect(ready).toBe(false);
    console.log("note: op-host-web/pkg bundle absent (needs EMSDK); ready()=false as expected");
  }

  // No-token direct hit on a privileged endpoint → 401.
  const unauth = await fetch(`${client.baseUrl}/api/mcp/version`);
  expect(unauth.status).toBe(401);

  // MCP initialize through the proxy → a JSON-RPC result.
  const proxy = new McpProxy(
    { active: { filePath: "x", client }, onActiveChanged: () => {} },
    silent,
  );
  const port = await proxy.listen(0);
  cleanups.push(() => proxy.dispose());
  const proxyUrl = `http://127.0.0.1:${port}/mcp`;

  const init = await fetch(proxyUrl, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}',
  });
  expect(init.status).toBe(200);
  const initBody = (await init.json()) as { result?: unknown };
  expect(initBody.result).toBeDefined();

  // A browser-style Origin header on the proxy → 403 (DNS-rebind defense).
  const withOrigin = await fetch(proxyUrl, {
    method: "POST",
    headers: { "content-type": "application/json", origin: "http://evil.example" },
    body: '{"jsonrpc":"2.0","id":2,"method":"initialize"}',
  });
  expect(withOrigin.status).toBe(403);

  // dispose() closes stdin → the daemon self-exits.
  await client.dispose();
  expect(client.alive).toBe(false);
});
