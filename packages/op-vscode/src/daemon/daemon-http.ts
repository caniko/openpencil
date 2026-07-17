// Token-authenticated HTTP client for the managed daemon's control endpoints.
// Node-only (global fetch, no vscode import). Document save/open deliberately do
// NOT go through here — those flow through the postMessage bridge's
// snapshot/open-document so the daemon's SyncGate stays authoritative; this
// layer only reads version, probes readiness, reads the raw document, and
// passes MCP requests through for the McpProxy.

export class DaemonHttp {
  private readonly baseUrl: string;
  private readonly token: string;
  private readonly timeoutMs: number;

  constructor(baseUrl: string, token: string, timeoutMs = 8_000) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    this.token = token;
    this.timeoutMs = timeoutMs;
  }

  /** GET /api/mcp/version → {"version":N} (note: no "ok" field). N must be a
   *  non-negative safe integer (mirrors the Rust u64), else reject. */
  async version(): Promise<number> {
    const res = await this.fetch("/api/mcp/version");
    if (res.status !== 200) {
      throw new Error(`version: unexpected status ${res.status}`);
    }
    const body = (await res.json()) as unknown;
    if (typeof body !== "object" || body === null) {
      throw new Error("version: response is not an object");
    }
    const v = (body as Record<string, unknown>).version;
    if (typeof v !== "number" || !Number.isSafeInteger(v) || v < 0) {
      throw new Error(`version: invalid version value ${JSON.stringify(v)}`);
    }
    return v;
  }

  /** The editor shell is servable only when both the root page AND the wasm
   *  glue return 200. The bundle-less daemon serves a 404 help page at `/`
   *  that itself mentions "op_host_web.js", so a body substring probe would
   *  false-positive — we probe HTTP status only. */
  async ready(): Promise<boolean> {
    const [root, glue] = await Promise.all([
      this.fetch("/"),
      this.fetch("/pkg/op_host_web.js"),
    ]);
    // Drain bodies so the sockets can close promptly.
    await Promise.all([root.text().catch(() => ""), glue.text().catch(() => "")]);
    return root.status === 200 && glue.status === 200;
  }

  async getDocument(): Promise<string> {
    const res = await this.fetch("/api/mcp/document");
    if (res.status !== 200) {
      throw new Error(`getDocument: unexpected status ${res.status}`);
    }
    return await res.text();
  }

  /** POST /mcp passthrough for the McpProxy: forwards a raw JSON-RPC body and
   *  returns the status, headers, and body verbatim (plus the injected token). */
  async mcpRaw(
    body: string,
    extraHeaders?: Record<string, string>,
  ): Promise<{ status: number; headers: Record<string, string>; body: string }> {
    const res = await this.fetch("/mcp", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        ...extraHeaders,
      },
      body,
    });
    const headers: Record<string, string> = {};
    res.headers.forEach((value, key) => {
      headers[key] = value;
    });
    return { status: res.status, headers, body: await res.text() };
  }

  private fetch(path: string, init?: RequestInit): Promise<Response> {
    return fetch(`${this.baseUrl}${path}`, {
      ...init,
      headers: {
        "X-OpenPencil-Token": this.token,
        ...init?.headers,
      },
      signal: AbortSignal.timeout(this.timeoutMs),
    });
  }
}
