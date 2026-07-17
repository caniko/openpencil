// CustomEditorProvider for .op files. Wires the tested cores (DaemonPool,
// PenSession, SessionRegistry, webview shell) to the VS Code custom-editor
// lifecycle. This is the vscode-coupled glue layer — verified by the Task 12
// integration smoke + manual matrix, not unit tests. The security-sensitive and
// protocol-critical logic all lives in the injected, unit-tested cores.

import * as vscode from "vscode";
import * as path from "node:path";
import type { DaemonClient, DaemonLogger } from "../daemon/daemon-client";
import { DaemonHttp } from "../daemon/daemon-http";
import type { DaemonPool } from "../daemon/daemon-pool";
import { PenSession, type SessionHost } from "../session/pen-session";
import type { SessionRegistry } from "../session/session-registry";
import type { BridgeOutboundToPage } from "../protocol/bridge";
import { encodeOutbound } from "../protocol/bridge";
import { buildBootHtml, buildWebviewHtml } from "./webview-shell";
import { pickRestartSource, resolveDaemonBinary, type LatestDurable } from "./restart-source";

const SHELL_READY_TIMEOUT_MS = 5_000;
const DAEMON_READY_TIMEOUT_MS = 10_000;
const DAEMON_READY_POLL_MS = 200;
const WATCHER_DEBOUNCE_MS = 300;

export class PenDocument implements vscode.CustomDocument {
  session?: PenSession;
  latestDurable: LatestDurable;
  constructor(
    readonly uri: vscode.Uri,
    readonly bootJson: string,
    initialSource: LatestDurable["source"],
  ) {
    this.latestDurable = { source: initialSource, json: bootJson };
  }
  dispose(): void {
    this.session?.dispose();
    this.session = undefined;
  }
}

export class PenEditorProvider implements vscode.CustomEditorProvider<PenDocument> {
  private readonly onDidChange = new vscode.EventEmitter<
    vscode.CustomDocumentContentChangeEvent<PenDocument>
  >();
  readonly onDidChangeCustomDocument = this.onDidChange.event;

  // One live mount per file. A single pool.onRestart handler (registered once)
  // looks up the mount and rebuilds it, disposing the previous mount's
  // listeners/watcher first so nothing accumulates across restarts.
  private readonly mounts = new Map<string, MountContext>();

  constructor(
    private readonly context: vscode.ExtensionContext,
    private readonly pool: DaemonPool,
    private readonly registry: SessionRegistry,
    private readonly logger: DaemonLogger,
  ) {
    this.pool.onRestart((filePath, client) => void this.handleRestart(filePath, client));
  }

  async openCustomDocument(
    uri: vscode.Uri,
    openContext: vscode.CustomDocumentOpenContext,
  ): Promise<PenDocument> {
    if (openContext.backupId) {
      // Hot-exit recovery: the backup bytes are authoritative and must survive
      // a subsequent daemon restart, so mark the durable source as "backup".
      const bytes = await vscode.workspace.fs.readFile(vscode.Uri.parse(openContext.backupId));
      return new PenDocument(uri, decode(bytes), "backup");
    }
    const bytes = await vscode.workspace.fs.readFile(uri);
    return new PenDocument(uri, decode(bytes), "disk");
  }

  async resolveCustomEditor(document: PenDocument, panel: vscode.WebviewPanel): Promise<void> {
    panel.webview.options = { enableScripts: true };
    const key = document.uri.fsPath;
    const watcherState = { suppressUntil: 0 };

    // Terminal teardown anchored to the PANEL (registered once, survives
    // restarts): whenever the panel closes, tear down the live mount if any and
    // ALWAYS release the daemon — the single guarantee that closing an editor
    // (even mid-restart, even after a failed re-mount) never orphans a daemon.
    panel.onDidDispose(() => {
      const live = this.mounts.get(key);
      if (live && live.panel === panel) {
        this.disposeMount(live);
        this.mounts.delete(key);
      }
      void this.pool.release(key);
    });

    try {
      const shellOrigin = await this.bootShellAndGetOrigin(panel);
      const client = await this.spawnAndAwaitReady(document.uri, shellOrigin);
      await this.mountEditor(document, panel, client, watcherState);
    } catch (err) {
      this.logger.error(`resolveCustomEditor failed: ${String(err)}`);
      panel.webview.html = errorPage(String(err));
      void this.pool.release(key); // failed initial mount → don't orphan
    }
  }

