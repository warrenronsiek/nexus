// @feature observability-ui
// @spec docs/features/observability-ui.md
import { defineConfig } from "vite";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

// Elm's own compiler remains the only Elm transpiler. Vite packages its output.
export default defineConfig({
  base: "/",
  plugins: [{
    name: "nexus-elm",
    resolveId(id) { return id === "virtual:nexus-elm" ? "\0nexus-elm" : undefined; },
    load(id) {
      if (id !== "\0nexus-elm") return;
      const output = resolve("elm-stuff/nexus-application.js");
      execFileSync(resolve("node_modules/.bin/elm"), ["make", "src/Main.elm", "--optimize", `--output=${output}`], { stdio: "inherit" });
      const compiled = readFileSync(output, "utf8");
      return `const scope = {}; (function(){${compiled}}).call(scope); export const Elm = scope.Elm;`;
    },
  }],
  server: { proxy: { "/ui/api": "http://127.0.0.1:8787" } },
  build: { target: "es2022", emptyOutDir: true },
});
