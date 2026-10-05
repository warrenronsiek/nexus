// @feature observability-ui
// @feature architecture-tooling
// @spec docs/features/observability-ui.md
// @spec docs/features/architecture-tooling.md
// @entrypoint terminal dashboard build
import { build } from "esbuild";
import { readFileSync } from "node:fs";

const licenses = [
  "PI-TUI-LICENSE.txt",
  "node_modules/get-east-asian-width/license",
  "node_modules/marked/LICENSE",
].map((path) => readFileSync(path, "utf8")).join("\n\n");

await build({
  entryPoints: ["src/terminal.ts"],
  outfile: "dist/nexus-tui.mjs",
  bundle: true,
  platform: "node",
  format: "esm",
  target: "node22.19",
  minify: true,
  legalComments: "eof",
  banner: { js: `/* Bundled terminal renderer licenses\n${licenses}\n*/` },
});
