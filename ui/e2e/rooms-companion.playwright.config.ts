import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig, devices } from "@playwright/test";

const e2eDirectory = path.dirname(fileURLToPath(import.meta.url));
const appRoot = path.resolve(e2eDirectory, "..");
const baseURL = "http://127.0.0.1:4181";

export default defineConfig({
	fullyParallel: false,
	reporter: "list",
	testDir: e2eDirectory,
	testMatch: "**/*.spec.ts",
	workers: 1,
	outputDir: "/tmp/ryu-rooms-companion-playwright-results",
	use: {
		...devices["Desktop Chrome"],
		baseURL,
		viewport: { width: 1440, height: 1000 },
	},
	webServer: {
		command:
			"bun run vite --config=vite.config.ts --host 127.0.0.1 --port 4181 --strictPort",
		cwd: appRoot,
		url: baseURL,
		reuseExistingServer: false,
		timeout: 120_000,
	},
});
