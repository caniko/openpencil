import { test, expect, afterEach } from "bun:test";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { McpProxy } from "./mcp-proxy";
import type { DaemonLogger } from "../daemon/daemon-client";

const silent: DaemonLogger = { info: () => {}, error: () => {} };

interface FakeDaemon {
  baseUrl: string;
  token: string;
  server: Server;
  lastToken?: string;
  lastBody?: string;
}

const daemons: FakeDaemon[] = [];
let proxy: McpProxy | undefined;

afterEach(async () => {
  if (proxy) {
    await proxy.dispose();
    proxy = undefined;
  }
  for (const d of daemons.splice(0)) {
    await new Promise<void>((r) => d.server.close(() => r()));
  }
});

async function fakeDaemon(token: string, respond: (body: string) => { status: number; body: string; sessionId?: string }): Promise<FakeDaemon> {
  const server = createServer((req: IncomingMessage, res: ServerResponse) => {
    d.lastToken = req.headers["x-openpencil-token"] as string | undefined;
    const chunks: Buffer[] = [];
    req.on("data", (c) => chunks.push(c as Buffer));
    req.on("end", () => {
      d.lastBody = Buffer.concat(chunks).toString();
      const out = respond(d.lastBody!);
      const headers: Record<string, string> = { "content-type": "application/json" };
      if (out.sessionId) headers["mcp-session-id"] = out.sessionId;
      res.writeHead(out.status, headers).end(out.body);
    });
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", () => r()));
  const addr = server.address();
  if (typeof addr !== "object" || addr === null) throw new Error("no addr");
  // One object shared by the request handler and the caller.
  const d: FakeDaemon = { token, server, baseUrl: `http://127.0.0.1:${addr.port}` };
  daemons.push(d);
  return d;
}

/** A mutable pool stub: set `.current` to switch the active daemon. */
class PoolStub {
  current?: FakeDaemon;
  private cbs: Array<() => void> = [];
  get active() {
    if (!this.current) return undefined;
    return {
      filePath: "/a.op",
      client: { baseUrl: this.current.baseUrl, handshake: { token: this.current.token } },
    };
  }
  onActiveChanged(cb: () => void) {
    this.cbs.push(cb);
  }
}

async function startProxy(pool: PoolStub): Promise<{ port: number; url: string }> {
  proxy = new McpProxy(pool, silent);
  const port = await proxy.listen(0);
  return { port, url: `http://127.0.0.1:${port}/mcp` };
}

const INIT = '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}';

test("no active document → -32002 error echoing the request id", async () => {
  const pool = new PoolStub();
  const { url } = await startProxy(pool);
  const res = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: INIT });
  expect(res.status).toBe(200);
  const json = (await res.json()) as { id: number; error: { code: number; message: string } };
  expect(json.id).toBe(1);
  expect(json.error.code).toBe(-32002);
  expect(json.error.message).toContain("No active OpenPencil document");
});

test("active daemon → forwards with the token injected, returns its response", async () => {
  const d = await fakeDaemon("TOKEN-A", () => ({ status: 200, body: '{"jsonrpc":"2.0","id":1,"result":{"ok":true}}' }));
  const pool = new PoolStub();
  pool.current = d;
  const { url } = await startProxy(pool);
  const res = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: INIT });
  expect(res.status).toBe(200);
  expect(d.lastToken).toBe("TOKEN-A");
  expect(d.lastBody).toContain('"method":"initialize"');
  const json = (await res.json()) as { result: { ok: boolean } };
  expect(json.result.ok).toBe(true);
});

test("switching active routes subsequent requests to the new daemon", async () => {
  const a = await fakeDaemon("TA", () => ({ status: 200, body: '{"jsonrpc":"2.0","id":1,"result":"A"}' }));
  const b = await fakeDaemon("TB", () => ({ status: 200, body: '{"jsonrpc":"2.0","id":1,"result":"B"}' }));
  const pool = new PoolStub();
  pool.current = a;
  const { url } = await startProxy(pool);
  let json = (await (await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: INIT })).json()) as { result: string };
  expect(json.result).toBe("A");
  pool.current = b;
  json = (await (await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: INIT })).json()) as { result: string };
  expect(json.result).toBe("B");
  expect(b.lastToken).toBe("TB");
});

test("a request carrying an Origin header is rejected 403 (DNS-rebinding defense)", async () => {
  const pool = new PoolStub();
  const { url } = await startProxy(pool);
  const res = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json", origin: "http://evil.example" },
    body: INIT,
  });
  expect(res.status).toBe(403);
});

test("a bad Host header is rejected 403", async () => {
  const pool = new PoolStub();
  const { port } = await startProxy(pool);
  // Send a raw request with a spoofed Host via a manual socket.
  const { connect } = await import("node:net");
  const status = await new Promise<number>((resolve) => {
    const sock = connect(port, "127.0.0.1", () => {
      const body = INIT;
      sock.write(
        `POST /mcp HTTP/1.1\r\nHost: evil.com\r\nContent-Type: application/json\r\nContent-Length: ${Buffer.byteLength(body)}\r\nConnection: close\r\n\r\n${body}`,
      );
    });
    let raw = "";
    sock.setEncoding("utf8");
    sock.on("data", (d) => (raw += d));
    sock.on("end", () => resolve(Number(raw.split(" ")[1])));
  });
  expect(status).toBe(403);
});

test("non-POST on /mcp is 405 (streamable-http GET probe); unknown paths are 404", async () => {
  const pool = new PoolStub();
  const { port } = await startProxy(pool);
  // Clients probe GET /mcp for the optional SSE stream — the spec's
  // "unsupported" answer is 405; a 404 makes them mark the connection broken.
  const get = await fetch(`http://127.0.0.1:${port}/mcp`, { method: "GET" });
  expect(get.status).toBe(405);
  expect(get.headers.get("allow")).toBe("POST");
  const del = await fetch(`http://127.0.0.1:${port}/mcp`, { method: "DELETE" });
  expect(del.status).toBe(405);
  const wrong = await fetch(`http://127.0.0.1:${port}/other`, { method: "POST", body: "{}" });
  expect(wrong.status).toBe(404);
});

test("upstream 500 is passed through verbatim", async () => {
  const d = await fakeDaemon("T", () => ({ status: 500, body: '{"jsonrpc":"2.0","id":1,"error":{"code":-32603}}' }));
  const pool = new PoolStub();
  pool.current = d;
  const { url } = await startProxy(pool);
  const res = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: INIT });
  expect(res.status).toBe(500);
});

test("mcp-session-id is forwarded upstream and returned downstream", async () => {
  const d = await fakeDaemon("T", () => ({ status: 200, body: "{}", sessionId: "sess-9" }));
  const pool = new PoolStub();
  pool.current = d;
  const { url } = await startProxy(pool);
  const res = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json", "mcp-session-id": "sess-9" },
    body: INIT,
  });
  expect(res.headers.get("mcp-session-id")).toBe("sess-9");
});
