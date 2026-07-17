// "Generate Code from Design" command + the @openpencil chat participant. Both
// use vscode.lm; when it is unavailable, generateCode guides the user to their
// IDE's own chat with a pre-filled prompt. The prompt assembly and output
// validation live in the tested, vscode-free codegen-prompt module.

import * as vscode from "vscode";
import * as fsPath from "node:path";
import type { SessionRegistry } from "../session/session-registry";
import {
  buildCodegenPrompt,
  parseCodegenOutput,
  type Framework,
} from "./codegen-prompt";

interface LmApi {
  selectChatModels(selector?: unknown): Thenable<unknown[]>;
}
function lm(): LmApi | undefined {
  return (vscode as unknown as { lm?: LmApi }).lm;
}

function framework(): Framework {
  const v = vscode.workspace.getConfiguration("openpencil").get<string>("codegen.framework", "react");
  return v === "vue" ? "vue" : "react";
}

async function activeDocJson(registry: SessionRegistry): Promise<string | undefined> {
  const session = registry.activeSession();
  if (!session) {
    void vscode.window.showWarningMessage("OpenPencil: open and focus a .op design first.");
    return undefined;
  }
  const bytes = await session.backup();
  return new TextDecoder().decode(bytes);
}

export async function generateCodeCommand(registry: SessionRegistry): Promise<void> {
  const docJson = await activeDocJson(registry);
  if (docJson === undefined) return;
  const prompt = buildCodegenPrompt(docJson, framework());

  const api = lm();
  if (!api) {
    // No language-model API: hand the prompt to the IDE's own chat.
    await vscode.env.clipboard.writeText(prompt);
    void vscode.window.showInformationMessage(
      "OpenPencil: this IDE has no language-model API. The codegen prompt is on your clipboard — paste it into your AI chat.",
    );
    return;
  }

  const models = (await api.selectChatModels()) as Array<{
    sendRequest(messages: unknown[], options?: unknown, token?: unknown): Thenable<{ text: AsyncIterable<string> }>;
  }>;
  if (models.length === 0) {
    void vscode.window.showWarningMessage("OpenPencil: no language model available.");
    return;
  }
  const model = models[0];

  const text = await vscode.window.withProgress(
    { location: vscode.ProgressLocation.Notification, title: "OpenPencil: generating code…", cancellable: true },
    async (_progress, token) => {
      const LanguageModelChatMessage = (
        vscode as unknown as { LanguageModelChatMessage: { User(text: string): unknown } }
      ).LanguageModelChatMessage;
      const res = await model.sendRequest([LanguageModelChatMessage.User(prompt)], {}, token);
      let acc = "";
      for await (const chunk of res.text) acc += chunk;
      return acc;
    },
  );

  const parsed = parseCodegenOutput(text);
  if (!parsed.ok) {
    const doc = await vscode.workspace.openTextDocument({ content: text, language: "markdown" });
    await vscode.window.showTextDocument(doc);
    void vscode.window.showWarningMessage(
      `OpenPencil: could not parse generated files (${parsed.errors[0]}). Raw output opened for you.`,
    );
    return;
  }

  const dir = await vscode.window.showOpenDialog({
    canSelectFolders: true,
    canSelectFiles: false,
    title: "Where should the generated files go?",
  });
  if (!dir || dir.length === 0) return;
  const outRoot = dir[0];

  for (const file of parsed.files) {
    const target = vscode.Uri.file(fsPath.join(outRoot.fsPath, file.path));
    if (await existsFs(target)) {
      const overwrite = await vscode.window.showWarningMessage(
        `${file.path} exists. Overwrite?`,
        "Overwrite",
        "Skip",
      );
      if (overwrite !== "Overwrite") continue;
    }
    await vscode.workspace.fs.createDirectory(vscode.Uri.file(fsPath.dirname(target.fsPath)));
    await vscode.workspace.fs.writeFile(target, new TextEncoder().encode(file.content));
  }
  void vscode.window.showInformationMessage(`OpenPencil: wrote ${parsed.files.length} file(s).`);
}

/** Register @openpencil as a read-only design advisor when the chat + lm APIs
 *  exist. Returns a disposable, or undefined when the APIs are absent. */
export function registerChatParticipant(
  registry: SessionRegistry,
): vscode.Disposable | undefined {
  const chat = (vscode as unknown as { chat?: { createChatParticipant?: unknown } }).chat;
  if (!lm() || !chat || typeof chat.createChatParticipant !== "function") return undefined;

  type ChatHandler = (
    request: { prompt: string },
    _ctx: unknown,
    response: { markdown(v: string): void },
    token: unknown,
  ) => Promise<void>;
  const handler: ChatHandler = async (request, _ctx, response, token) => {
    const session = registry.activeSession();
    if (!session) {
      response.markdown("Open and focus a `.op` design first, then ask again.");
      return;
    }
    const docJson = new TextDecoder().decode(await session.backup());
    const api = lm() as unknown as {
      selectChatModels(): Thenable<Array<{ sendRequest(m: unknown[], o?: unknown, t?: unknown): Thenable<{ text: AsyncIterable<string> }> }>>;
    };
    const models = await api.selectChatModels();
    if (models.length === 0) {
      response.markdown("No language model is available.");
      return;
    }
    const LanguageModelChatMessage = (
      vscode as unknown as { LanguageModelChatMessage: { User(text: string): unknown } }
    ).LanguageModelChatMessage;
    const prompt = `You are a read-only advisor for this OpenPencil design. Answer the question; do not attempt to modify the design.\n\nDesign:\n${docJson}\n\nQuestion: ${request.prompt}`;
    const res = await models[0].sendRequest([LanguageModelChatMessage.User(prompt)], {}, token);
    for await (const chunk of res.text) response.markdown(chunk);
  };

  const create = chat.createChatParticipant as (id: string, h: ChatHandler) => vscode.Disposable;
  return create("openpencil.assistant", handler);
}

async function existsFs(uri: vscode.Uri): Promise<boolean> {
  try {
    await vscode.workspace.fs.stat(uri);
    return true;
  } catch {
    return false;
  }
}