  /** Phase 1: install the message listener + timeout BEFORE assigning boot HTML
   *  (the shell script may run synchronously on html assignment). */
  private bootShellAndGetOrigin(panel: vscode.WebviewPanel): Promise<string> {
    return new Promise<string>((resolve, reject) => {
      let settled = false;
      const timer = setTimeout(() => {
        if (settled) return;
        settled = true;
        sub.dispose();
        reject(new Error("webview shell did not report ready"));
      }, SHELL_READY_TIMEOUT_MS);
      const sub = panel.webview.onDidReceiveMessage((raw: unknown) => {
        const origin = parseShellReadyOrigin(raw);
        if (origin === undefined || settled) return;
        settled = true;
        clearTimeout(timer);
        sub.dispose();
        resolve(origin);
      });
      panel.webview.html = buildBootHtml(makeNonce());
    });
  }

  private async spawnAndAwaitReady(uri: vscode.Uri, shellOrigin: string): Promise<DaemonClient> {
    const client = await this.pool.acquire(uri.fsPath, shellOrigin);
    const http = new DaemonHttp(client.baseUrl, client.handshake.token);
    const deadline = Date.now() + DAEMON_READY_TIMEOUT_MS;
    for (;;) {
      if (await http.ready().catch(() => false)) return client;
      if (Date.now() > deadline) throw new Error("daemon did not become ready");
      await delay(DAEMON_READY_POLL_MS);
    }
  }

  /** Phase 2: rebuild the full shell around the daemon iframe, wire the session
   *  and all lifecycle plumbing, and start the init loop. Every listener/watcher
   *  is collected into the mount's disposable set so a restart (or panel close)
   *  tears them all down with no accumulation. */
  private async mountEditor(
    document: PenDocument,
    panel: vscode.WebviewPanel,
    client: DaemonClient,
    watcherState: { suppressUntil: number },
  ): Promise<void> {
    const key = document.uri.fsPath;
    const nonce = makeNonce();
    const external = await vscode.env.asExternalUri(vscode.Uri.parse(`${client.baseUrl}/`));
    panel.webview.html = buildWebviewHtml({ iframeSrc: external.toString(), nonce });

    const host = this.makeHost(document, panel, watcherState);
    const session = new PenSession(host, client.handshake.token, document.bootJson);
    document.session = session;
    this.registry.register(key, session);

    const disposables: vscode.Disposable[] = [];
    disposables.push(
      panel.webview.onDidReceiveMessage((raw: unknown) => {
        if (isShellControl(raw)) return;
        session.onPageMessage(raw);
      }),
    );

    if (panel.active) this.setActive(key);
    disposables.push(
      panel.onDidChangeViewState((e) => {
        if (e.webviewPanel.active) this.setActive(key);
        else if (this.pool.active?.filePath === key) this.setActive(undefined);
      }),
    );

    // File watcher (GlobPattern required — Uri is not accepted).
    const watcher = vscode.workspace.createFileSystemWatcher(
      new vscode.RelativePattern(path.dirname(key), path.basename(key)),
    );
    const onExternal = async () => {
      if (Date.now() < watcherState.suppressUntil) return; // our own write
      try {
        const bytes = await vscode.workspace.fs.readFile(document.uri);
        await session.externalFileChanged(decode(bytes), session.isRustDirty);
      } catch (err) {
        this.logger.error(`external change handling failed: ${String(err)}`);
      }
    };
    disposables.push(watcher, watcher.onDidChange(onExternal), watcher.onDidCreate(onExternal));

    // Terminal teardown (daemon release + mount disposal on panel close) is
    // anchored at the PANEL scope in resolveCustomEditor — registered once so it
    // survives restarts. Here we only record the live mount.
    const ctx: MountContext = { document, panel, watcherState, session, disposables };
    this.mounts.set(key, ctx);

    session.start();
  }

