import * as vscode from "vscode";

let outputChannel: vscode.OutputChannel | undefined;

export function activate(_context: vscode.ExtensionContext): void {
  outputChannel = vscode.window.createOutputChannel("OpenPencil");
}

export function deactivate(): void {
  outputChannel?.dispose();
  outputChannel = undefined;
}
