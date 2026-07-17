// "Install AI Skill" / "Remove AI Skill" command shells. The skill markdown is
// extracted at build time into dist/assets/openpencil-skill.md (see build.mjs);
// this writes it into the current IDE's rules path. vscode uses MCP directly, so
// it needs no rules file.

import * as vscode from "vscode";
import * as fsPath from "node:path";
import { adapterFor, detectIde, type IdeKind } from "../mcp/mcp-config";

/** Rules file path per IDE (relative to the workspace root). null = the IDE
 *  drives OpenPencil purely through MCP and needs no rules file. */
function rulesRelPath(kind: IdeKind): string | null {
  switch (kind) {
    case "cursor":
      return ".cursor/rules/openpencil.mdc";
    case "trae":
      return ".trae/rules/openpencil.md";
    case "windsurf":
      return ".windsurf/rules/openpencil.md";
    case "vscode":
      return null;
  }
}

function detectKind(root: string): IdeKind {
  return detectIde({
    appName: vscode.env.appName,
    hasDir: (rel) => existsQuiet(fsPath.join(root, rel)),
  }).valueOf() as IdeKind;
}

async function firstRoot(): Promise<string | undefined> {
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

export async function installSkillCommand(context: vscode.ExtensionContext): Promise<void> {
  const root = await firstRoot();
  if (!root) {
    void vscode.window.showErrorMessage("OpenPencil: open a workspace folder first.");
    return;
  }
  const kind = detectKind(root);
  // adapterFor is used only to assert the kind resolves to a known adapter.
  adapterFor(kind);
  const rel = rulesRelPath(kind);
  if (rel === null) {
    void vscode.window.showInformationMessage(
      "On VS Code, OpenPencil works through MCP — no rules file needed. Run 'Configure AI (MCP)'.",
    );
    return;
  }
  const skill = await readSkillAsset(context);
  if (skill === null) {
    void vscode.window.showErrorMessage("OpenPencil: bundled skill asset missing from the extension.");
    return;
  }
  const target = vscode.Uri.file(fsPath.join(root, rel));
  const confirm = await vscode.window.showInformationMessage(
    `Install the OpenPencil design skill for ${kind} at ${rel}?`,
    "Install",
  );
  if (confirm !== "Install") return;
  await vscode.workspace.fs.createDirectory(vscode.Uri.file(fsPath.dirname(target.fsPath)));
  await vscode.workspace.fs.writeFile(target, new TextEncoder().encode(skill));
  void vscode.window.showInformationMessage(`OpenPencil skill installed at ${rel}.`);
}

export async function removeSkillCommand(): Promise<void> {
  const root = await firstRoot();
  if (!root) return;
  const rel = rulesRelPath(detectKind(root));
  if (rel === null) {
    void vscode.window.showInformationMessage("Nothing to remove on VS Code.");
    return;
  }
  const target = vscode.Uri.file(fsPath.join(root, rel));
  const confirm = await vscode.window.showInformationMessage(
    `Remove the OpenPencil skill file at ${rel}?`,
    "Remove",
  );
  if (confirm !== "Remove") return;
  try {
    await vscode.workspace.fs.delete(target);
    void vscode.window.showInformationMessage("OpenPencil skill removed.");
  } catch {
    void vscode.window.showInformationMessage("OpenPencil skill file was not present.");
  }
}

async function readSkillAsset(context: vscode.ExtensionContext): Promise<string | null> {
  const uri = vscode.Uri.joinPath(context.extensionUri, "dist", "assets", "openpencil-skill.md");
  try {
    return new TextDecoder().decode(await vscode.workspace.fs.readFile(uri));
  } catch {
    return null;
  }
}

function existsQuiet(p: string): boolean {
  try {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    return (require("node:fs") as typeof import("node:fs")).existsSync(p);
  } catch {
    return false;
  }
}