  private disposeMount(ctx: MountContext): void {
    for (const d of ctx.disposables) {
      try {
        d.dispose();
      } catch {
        /* best-effort */
      }
    }
    ctx.disposables.length = 0;
    this.registry.unregister(ctx.document.uri.fsPath);
    ctx.session.dispose();
  }

  /** Single pool.onRestart handler: rebuild the live mount for this file with
   *  the correct recovery bytes, disposing the old mount's plumbing first. */
  private async handleRestart(filePath: string, client: DaemonClient | undefined): Promise<void> {
    const ctx = this.mounts.get(filePath);
    if (!ctx) return; // no live editor for this file
    const { document, panel } = ctx;
    const wasDirty = ctx.session.isRustDirty;
    this.disposeMount(ctx);
    this.mounts.delete(filePath);

    // Release the daemon on EVERY non-success path (repeated crash, or a failed
    // re-mount) so a broken restart can't leave a live daemon with no owning
    // mount. On success the fresh mount owns it (released at panel close). The
    // panel-scope onDidDispose in resolveCustomEditor is the final backstop;
    // pool.release is idempotent so a later close is a harmless no-op.
    let remounted = false;
    try {
      if (!client) {
        panel.webview.html = errorPage("The design daemon crashed repeatedly.");
        return;
      }
      const source = pickRestartSource(document.latestDurable, wasDirty);
      const bootJson =
        source.from === "durable"
          ? source.json
          : decode(await vscode.workspace.fs.readFile(document.uri));
      (document as { bootJson: string }).bootJson = bootJson;
      const shellOrigin = await this.bootShellAndGetOrigin(panel);
      const fresh = await this.spawnAndAwaitReady(document.uri, shellOrigin);
      await this.mountEditor(document, panel, fresh, { suppressUntil: 0 });
      remounted = true;
    } catch (err) {
      panel.webview.html = errorPage(`restart failed: ${String(err)}`);
    } finally {
      if (!remounted) void this.pool.release(filePath);
    }
  }

  private makeHost(
    document: PenDocument,
    panel: vscode.WebviewPanel,
    watcherState: { suppressUntil: number },
  ): SessionHost {
    const post = (msg: BridgeOutboundToPage) => {
      void panel.webview.postMessage(encodeOutbound(msg));
    };
    return {
      postToPage: post,
      contentChanged: () => this.onDidChange.fire({ document }),
      writeFile: async (bytes) => {
        watcherState.suppressUntil = Date.now() + WATCHER_DEBOUNCE_MS;
        await atomicWrite(document.uri, bytes);
        document.latestDurable = { source: "save", json: decode(bytes) };
      },
      writeBackup: async (name, bytes) => {
        const uri = this.backupUri(name);
        await vscode.workspace.fs.writeFile(uri, bytes);
      },
      writeBackupFallback: async (bytes) => {
        const target = await vscode.window.showSaveDialog({
          title: "Save your OpenPencil version",
          saveLabel: "Save copy",
        });
        if (!target) throw new Error("backup save cancelled — bytes not persisted");
        await vscode.workspace.fs.writeFile(target, bytes);
        return target.fsPath;
      },
      showConflictDialog: async (serverVersion) => {
        const pick = await vscode.window.showWarningMessage(
          `This design changed elsewhere (v${serverVersion}) while you were editing. Keep which version?`,
          { modal: true },
          "Keep my version",
          "Take the other version",
        );
        if (pick === "Keep my version") return "use-local";
        if (pick === "Take the other version") return "accept-remote";
        return undefined;
      },
      showExternalChangeDialog: async () => {
        const pick = await vscode.window.showWarningMessage(
          "The file changed on disk while you had unsaved edits.",
          { modal: true },
          "Reload from disk (discard mine)",
          "Keep my version",
          "Save disk copy elsewhere",
        );
        if (pick === "Reload from disk (discard mine)") return "reload";
        if (pick === "Save disk copy elsewhere") return "save-disk-copy";
        if (pick === "Keep my version") return "keep-local";
        return undefined;
      },
      schedule: (fn, ms) => {
        const t = setTimeout(fn, ms);
        return () => clearTimeout(t);
      },
      warn: (message) => {
        this.logger.error(message);
        void vscode.window.showWarningMessage(`OpenPencil: ${message}`);
      },
    };
  }

