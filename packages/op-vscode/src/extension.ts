// Activation assembly. Single registration topology (commands + one editor
// provider registered exactly once), late-bound through a mutable AppState so
// the limited→trusted transition never re-registers anything. Untrusted
// workspaces get a restricted placeholder; granting trust assembles the full
// stack and reopens placeholder tabs.

import * as vscode from "vscode";
import type { DaemonLogger } from "./daemon/daemon-client";
import { DaemonClient } from "./daemon/daemon-client";
import { DaemonPool } from "./daemon/daemon-pool";
import { McpProxy } from "./mcp/mcp-proxy";
import { SessionRegistry } from "./session/session-registry";
import { PenEditorProvider } from "./vscode/pen-editor-provider";
import { resolveDaemonBinary } from "./vscode/restart-source";
import { configureMcpCommand, removeMcpCommand } from "./vscode/configure-command";
import { installSkillCommand, removeSkillCommand } from "./vscode/skill-command";
import { generateCodeCommand, registerChatParticipant } from "./vscode/codegen-command";

const PROXY_PORT_KEY = "openpencil.proxyPort";

interface Assembled {
  pool: DaemonPool;
  proxy: McpProxy;
  registry: SessionRegistry;
  proxyPort: number;
}

interface AppState {
  trusted: boolean;
  assembled?: Assembled;
  /** In-flight assembly, so the provider can await it instead of falling back
   *  to the restricted placeholder when a trusted workspace opens a .op file
   *  before assembly finished. */
  assembling?: Promise<void>;
}

let output: vscode.OutputChannel | undefined;
const placeholderUris: vscode.Uri[] = [];

export function activate(context: vscode.ExtensionContext): void {
  output = vscode.window.createOutputChannel("OpenPencil");
  const logger: DaemonLogger = {
    info: (l) => output?.appendLine(l),
    error: (l) => output?.appendLine(`[error] ${l}`),
  };
  const state: AppState = { trusted: vscode.workspace.isTrusted };
  currentState = state;

  // Commands are registered ONCE; handlers dispatch through the mutable state.
  registerCommands(context, state);

  // The editor provider is registered ONCE; it branches on state.trusted.
  const provider = new UnifiedPenProvider(context, state, logger);
  context.subscriptions.push(
    vscode.window.registerCustomEditorProvider("openpencil.penEditor", provider, {
      supportsMultipleEditorsPerDocument: false,
      webviewOptions: { retainContextWhenHidden: true },
    }),
  );

  if (state.trusted) {
    state.assembling = assemble(context, state, logger);
    void state.assembling;
  } else {
    context.subscriptions.push(
      vscode.workspace.onDidGrantWorkspaceTrust(() => {
        void onTrustGranted(context, state, logger);
      }),
    );
  }
}

export async function deactivate(): Promise<void> {
  // Await the daemon shutdown chain so no managed daemon is orphaned.
  if (currentState?.assembled) {
    await currentState.assembled.proxy.dispose();
    await currentState.assembled.pool.disposeAll();
  }
  output?.dispose();
  output = undefined;
}

let currentState: AppState | undefined;

// ---- assembly ----

async function assemble(
  context: vscode.ExtensionContext,
  state: AppState,
  logger: DaemonLogger,
): Promise<void> {
  if (state.assembled) return; // idempotent guard
  const registry = new SessionRegistry();
  const spawn = makeSpawn(logger);
  const pool = new DaemonPool(spawn, logger);
  const proxy = new McpProxy(pool, logger);

  const preferred =
    context.workspaceState.get<number>(PROXY_PORT_KEY) ??
    vscode.workspace.getConfiguration("openpencil").get<number>("proxy.port", 0);
  let proxyPort: number;
  try {
    proxyPort = await proxy.listen(preferred);
  } catch {
    proxyPort = await proxy.listen(0); // preferred taken → OS-assign
    void vscode.window.showInformationMessage(
      "OpenPencil: the saved MCP port was busy; re-run 'Configure AI (MCP)' to update your config.",
    );
  }
  await context.workspaceState.update(PROXY_PORT_KEY, proxyPort);

  state.assembled = { pool, proxy, registry, proxyPort };
  context.subscriptions.push(
    new vscode.Disposable(() => {
      void proxy.dispose();
      void pool.disposeAll();
    }),
  );
  const participant = registerChatParticipant(registry);
  if (participant) context.subscriptions.push(participant);
}

async function onTrustGranted(
  context: vscode.ExtensionContext,
  state: AppState,
  logger: DaemonLogger,
): Promise<void> {
  if (state.assembled) return;
  state.trusted = true;
  await assemble(context, state, logger);
  // VS Code does not re-resolve already-open custom editors, so reopen the
  // placeholder tabs to pick up the now-trusted provider branch.
  for (const uri of placeholderUris.splice(0)) {
    await closeTabFor(uri);
    await vscode.commands.executeCommand("vscode.openWith", uri, "openpencil.penEditor");
  }
}

/** DaemonClient spawn factory: resolves the binary per file's workspace. */
function makeSpawn(logger: DaemonLogger) {
  return async (filePath: string, allowOrigin: string) => {
    const uri = vscode.Uri.file(filePath);
    const folder = vscode.workspace.getWorkspaceFolder(uri) ?? vscode.workspace.workspaceFolders?.[0];
    const configured = vscode.workspace.getConfiguration("openpencil").get<string>("dev.daemonPath");
    const binary = resolveDaemonBinary(configured, folder?.uri.fsPath);
    if (!binary) throw new Error("op-host-web-server binary not found; set openpencil.dev.daemonPath");
    return DaemonClient.spawn({
      command: [binary],
      filePath,
      allowOrigin,
      logger,
      expectedVersion: undefined,
    });
  };
}

