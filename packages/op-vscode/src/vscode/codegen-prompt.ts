// Pure helpers for the single-shot codegen command: build the model prompt and
// parse/validate the model's structured response. No vscode import — the
// command shell (codegen-command.ts) handles model selection, streaming, and
// the file writes. Keeping these pure makes the validation (the security-
// sensitive part) unit-testable.

export type Framework = "react" | "vue";

export interface CodegenFile {
  path: string;
  content: string;
}
export type CodegenParse =
  | { ok: true; files: CodegenFile[] }
  | { ok: false; errors: string[] };

const MAX_FILE_BYTES = 1024 * 1024; // 1 MiB per file
const MAX_TOTAL_BYTES = 10 * 1024 * 1024; // 10 MiB total

/** Ask the model for a strict JSON envelope so the output is machine-parseable
 *  rather than free-form prose. */
export function buildCodegenPrompt(docJson: string, framework: Framework): string {
  return [
    `You are generating ${framework} component code from an OpenPencil design document.`,
    "Respond with ONLY a JSON object of this exact shape, no prose, no markdown:",
    '{"files":[{"path":"relative/Component.tsx","content":"<file contents>"}]}',
    "Rules: paths are RELATIVE (no leading slash, no ..), unique, and reasonably small.",
    "",
    "Design document:",
    docJson,
  ].join("\n");
}

/** Parse + validate the model output. Tolerates a single ```json fenced block.
 *  Returns an error list (and writes NOTHING) on any violation. */
export function parseCodegenOutput(text: string): CodegenParse {
  const jsonText = stripFence(text).trim();
  let value: unknown;
  try {
    value = JSON.parse(jsonText);
  } catch {
    return { ok: false, errors: ["output is not valid JSON"] };
  }
  if (typeof value !== "object" || value === null || !Array.isArray((value as { files?: unknown }).files)) {
    return { ok: false, errors: ['output must be {"files":[...]}'] };
  }
  const rawFiles = (value as { files: unknown[] }).files;
  const errors: string[] = [];
  const files: CodegenFile[] = [];
  const seen = new Set<string>();
  let total = 0;

  rawFiles.forEach((raw, i) => {
    if (typeof raw !== "object" || raw === null) {
      errors.push(`file[${i}] is not an object`);
      return;
    }
    const { path, content } = raw as Record<string, unknown>;
    if (typeof path !== "string" || path.length === 0) {
      errors.push(`file[${i}] has no path`);
      return;
    }
    if (typeof content !== "string") {
      errors.push(`file[${i}] (${path}) has no string content`);
      return;
    }
    if (isUnsafeRoot(path)) errors.push(`absolute or drive-rooted path rejected: ${path}`);
    if (hasDotDot(path)) errors.push(`path escapes the output dir: ${path}`);
    if (seen.has(path)) errors.push(`duplicate path: ${path}`);
    seen.add(path);
    const bytes = Buffer.byteLength(content, "utf8");
    if (bytes > MAX_FILE_BYTES) errors.push(`file too large (${bytes} bytes): ${path}`);
    total += bytes;
    files.push({ path, content });
  });

  if (total > MAX_TOTAL_BYTES) errors.push(`total output too large (${total} bytes)`);
  if (files.length === 0 && errors.length === 0) errors.push("no files in output");

  return errors.length > 0 ? { ok: false, errors } : { ok: true, files };
}

/** Strip a single leading/trailing markdown code fence if present. */
function stripFence(text: string): string {
  const fence = /^```[a-zA-Z]*\n([\s\S]*?)\n```$/;
  const m = text.trim().match(fence);
  return m ? m[1] : text;
}

/** Reject anything that is not a pure relative path under the output dir:
 *  POSIX absolute (`/x`), Windows root-relative or UNC (`\x`, `\\srv`), and ANY
 *  Windows drive prefix (`C:\x`, `C:/x`, and crucially the drive-RELATIVE forms
 *  `C:x` / `C:..\x` that resolve against the current dir of drive C: — an
 *  escape a separator-requiring check would miss). */
function isUnsafeRoot(p: string): boolean {
  return p.startsWith("/") || p.startsWith("\\") || /^[a-zA-Z]:/.test(p);
}

function hasDotDot(p: string): boolean {
  return p.split(/[\\/]/).includes("..");
}