  // ---- CustomEditorProvider save/backup/revert ----

  saveCustomDocument(document: PenDocument, cancellation: vscode.CancellationToken): Thenable<void> {
    return this.requireSession(document).save(() => cancellation.isCancellationRequested);
  }

  async saveCustomDocumentAs(
    document: PenDocument,
    destination: vscode.Uri,
    cancellation: vscode.CancellationToken,
  ): Promise<void> {
    const bytes = await this.requireSession(document).backup(() => cancellation.isCancellationRequested);
    await atomicWrite(destination, bytes);
  }

  async revertCustomDocument(document: PenDocument): Promise<void> {
    const bytes = await vscode.workspace.fs.readFile(document.uri);
    const json = decode(bytes);
    await this.requireSession(document).revert(json);
    document.latestDurable = { source: "revert", json };
  }

  async backupCustomDocument(
    document: PenDocument,
    context: vscode.CustomDocumentBackupContext,
    cancellation: vscode.CancellationToken,
  ): Promise<vscode.CustomDocumentBackup> {
    const bytes = await this.requireSession(document).backup(() => cancellation.isCancellationRequested);
    await vscode.workspace.fs.writeFile(context.destination, bytes);
    document.latestDurable = { source: "backup", json: decode(bytes) };
    return {
      id: context.destination.toString(),
      delete: () => void vscode.workspace.fs.delete(context.destination).then(undefined, () => {}),
    };
  }

  // ---- helpers ----

  private requireSession(document: PenDocument): PenSession {
    if (!document.session) throw new Error("editor not ready");
    return document.session;
  }
  private setActive(filePath: string | undefined): void {
    this.pool.setActive(filePath);
    this.registry.setActive(filePath);
  }
  private backupUri(name: string): vscode.Uri {
    const dir = this.context.storageUri ?? this.context.globalStorageUri;
    return vscode.Uri.joinPath(dir, name);
  }
}

interface MountContext {
  document: PenDocument;
  panel: vscode.WebviewPanel;
  watcherState: { suppressUntil: number };
  session: PenSession;
  disposables: vscode.Disposable[];
}

// ---- module helpers ----

const decoder = new TextDecoder();
function decode(bytes: Uint8Array): string {
  return decoder.decode(bytes);
}

/** Atomic write: temp file + rename, so a crash mid-write can't truncate. */
async function atomicWrite(uri: vscode.Uri, bytes: Uint8Array): Promise<void> {
  const tmp = uri.with({ path: `${uri.path}.op-tmp` });
  await vscode.workspace.fs.writeFile(tmp, bytes);
  await vscode.workspace.fs.rename(tmp, uri, { overwrite: true });
}

function makeNonce(): string {
  const bytes = new Uint8Array(16);
  globalThis.crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

function delay(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

/** Parse an op-shell/ready control message → its reported origin, else undefined. */
function parseShellReadyOrigin(raw: unknown): string | undefined {
  if (typeof raw !== "string") return undefined;
  try {
    const v = JSON.parse(raw) as { type?: unknown; origin?: unknown };
    if (v.type === "op-shell/ready" && typeof v.origin === "string") return v.origin;
  } catch {
    /* not a control message */
  }
  return undefined;
}

function isShellControl(raw: unknown): boolean {
  return typeof raw === "string" && raw.includes("op-shell/");
}

function errorPage(detail: string): string {
  const safe = detail.replace(/[<&]/g, (c) => (c === "<" ? "&lt;" : "&amp;"));
  return `<!doctype html><html><body style="font-family:sans-serif;padding:2rem;color:var(--vscode-foreground)">
<h3>OpenPencil could not open this design</h3><p>${safe}</p>
<p>Check that <code>op-host-web-server</code> is built (<code>cargo build -p op-host-web-server</code>) or set <code>openpencil.dev.daemonPath</code>.</p>
</body></html>`;
}

export { resolveDaemonBinary };
