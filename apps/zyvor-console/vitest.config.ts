import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

// `@/` is the app root, as in tsconfig.json, so library files that import each other that way can be tested.
export default defineConfig({ resolve: { alias: { "@": fileURLToPath(new URL(".", import.meta.url)) } } });
