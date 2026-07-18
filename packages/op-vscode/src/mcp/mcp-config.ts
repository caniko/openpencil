// Per-IDE MCP config adapters. Pure text-in/text-out using jsonc-parser's
// modify+applyEdits so comments and trailing commas in an existing config are
// preserved. The config only ever carries the proxy URL — never a token (the
// two-tier credential contract keeps the daemon token in-process only). No
// vscode import; the command shell (configure-command.ts) drives these.

import { applyEdits, modify, parse, type ParseError } from "jsonc-parser";

export type IdeKind = "vscode" | "cursor" | "trae" | "windsurf";

export interface IdeProbe {
  appName: string;
  /** Feature probe: does a marker directory (e.g. ".cursor") exist in the
   *  workspace or home? Corroborates an ambiguous appName. */
  hasDir(rel: string): boolean;
}

/** Malformed existing JSONC — the caller must not overwrite it blindly. */
export class McpConfigParseError extends Error {
  constructor(readonly errors: ParseError[]) {
    super("existing MCP config is not valid JSONC");
  }
}

/** appName is the primary signal; a marker directory corroborates when the
 *  name is ambiguous (the spec requires appName + feature probe). */
export function detectIde(probe: IdeProbe): IdeKind {
  const name = probe.appName.toLowerCase();
  if (name.includes("cursor")) return "cursor";
  if (name.includes("trae")) return "trae";
  if (name.includes("windsurf")) return "windsurf";
  if (probe.hasDir(".cursor")) return "cursor";
  if (probe.hasDir(".trae")) return "trae";
  if (probe.hasDir(".windsurf")) return "windsurf";
  return "vscode";
}

export interface McpAdapter {
  kind: IdeKind;
  configPath(workspaceRoot: string): string;
  /** Upsert the "openpencil" server entry, preserving everything else. Throws
   *  McpConfigParseError if existingText is present but not valid JSONC. */
  upsert(existingText: string | null, proxyUrl: string): string;
  /** Remove the "openpencil" server entry, preserving everything else. Returns
   *  null if there was nothing to change (no file / entry absent). */
  remove(existingText: string | null): string | null;
  needsReload: boolean;
}

const MODIFY_OPTS = {
  formattingOptions: { insertSpaces: true, tabSize: 2 },
} as const;

/** vscode uses `servers` with a typed http entry; the forks use `mcpServers`
 *  with a bare url entry. Fields are set individually so an existing
 *  openpencil entry's other keys are preserved (only url/type are overwritten). */
function makeAdapter(
  kind: IdeKind,
  dir: string,
  rootKey: "servers" | "mcpServers",
  fields: (url: string) => Array<[string, unknown]>,
  needsReload: boolean,
): McpAdapter {
  const path = [rootKey, "openpencil"];
  return {
    kind,
    needsReload,
    configPath: (root) => `${root}/${dir}/mcp.json`,
    upsert(existingText, proxyUrl) {
      let text = ensureObject(existingText);
      for (const [field, value] of fields(proxyUrl)) {
        // Apply sequentially: each modify is computed against the current text
        // (offsets shift as edits land), and per-field writes preserve siblings.
        const edits = modify(text, [...path, field], value, MODIFY_OPTS);
        text = applyEdits(text, edits);
      }
      return text;
    },
    remove(existingText) {
      if (existingText === null) return null;
      assertValidJsonc(existingText);
      const current = parse(existingText) as Record<string, unknown> | undefined;
      const root = current?.[rootKey] as Record<string, unknown> | undefined;
      if (!root || !("openpencil" in root)) return null; // nothing to remove
      const edits = modify(existingText, path, undefined, MODIFY_OPTS);
      return applyEdits(existingText, edits);
    },
  };
}

const ADAPTERS: Record<IdeKind, McpAdapter> = {
  vscode: makeAdapter("vscode", ".vscode", "servers", (url) => [["type", "http"], ["url", url]], true),
  // Forks get the same typed-http entry the editor's MCP settings card
  // shows ({"type":"http","url":…}): a bare `url` entry is treated as a
  // legacy SSE server by Cursor and never connects to the streamable-HTTP
  // proxy endpoint.
  cursor: makeAdapter("cursor", ".cursor", "mcpServers", (url) => [["type", "http"], ["url", url]], false),
  trae: makeAdapter("trae", ".trae", "mcpServers", (url) => [["type", "http"], ["url", url]], false),
  windsurf: makeAdapter("windsurf", ".windsurf", "mcpServers", (url) => [["type", "http"], ["url", url]], true),
};

export function adapterFor(kind: IdeKind): McpAdapter {
  return ADAPTERS[kind];
}

/** Null/empty → a fresh object to modify; otherwise validate the existing JSONC
 *  (never clobber a malformed user file). */
function ensureObject(existingText: string | null): string {
  if (existingText === null || existingText.trim() === "") return "{}";
  assertValidJsonc(existingText);
  return existingText;
}

function assertValidJsonc(text: string): void {
  const errors: ParseError[] = [];
  parse(text, errors, { allowTrailingComma: true });
  if (errors.length > 0) throw new McpConfigParseError(errors);
}
