// The "Configure AI (MCP)" and "Remove MCP" command shells. Drives the tested,
// vscode-free mcp-config adapters (detectIde/adapterFor/upsert/remove) with the
// diff-confirm UX, workspace-folder resolution, and reload prompt.

import * as vscode from "vscode";
import * as fsPath from "node:path";
import {
  adapterFor,
  detectIde,
  McpConfigParseError,
  type McpAdapter,
} from "../mcp/mcp-config";

export function proxyUrl(port: number): string {
  return `http://127.0.0.1:${port}/mcp`;
}

function currentAdapter(root: string): McpAdapter {
  const kind = detectIde({
    appName: vscode.env.appName,
    hasDir: (rel) => existsSyncQuiet(vscode.Uri.file(fsPath.join(root, rel))),
  });
  return adapterFor(kind);
}

/** Resolve the workspace folder to write the config into: the active .op file's
 *  folder if any, else a single root, else a QuickPick. */
async function pickWorkspaceRoot(): Promise<string | undefined> {
  const folders = vscode.workspace.workspaceFolders ?? [];
  if (folders.length === 0) {
    void vscode.window.showErrorMessage("OpenPencil: open a workspace folder first.");
    return undefined;
  }
  const active = vscode.window.activeTextEditor?.document.uri;
  if (active) {
    const f = vscode.workspace.getWorkspaceFolder(active);
    if (f) return f.uri.fsPath;
  }
  if (folders.length === 1) return folders[0].uri.fsPath;
  const pick = await vscode.window.showQuickPick(
    folders.map((f) => ({ label: f.name, description: f.uri.fsPath })),
    { title: "Configure OpenPencil MCP in which folder?" },
  );
  return pick?.description;
}

export async function configureMcpCommand(port: number): Promise<void> {
  if (!vscode.workspace.isTrusted) {
    void vscode.window.showWarningMessage("OpenPencil: MCP config needs a trusted workspace.");
    return;
  }
  const root = await pickWorkspaceRoot();
  if (!root) return;
  const adapter = currentAdapter(root);
  const target = vscode.Uri.file(adapter.configPath(root));

  const existing = await readTextOrNull(target);
  let next: string;
  try {
    next = adapter.upsert(existing, proxyUrl(port));
  } catch (err) {
    if (err instanceof McpConfigParseError) {
      void vscode.window.showErrorMessage(
        `OpenPencil: ${target.fsPath} is not valid JSON — please fix it by hand.`,
      );
      return;
    }
    throw err;
  }
  if (next === existing) {
    void vscode.window.showInformationMessage("OpenPencil MCP is already configured.");
    return;
  }

  const choice = await vscode.window.showInformationMessage(
    `Configure OpenPencil MCP in ${adapter.kind} (${target.fsPath})?`,
    "Show diff",
    "Write",
  );
  if (choice === "Show diff") {
    await showDiff(target, existing ?? "", next);
    const confirm = await vscode.window.showInformationMessage("Write this MCP config?", "Write");
    if (confirm !== "Write") return;
  } else if (choice !== "Write") {
    return;
  }

  await writeConfig(target, next);
  if (adapter.needsReload) {
    void vscode.window.showInformationMessage(
      "OpenPencil MCP written. Reload the window for the AI agent to pick it up.",
      "Reload",
    ).then((r) => {
      if (r === "Reload") void vscode.commands.executeCommand("workbench.action.reloadWindow");
    });
  } else {
    void vscode.window.showInformationMessage("OpenPencil MCP configured.");
  }
}

export async function removeMcpCommand(): Promise<void> {
  const root = await pickWorkspaceRoot();
  if (!root) return;
  const adapter = currentAdapter(root);
  const target = vscode.Uri.file(adapter.configPath(root));
  const existing = await readTextOrNull(target);
  let next: string | null;
  try {
    next = adapter.remove(existing);
  } catch (err) {
    if (err instanceof McpConfigParseError) {
      void vscode.window.showErrorMessage(`OpenPencil: ${target.fsPath} is not valid JSON.`);
      return;
    }
    throw err;
  }
  if (next === null) {
    void vscode.window.showInformationMessage("OpenPencil MCP entry not present.");
    return;
  }
  const confirm = await vscode.window.showInformationMessage(
    `Remove the OpenPencil MCP entry from ${target.fsPath}?`,
    "Remove",
  );
  if (confirm !== "Remove") return;
  await writeConfig(target, next);
  void vscode.window.showInformationMessage("OpenPencil MCP entry removed.");
}

// ---- helpers ----

async function readTextOrNull(uri: vscode.Uri): Promise<string | null> {
  try {
    return new TextDecoder().decode(await vscode.workspace.fs.readFile(uri));
  } catch {
    return null;
  }
}

async function writeConfig(uri: vscode.Uri, text: string): Promise<void> {
  try {
    await vscode.workspace.fs.createDirectory(vscode.Uri.file(fsPath.dirname(uri.fsPath)));
    await vscode.workspace.fs.writeFile(uri, new TextEncoder().encode(text));
  } catch (err) {
    void vscode.window.showErrorMessage(`OpenPencil: failed to write ${uri.fsPath}: ${String(err)}`);
    throw err;
  }
}

async function showDiff(target: vscode.Uri, before: string, after: string): Promise<void> {
  const left = target.with({ scheme: "untitled", path: `${target.path}.current` });
  const right = target.with({ scheme: "untitled", path: `${target.path}.proposed` });
  await vscode.workspace.openTextDocument(left).then((d) => applyText(d, before));
  await vscode.workspace.openTextDocument(right).then((d) => applyText(d, after));
  await vscode.commands.executeCommand("vscode.diff", left, right, "OpenPencil MCP: current ↔ proposed");
}

async function applyText(doc: vscode.TextDocument, text: string): Promise<void> {
  const edit = new vscode.WorkspaceEdit();
  edit.insert(doc.uri, new vscode.Position(0, 0), text);
  await vscode.workspace.applyEdit(edit);
}

function existsSyncQuiet(uri: vscode.Uri): boolean {
  try {
    // fs.stat is async; for the detect probe we accept a best-effort require.
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const nodeFs = require("node:fs") as typeof import("node:fs");
    return nodeFs.existsSync(uri.fsPath);
  } catch {
    return false;
  }
}
