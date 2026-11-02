import { defineConfig } from "@playwright/test";

const host = "127.0.0.1";
const port = 4180;
const baseURL = `http://${host}:${port}`;

export default defineConfig({
	fullyParallel: false,
	reporter: "list",
	testDir: "./e2e",
	testMatch: "**/*.spec.ts",
	use: {
		baseURL,
		browserName: "chromium",
		viewport: { height: 1000, width: 1440 },
	},
	webServer: {
		command: `bun run build && ./node_modules/.bin/vite preview --host ${host} --port ${port} --strictPort`,
		reuseExistingServer: !process.env.CI,
		timeout: 120_000,
		url: baseURL,
	},
});
