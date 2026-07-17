import { test, expect, afterEach } from "bun:test";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { DaemonHttp } from "./daemon-http";

type Handler = (req: IncomingMessage, res: ServerResponse) => void;

let server: Server | undefined;

afterEach(async () => {
  if (server) {
    await new Promise<void>((r) => server!.close(() => r()));
    server = undefined;
  }
});

/** Start a stub daemon on a random loopback port; returns its baseUrl. */
async function stub(handler: Handler): Promise<string> {
  server = createServer(handler);
  await new Promise<void>((r) => server!.listen(0, "127.0.0.1", () => r()));
  const addr = server.address();
  if (typeof addr !== "object" || addr === null) throw new Error("no address");
  return `http://127.0.0.1:${addr.port}`;
}

const TOKEN = "tok-123";

test("attaches the token header to every request", async () => {
  let seenToken: string | undefined;
  const base = await stub((req, res) => {
    seenToken = req.headers["x-openpencil-token"] as string | undefined;
    res.writeHead(200, { "content-type": "application/json" });
    res.end('{"version":1}');
  });
  await new DaemonHttp(base, TOKEN).version();
  expect(seenToken).toBe(TOKEN);
});

test("version parses {\"version\":N}", async () => {
  const base = await stub((_req, res) => {
    res.writeHead(200, { "content-type": "application/json" });
    res.end('{"version":7}');
  });
  expect(await new DaemonHttp(base, TOKEN).version()).toBe(7);
});

test("version rejects a 401 (missing/wrong token)", async () => {
  const base = await stub((_req, res) => {
    res.writeHead(401, { "content-type": "application/json" });
    res.end('{"ok":false,"error":"unauthorized"}');
  });
  await expect(new DaemonHttp(base, TOKEN).version()).rejects.toThrow();
});

test("version rejects negative / fractional / string versions", async () => {
  for (const raw of ['{"version":-1}', '{"version":1.5}', '{"version":"7"}']) {
    const base = await stub((_req, res) => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(raw);
    });
    await expect(new DaemonHttp(base, TOKEN).version()).rejects.toThrow();
    await new Promise<void>((r) => server!.close(() => r()));
    server = undefined;
  }
});

test("ready is false when / returns 404 (bundle-less help page)", async () => {
  const base = await stub((req, res) => {
    if (req.url === "/") {
      res.writeHead(404);
      res.end("help page mentioning op_host_web.js");
    } else {
      res.writeHead(200);
      res.end("ok");
    }
  });
  expect(await new DaemonHttp(base, TOKEN).ready()).toBe(false);
});

test("ready is false when / is 200 but /pkg/op_host_web.js is 404", async () => {
  const base = await stub((req, res) => {
    if (req.url === "/pkg/op_host_web.js") {
      res.writeHead(404);
      res.end("missing");
    } else {
      res.writeHead(200);
      res.end("shell");
    }
  });
  expect(await new DaemonHttp(base, TOKEN).ready()).toBe(false);
});

test("ready is true when both / and /pkg/op_host_web.js are 200", async () => {
  const base = await stub((_req, res) => {
    res.writeHead(200);
    res.end("ok");
  });
  expect(await new DaemonHttp(base, TOKEN).ready()).toBe(true);
});

test("getDocument returns the raw body", async () => {
  const base = await stub((_req, res) => {
    res.writeHead(200, { "content-type": "application/json" });
    res.end('{"document":{"version":"1.0.0","children":[]}}');
  });
  expect(await new DaemonHttp(base, TOKEN).getDocument()).toContain('"children":[]');
});

test("mcpRaw passes the body through and returns status/headers/body", async () => {
  let receivedBody = "";
  const base = await stub((req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", (c) => chunks.push(c as Buffer));
    req.on("end", () => {
      receivedBody = Buffer.concat(chunks).toString();
      res.writeHead(200, { "content-type": "application/json", "mcp-session-id": "s1" });
      res.end('{"jsonrpc":"2.0","id":1,"result":{}}');
    });
  });
  const out = await new DaemonHttp(base, TOKEN).mcpRaw(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}',
    { "mcp-session-id": "s1" },
  );
  expect(receivedBody).toContain('"method":"initialize"');
  expect(out.status).toBe(200);
  expect(out.headers["mcp-session-id"]).toBe("s1");
  expect(out.body).toContain('"result"');
});

test("rejects when the daemon never responds (injected short timeout)", async () => {
  const base = await stub(() => {
    // never respond
  });
  await expect(new DaemonHttp(base, TOKEN, 200).version()).rejects.toThrow();
});
