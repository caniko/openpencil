import { test, expect } from "bun:test";
import { buildCodegenPrompt, parseCodegenOutput } from "./codegen-prompt";

test("buildCodegenPrompt embeds the framework, the JSON contract, and the doc", () => {
  const p = buildCodegenPrompt('{"pages":[]}', "vue");
  expect(p).toContain("vue");
  expect(p).toContain('{"files":[{"path"');
  expect(p).toContain('{"pages":[]}');
});

test("parses a clean files envelope", () => {
  const out = parseCodegenOutput('{"files":[{"path":"a/B.tsx","content":"x"}]}');
  expect(out.ok).toBe(true);
  if (out.ok) {
    expect(out.files.length).toBe(1);
    expect(out.files[0].path).toBe("a/B.tsx");
  }
});

test("tolerates a ```json fenced block", () => {
  const fenced = "```json\n" + '{"files":[{"path":"a.tsx","content":"y"}]}' + "\n```";
  const out = parseCodegenOutput(fenced);
  expect(out.ok).toBe(true);
});

test("rejects non-JSON output", () => {
  const out = parseCodegenOutput("Here is your code: ...");
  expect(out.ok).toBe(false);
  if (!out.ok) expect(out.errors[0]).toContain("not valid JSON");
});

test("rejects a non-files shape", () => {
  const out = parseCodegenOutput('{"result":"ok"}');
  expect(out.ok).toBe(false);
});

test("rejects absolute paths", () => {
  const out = parseCodegenOutput('{"files":[{"path":"/etc/passwd","content":"x"}]}');
  expect(out.ok).toBe(false);
  if (!out.ok) expect(out.errors.some((e) => e.includes("absolute or drive-rooted"))).toBe(true);
});

test("rejects .. traversal", () => {
  const out = parseCodegenOutput('{"files":[{"path":"../../evil.tsx","content":"x"}]}');
  expect(out.ok).toBe(false);
  if (!out.ok) expect(out.errors.some((e) => e.includes("escapes"))).toBe(true);
});

test("rejects Windows-absolute and UNC paths", () => {
  expect(parseCodegenOutput('{"files":[{"path":"C:\\\\x.tsx","content":"x"}]}').ok).toBe(false);
  expect(parseCodegenOutput('{"files":[{"path":"\\\\\\\\srv\\\\x","content":"x"}]}').ok).toBe(false);
});

test("rejects Windows drive-RELATIVE escapes (no separator after the colon)", () => {
  // C:foo resolves against the current dir of drive C: — a real escape a
  // separator-requiring check misses.
  for (const p of ["C:foo", "c:evil.tsx", "C:..\\x", "C:./x", "\\rooted"]) {
    const body = JSON.stringify({ files: [{ path: p, content: "x" }] });
    expect(parseCodegenOutput(body).ok).toBe(false);
  }
});

test("rejects duplicate paths", () => {
  const out = parseCodegenOutput(
    '{"files":[{"path":"a.tsx","content":"1"},{"path":"a.tsx","content":"2"}]}',
  );
  expect(out.ok).toBe(false);
  if (!out.ok) expect(out.errors.some((e) => e.includes("duplicate"))).toBe(true);
});

test("rejects a single file over 1 MiB", () => {
  const big = "x".repeat(1024 * 1024 + 1);
  const out = parseCodegenOutput(JSON.stringify({ files: [{ path: "big.tsx", content: big }] }));
  expect(out.ok).toBe(false);
  if (!out.ok) expect(out.errors.some((e) => e.includes("too large"))).toBe(true);
});

test("rejects total output over 10 MiB", () => {
  const chunk = "x".repeat(900 * 1024); // under per-file cap
  const files = Array.from({ length: 12 }, (_, i) => ({ path: `f${i}.tsx`, content: chunk }));
  const out = parseCodegenOutput(JSON.stringify({ files }));
  expect(out.ok).toBe(false);
  if (!out.ok) expect(out.errors.some((e) => e.includes("total output too large"))).toBe(true);
});

test("rejects an empty files array", () => {
  const out = parseCodegenOutput('{"files":[]}');
  expect(out.ok).toBe(false);
  if (!out.ok) expect(out.errors.some((e) => e.includes("no files"))).toBe(true);
});

test("collects multiple violations at once", () => {
  const out = parseCodegenOutput(
    '{"files":[{"path":"/abs","content":"x"},{"path":"../up","content":"y"}]}',
  );
  expect(out.ok).toBe(false);
  if (!out.ok) expect(out.errors.length).toBeGreaterThanOrEqual(2);
});
