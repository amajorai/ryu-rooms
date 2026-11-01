import { afterEach, describe, expect, test } from "bun:test";
import { GlobalRegistrator } from "@happy-dom/global-registrator";
import type { Root } from "react-dom/client";
import type { RoomSnapshot } from "./types.ts";

if (!GlobalRegistrator.isRegistered) {
	GlobalRegistrator.register();
}
Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
const { act } = await import("react");
const { createRoot } = await import("react-dom/client");
const { App } = await import("./App.tsx");

const ROOM: RoomSnapshot = {
	contract: "rooms/1",
	currentRun: null,
	engine: "mesh-llm",
	id: "room_7f3c9b2a4d5e6f70",
	messages: [],
	modelId: "llama3-8b",
	participants: [
		{
			displayName: "Host",
			id: "member_1",
			online: true,
			role: "host",
		},
	],
	schemaVersion: 1,
	status: "idle",
	updatedAt: "2026-09-28T10:00:00.000Z",
};
const CLOSED_ROOM: RoomSnapshot = { ...ROOM, status: "closed" };
const SHARE_ORIGIN = "http://192.168.1.20:7980";

interface MountedApp {
	container: HTMLDivElement;
	root: Root;
}

const mountedApps: MountedApp[] = [];

function installBridge() {
	const calls: { body?: unknown; method: string; path: string }[] = [];
	let rooms: RoomSnapshot[] = [];
	let joinUrl = `${SHARE_ORIGIN}/api/rooms/guest#invite=first-token`;
	Object.defineProperty(window, "ryu", {
		configurable: true,
		value: {
			app: {
				request: async (input: {
					body?: unknown;
					method?: string;
					path: string;
				}) => {
					const method = input.method ?? "GET";
					calls.push({ body: input.body, method, path: input.path });
					if (method === "GET" && input.path === "/") {
						return { rooms };
					}
					if (method === "POST" && input.path === "/") {
						rooms = [ROOM];
						return { joinUrl, room: ROOM };
					}
					if (method === "POST" && input.path === `/${ROOM.id}/invite`) {
						joinUrl = `${SHARE_ORIGIN}/api/rooms/guest#invite=rotated-token`;
						return { joinUrl };
					}
					if (method === "POST" && input.path === `/${ROOM.id}/invite/revoke`) {
						return {};
					}
					if (method === "POST" && input.path === `/${ROOM.id}/close`) {
						rooms = [];
						return { snapshot: CLOSED_ROOM };
					}
					throw new Error(
						`Unexpected local Rooms call: ${method} ${input.path}`
					);
				},
			},
			node: {
				shareOrigins: async () => [
					{ origin: SHARE_ORIGIN, reachable: true, source: "active" },
				],
			},
			registry: {
				engineModels: async () => ({
					"mesh-llm": [
						{ id: "qwen3-8b", name: "Qwen 3 8B" },
						{ id: "llama3-8b", name: "Llama 3 8B" },
					],
				}),
			},
		},
	});
	return calls;
}

async function tick(): Promise<void> {
	await new Promise((resolve) => setTimeout(resolve, 0));
}

async function mountApp(): Promise<HTMLDivElement> {
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	mountedApps.push({ container, root });
	await act(async () => {
		root.render(<App />);
		await tick();
		await tick();
	});
	return container;
}

function findButton(
	container: HTMLElement,
	text: string,
	last = false
): HTMLButtonElement {
	const buttons = [...container.querySelectorAll("button")].filter(
		(candidate) => candidate.textContent?.includes(text)
	);
	const button = last ? buttons.at(-1) : buttons[0];
	if (!(button instanceof HTMLButtonElement)) {
		throw new Error(`Could not find button containing: ${text}`);
	}
	return button;
}

async function clickButton(
	container: HTMLElement,
	text: string,
	last = false
): Promise<void> {
	await act(async () => {
		findButton(container, text, last).click();
		await tick();
		await tick();
	});
}

async function selectModel(
	container: HTMLElement,
	modelId: string
): Promise<void> {
	const select = container.querySelector<HTMLSelectElement>(
		'select[aria-label="Mesh LLM model"]'
	);
	if (!select) {
		throw new Error("Could not find the Mesh LLM model selector.");
	}
	await act(async () => {
		const setter = Object.getOwnPropertyDescriptor(
			Object.getPrototypeOf(select),
			"value"
		)?.set;
		setter?.call(select, modelId);
		select.dispatchEvent(new Event("change", { bubbles: true }));
		await tick();
	});
}

afterEach(async () => {
	for (const { container, root } of mountedApps.splice(0)) {
		await act(async () => root.unmount());
		container.remove();
	}
	Reflect.set(window, "ryu", undefined);
});

describe("Rooms rendered host workflow", () => {
	test("starts a room with the chosen model, rotates and revokes its invite, then closes it", async () => {
		const calls = installBridge();
		const container = await mountApp();
		expect(container.textContent).toContain("Shareable node");
		expect(container.textContent).toContain(SHARE_ORIGIN);
		await selectModel(container, "llama3-8b");
		await clickButton(container, "Start room");

		expect(container.textContent).toContain("llama3-8b");
		expect(container.textContent).toContain(
			"Your room is ready for its first prompt."
		);
		expect(container.textContent).toContain(
			`${SHARE_ORIGIN}/api/rooms/guest#invite=first-token`
		);
		expect(calls).toContainEqual({
			body: { modelId: "llama3-8b", shareOrigin: SHARE_ORIGIN },
			method: "POST",
			path: "/",
		});

		await clickButton(container, "New invite");
		expect(container.textContent).toContain("rotated-token");
		expect(calls).toContainEqual({
			method: "POST",
			path: `/${ROOM.id}/invite`,
		});
	});
});