// ---- unified provider (trusted branch delegates to PenEditorProvider) ----

class UnifiedPenProvider implements vscode.CustomEditorProvider<vscode.CustomDocument> {
  private readonly emitter = new vscode.EventEmitter<
    vscode.CustomDocumentContentChangeEvent<vscode.CustomDocument>
  >();
  readonly onDidChangeCustomDocument = this.emitter.event;
  private real?: PenEditorProvider;

  constructor(
    private readonly context: vscode.ExtensionContext,
    private readonly state: AppState,
    private readonly logger: DaemonLogger,
  ) {}

  /** Trusted → await any in-flight assembly, then the real provider. Untrusted
   *  → undefined (placeholder). Never returns undefined for a trusted workspace
   *  just because assembly hasn't finished yet — that would strand a valid file
   *  on the restricted placeholder with no trust event ever coming. */
  private async realProvider(): Promise<PenEditorProvider | undefined> {
    if (!this.state.trusted) return undefined;
    if (!this.state.assembled && this.state.assembling) await this.state.assembling;
    if (!this.state.assembled) return undefined; // assembly failed
    if (!this.real) {
      this.real = new PenEditorProvider(
        this.context,
        this.state.assembled.pool,
        this.state.assembled.registry,
        this.logger,
      );
      this.real.onDidChangeCustomDocument((e) =>
        this.emitter.fire({ document: e.document as vscode.CustomDocument }),
      );
    }
    return this.real;
  }

  async openCustomDocument(
    uri: vscode.Uri,
    openContext: vscode.CustomDocumentOpenContext,
    _token: vscode.CancellationToken,
  ): Promise<vscode.CustomDocument> {
    const real = await this.realProvider();
    if (real) return real.openCustomDocument(uri, openContext);
    // Untrusted only: a minimal placeholder document, tracked for the upgrade.
    placeholderUris.push(uri);
    return { uri, dispose: () => undefined };
  }

  async resolveCustomEditor(
    document: vscode.CustomDocument,
    panel: vscode.WebviewPanel,
    _token: vscode.CancellationToken,
  ): Promise<void> {
    const real = await this.realProvider();
    if (real) return real.resolveCustomEditor(document as never, panel);
    panel.webview.options = { enableScripts: true };
    panel.webview.html = restrictedPage();
    panel.webview.onDidReceiveMessage((m: unknown) => {
      if (m === "trust") void vscode.commands.executeCommand("workbench.action.manageTrust");
    });
  }

  // save/backup/revert only run after resolveCustomEditor set `this.real`
  // (a placeholder document never becomes dirty/savable), so use it directly.
  saveCustomDocument(document: vscode.CustomDocument, c: vscode.CancellationToken): Thenable<void> {
    return this.real?.saveCustomDocument(document as never, c) ?? Promise.resolve();
  }
  saveCustomDocumentAs(
    document: vscode.CustomDocument,
    dest: vscode.Uri,
    c: vscode.CancellationToken,
  ): Thenable<void> {
    return this.real?.saveCustomDocumentAs(document as never, dest, c) ?? Promise.resolve();
  }
  revertCustomDocument(document: vscode.CustomDocument, _c: vscode.CancellationToken): Thenable<void> {
    return this.real?.revertCustomDocument(document as never) ?? Promise.resolve();
  }
  backupCustomDocument(
    document: vscode.CustomDocument,
    ctx: vscode.CustomDocumentBackupContext,
    c: vscode.CancellationToken,
  ): Thenable<vscode.CustomDocumentBackup> {
    if (this.real) return this.real.backupCustomDocument(document as never, ctx, c);
    return Promise.resolve({ id: ctx.destination.toString(), delete: () => undefined });
  }
}

// ---- commands ----

function registerCommands(context: vscode.ExtensionContext, state: AppState): void {
  const guard = (fn: (a: Assembled) => Promise<void> | void) => async () => {
    if (!state.trusted || !state.assembled) {
      void vscode.window.showWarningMessage("OpenPencil: trust this workspace to use this command.");
      return;
    }
    await fn(state.assembled);
  };
  context.subscriptions.push(
    vscode.commands.registerCommand("openpencil.configureMcp", guard((a) => configureMcpCommand(a.proxyPort))),
    vscode.commands.registerCommand("openpencil.removeMcp", guard(() => removeMcpCommand())),
    vscode.commands.registerCommand("openpencil.installSkill", guard(() => installSkillCommand(context))),
    vscode.commands.registerCommand("openpencil.removeSkill", guard(() => removeSkillCommand())),
    vscode.commands.registerCommand("openpencil.generateCode", guard((a) => generateCodeCommand(a.registry))),
  );
}

// ---- pages ----

async function closeTabFor(uri: vscode.Uri): Promise<void> {
  for (const group of vscode.window.tabGroups.all) {
    for (const tab of group.tabs) {
      const input = tab.input as { uri?: vscode.Uri } | undefined;
      if (input?.uri?.toString() === uri.toString()) {
        await vscode.window.tabGroups.close(tab);
      }
    }
  }
}

function restrictedPage(): string {
  return `<!doctype html><html><body style="font-family:sans-serif;padding:2rem;color:var(--vscode-foreground)">
<h3>OpenPencil is in Restricted Mode</h3>
<p>This workspace isn't trusted, so OpenPencil won't start a local design daemon or write MCP config.</p>
<button onclick="acquireVsCodeApi().postMessage('trust')">Manage Workspace Trust…</button>
</body></html>`;
}
