// Stable loopback MCP endpoint. IDE AI agents point at a single, port-stable
// URL (written once into their MCP config); every request is routed to the
// CURRENTLY active .op document's daemon — so a daemon restart on a new port is
// transparent to the client. No vscode import (the port-persistence lives in
// extension.ts); the pool is injected as a narrow interface.

import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { DaemonLogger } from "../daemon/daemon-client";

interface RoutableClient {
  readonly baseUrl: string;
  readonly handshake: { token: string };
}
interface ActivePool {
  readonly active: { filePath: string; client: RoutableClient } | undefined;
  onActiveChanged(cb: () => void): void;
}

const NO_ACTIVE_MESSAGE = "No active OpenPencil document. Open a .op file in the editor first.";

export class McpProxy {
  private readonly pool: ActivePool;
  private readonly logger: DaemonLogger;
  private server?: Server;
  private boundPort = 0;

  constructor(pool: ActivePool, logger: DaemonLogger) {
    this.pool = pool;
    this.logger = logger;
  }

  /** Bind to 127.0.0.1 on preferredPort (0 = OS-assigned); returns the actual
   *  port. Rejects if the port is taken so the caller can pick another. */
  listen(preferredPort: number): Promise<number> {
    return new Promise<number>((resolve, reject) => {
      const server = createServer((req, res) => void this.handle(req, res));
      server.on("error", reject);
      server.listen(preferredPort, "127.0.0.1", () => {
        server.removeListener("error", reject);
        const addr = server.address();
        if (typeof addr !== "object" || addr === null) {
          reject(new Error("proxy: no bound address"));
          return;
        }
        this.server = server;
        this.boundPort = addr.port;
        resolve(addr.port);
      });
    });
  }

  get port(): number {
    return this.boundPort;
  }

  dispose(): Promise<void> {
    const server = this.server;
    this.server = undefined;
    if (!server) return Promise.resolve();
    return new Promise<void>((resolve) => server.close(() => resolve()));
  }

  private async handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
    if (req.method !== "POST" || req.url !== "/mcp") {
      res.writeHead(404).end();
      return;
    }
    // Loopback cross-site defenses: MCP clients send no Origin; a request that
    // carries one is a browser (DNS-rebinding vector) → reject. The Host header
    // must name our own loopback authority.
    if (req.headers.origin !== undefined) {
      res.writeHead(403).end("origin not allowed");
      return;
    }
    if (!this.hostAllowed(req.headers.host)) {
      res.writeHead(403).end("host not allowed");
      return;
    }

    const body = await readBody(req);
    const active = this.pool.active;
    if (!active) {
      const id = extractJsonRpcId(body);
      const payload = JSON.stringify({
        jsonrpc: "2.0",
        id,
        error: { code: -32002, message: NO_ACTIVE_MESSAGE },
      });
      res.writeHead(200, { "content-type": "application/json" }).end(payload);
      return;
    }

    try {
      const forwardHeaders: Record<string, string> = {
        "content-type": "application/json",
        "X-OpenPencil-Token": active.client.handshake.token,
      };
      const sessionId = req.headers["mcp-session-id"];
      if (typeof sessionId === "string") forwardHeaders["mcp-session-id"] = sessionId;

      const upstream = await fetch(`${active.client.baseUrl}/mcp`, {
        method: "POST",
        headers: forwardHeaders,
        body,
      });
      const upstreamBody = await upstream.text();
      const outHeaders: Record<string, string> = { "content-type": "application/json" };
      const upstreamSession = upstream.headers.get("mcp-session-id");
      if (upstreamSession) outHeaders["mcp-session-id"] = upstreamSession;
      res.writeHead(upstream.status, outHeaders).end(upstreamBody);
    } catch (err) {
      this.logger.error(`mcp proxy forward failed: ${String(err)}`);
      const id = extractJsonRpcId(body);
      res
        .writeHead(502, { "content-type": "application/json" })
        .end(JSON.stringify({ jsonrpc: "2.0", id, error: { code: -32000, message: "daemon unreachable" } }));
    }
  }

  private hostAllowed(host: string | undefined): boolean {
    if (!host) return false;
    return host === `127.0.0.1:${this.boundPort}` || host === `localhost:${this.boundPort}`;
  }
}

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise<string>((resolve, reject) => {
    const chunks: Buffer[] = [];
    req.on("data", (c: Buffer) => chunks.push(c));
    req.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    req.on("error", reject);
  });
}

/** Best-effort JSON-RPC id for an error echo; null if the body isn't parseable
 *  or has no id (JSON-RPC allows a null id on errors). */
function extractJsonRpcId(body: string): number | string | null {
  try {
    const v = JSON.parse(body) as unknown;
    if (typeof v === "object" && v !== null) {
      const id = (v as Record<string, unknown>).id;
      if (typeof id === "number" || typeof id === "string") return id;
    }
  } catch {
    /* fall through */
  }
  return null;
}
