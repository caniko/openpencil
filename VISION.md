# OpenPencil vision

An AI-native vector design tool, implemented in Rust with a wasm-backed web SDK.
A conversational agent draws on an infinite canvas, concurrent agent teams split
a page into spatial sub-tasks, and `.op` documents stay JSON — human-readable,
Git-friendly, and diffable. This checkout is the `caniko` fork of
`ZSeven-W/openpencil` (kept as the `upstream-zw` remote), extended with a
deterministic headless export pipeline.

## Problem

Design tools keep authored design in a binary or proprietary format, so design
cannot be reviewed, diffed, or consumed by code the way source can. AI design
tools go further and produce output that only their own viewer can render, which
leaves UI consumers rewriting an ad-hoc translation layer per project.

## Principles

- **Design-as-code.** `.op` files are JSON; design variables generate CSS custom
  properties, and code export targets HTML, React, Vue, Svelte, Flutter, SwiftUI,
  Compose and React Native, with React emitting inline styles rather than
  Tailwind classes.
- **Rust is the product.** The TypeScript apps and `pen-*` packages are retired;
  the product is the `crates/` workspace plus a thin wasm web SDK. Historical
  `// ported from pen-*` comments name the retired sources.
- **Deterministic export is a first-class output.** A headless reference
  renderer ships as a reproducible Nix output, and OPUI export writes a manifest
  plus local, content-addressed asset sidecars rather than fetching at render
  time.
- **Exports are safe to watch.** The designer export path watches for changes,
  recovers after startup errors, and clears copied identities so repeated exports
  do not smuggle stale or duplicate runtime IDs into the output.
- **Agents are a supported user, not a plugin.** The orchestrator, ACP client,
  and MCP tool registry are in-tree, and AI skills/style guides are project data
  with their own loaders and audits.
- **Typed edges are deliberate.** Canonical gradient endpoints, ellipse
  fallbacks, and native-versus-exported typography alignment are fixed at the
  renderer boundary instead of diverging per host.

## Scope

The Rust workspace: editor core and UI, native/web/desktop hosts, AI and
orchestrator crates, codegen, MCP, Figma interchange, merge and Git crates,
design lint, the `op` CLI, the `openpencil-desktop` app, the reference renderer,
and the OPUI runtime exporter; plus the wasm web SDK under `packages/`, a
multi-language README set, and vendored agent/casement sources.

The MCP server lets an agent read, create, and modify `.op` documents, either
against a file through `openpencil-desktop --mcp <path>` or through the live
desktop editor, where the document must be open.

## Non-goals

- It is not the other open-source project with the same name that focuses on
  Figma-compatible real-time collaboration; this one focuses on AI-native
  design-to-code workflows.
- OPUI export targets a data-only interchange contract: no scripts, animation,
  interaction handlers, embedded binaries, or automatically fetched network
  resources.

## Direction

Recent work builds the runtime-UI export lane: a safe designer export watch,
semantic and component metadata authoring by node ID, live component identity
resolution, and alignment of native and exported typography and gradients.

## Place in the fleet

The `open-pencil` MCP server canix registers for agents
(`home/modules/ai/open-pencil.nix`, `docs/src/operations/mcp.md`) is the
unrelated `open-pencil/open-pencil` project, not this repository. The `opui`
contract and `opui-integration` harness consume this repository's exporter
output and reference-renderer artifacts, and Atlas's update automation lists the
repository while disabling its Simit enrollment.
