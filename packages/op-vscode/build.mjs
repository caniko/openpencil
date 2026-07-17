import esbuild from "esbuild";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

const watch = process.argv.includes("--watch");

// Extract the bundled OpenPencil design skill markdown from the CLI's skill
// bundle into dist/assets so the "Install AI Skill" command can ship it. The
// name openpencil-design lives only here, NOT in the op-ai-skills registry.
function extractSkill() {
  const src = "../../crates/op-cli/assets/skill-bundle.json";
  const out = "dist/assets/openpencil-skill.md";
  try {
    const bundle = JSON.parse(readFileSync(src, "utf8"));
    // `files` is a flat map keyed by full relative path.
    const md = bundle?.files?.["skills/openpencil-design/SKILL.md"];
    if (typeof md !== "string") throw new Error("SKILL.md not found in skill-bundle.json");
    mkdirSync(dirname(out), { recursive: true });
    writeFileSync(out, md);
    console.log(`extracted skill (${md.length} bytes) → ${out}`);
  } catch (err) {
    console.warn(`skill extraction skipped: ${err.message}`);
  }
}

extractSkill();

const ctx = await esbuild.context({
  entryPoints: ["src/extension.ts"],
  bundle: true,
  outfile: "dist/extension.js",
  external: ["vscode"],
  format: "cjs",
  platform: "node",
  target: "node18",
  sourcemap: true,
});

if (watch) {
  await ctx.watch();
} else {
  await ctx.rebuild();
  await ctx.dispose();
}
