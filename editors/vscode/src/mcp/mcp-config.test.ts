import { test, expect } from "bun:test";
import { parse } from "jsonc-parser";
import {
  adapterFor,
  detectIde,
  McpConfigParseError,
  type IdeKind,
} from "./mcp-config";

const URL = "http://127.0.0.1:41000/mcp";

function noDir(): boolean {
  return false;
}

test("detectIde: appName keywords win", () => {
  expect(detectIde({ appName: "Cursor", hasDir: noDir })).toBe("cursor");
  expect(detectIde({ appName: "Trae AI", hasDir: noDir })).toBe("trae");
  expect(detectIde({ appName: "Windsurf", hasDir: noDir })).toBe("windsurf");
  expect(detectIde({ appName: "Visual Studio Code", hasDir: noDir })).toBe("vscode");
});

test("detectIde: ambiguous appName falls back to marker directory", () => {
  expect(detectIde({ appName: "Code - OSS", hasDir: (d) => d === ".cursor" })).toBe("cursor");
  expect(detectIde({ appName: "Code", hasDir: (d) => d === ".trae" })).toBe("trae");
  expect(detectIde({ appName: "Code", hasDir: (d) => d === ".windsurf" })).toBe("windsurf");
  expect(detectIde({ appName: "Code", hasDir: noDir })).toBe("vscode");
});

test("configPath differs per IDE", () => {
  expect(adapterFor("vscode").configPath("/w")).toBe("/w/.vscode/mcp.json");
  expect(adapterFor("cursor").configPath("/w")).toBe("/w/.cursor/mcp.json");
  expect(adapterFor("trae").configPath("/w")).toBe("/w/.trae/mcp.json");
  expect(adapterFor("windsurf").configPath("/w")).toBe("/w/.windsurf/mcp.json");
});

test("vscode upsert into an empty file produces a servers.openpencil http entry", () => {
  const out = adapterFor("vscode").upsert(null, URL);
  const parsed = parse(out) as { servers: { openpencil: { type: string; url: string } } };
  expect(parsed.servers.openpencil.type).toBe("http");
  expect(parsed.servers.openpencil.url).toBe(URL);
});

test("fork adapters use mcpServers with a bare url entry", () => {
  for (const kind of ["cursor", "trae", "windsurf"] as IdeKind[]) {
    const out = adapterFor(kind).upsert(null, URL);
    const parsed = parse(out) as { mcpServers: { openpencil: { url: string; type?: string } } };
    expect(parsed.mcpServers.openpencil.url).toBe(URL);
    expect(parsed.mcpServers.openpencil.type).toBeUndefined();
  }
});

test("upsert preserves comments and other servers in existing JSONC", () => {
  const existing = `{
  // my servers
  "servers": {
    "other": { "type": "http", "url": "http://other" }
  }
}`;
  const out = adapterFor("vscode").upsert(existing, URL);
  expect(out).toContain("// my servers"); // comment preserved
  const parsed = parse(out) as { servers: Record<string, { url: string }> };
  expect(parsed.servers.other.url).toBe("http://other"); // untouched
  expect(parsed.servers.openpencil.url).toBe(URL); // added
});

test("upsert overwrites an existing openpencil url, keeping siblings", () => {
  const existing = `{"mcpServers":{"openpencil":{"url":"http://old","extra":1},"keep":{"url":"http://keep"}}}`;
  const out = adapterFor("cursor").upsert(existing, URL);
  const parsed = parse(out) as { mcpServers: Record<string, { url: string; extra?: number }> };
  expect(parsed.mcpServers.openpencil.url).toBe(URL);
  expect(parsed.mcpServers.openpencil.extra).toBe(1); // sibling field preserved
  expect(parsed.mcpServers.keep.url).toBe("http://keep");
});

test("upsert throws on malformed existing JSONC (never clobbers)", () => {
  expect(() => adapterFor("vscode").upsert("{ not: json ", URL)).toThrow(McpConfigParseError);
});

test("remove deletes the openpencil entry, keeping the rest", () => {
  const existing = `{"mcpServers":{"openpencil":{"url":"http://x"},"keep":{"url":"http://keep"}}}`;
  const out = adapterFor("cursor").remove(existing);
  expect(out).not.toBeNull();
  const parsed = parse(out!) as { mcpServers: Record<string, unknown> };
  expect(parsed.mcpServers.openpencil).toBeUndefined();
  expect(parsed.mcpServers.keep).toBeDefined();
});

test("remove returns null when there is nothing to remove", () => {
  expect(adapterFor("cursor").remove(null)).toBeNull();
  expect(adapterFor("cursor").remove('{"mcpServers":{"other":{"url":"x"}}}')).toBeNull();
});

test("no adapter output ever contains the token header name", () => {
  for (const kind of ["vscode", "cursor", "trae", "windsurf"] as IdeKind[]) {
    const out = adapterFor(kind).upsert(null, URL);
    expect(out).not.toContain("X-OpenPencil-Token");
    expect(out.toLowerCase()).not.toContain("token");
  }
});
