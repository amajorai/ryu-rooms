import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { viteSingleFile } from "vite-plugin-singlefile";

const here = dirname(fileURLToPath(import.meta.url));

export default defineConfig({
	base: "./",
	plugins: [react(), viteSingleFile()],
	root: here,
	build: {
		assetsInlineLimit: Number.POSITIVE_INFINITY,
		cssCodeSplit: false,
		emptyOutDir: true,
		modulePreload: { polyfill: false },
		outDir: "dist",
		rollupOptions: {
			input: { index: resolve(here, "index.html") },
			output: { inlineDynamicImports: true },
		},
		target: "esnext",
	},
});
