import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";
import type { z } from "zod";
import type { RyuRoomBridge } from "../src/ryu.d.ts";
import type { RoomsAppResponse } from "../src/schemas.ts";
import {
	guestExchangeRequestSchema,
	roomTurnRequestSchema,
} from "../src/schemas.ts";
import type { RoomSnapshot } from "../src/types.ts";
import type {
	GuestRoomEvent,
	HostFixture,
	HostRequest,
} from "./fixture-types.ts";

const shareOrigin = "http://192.168.1.20:7980";
const roomId = "room_7f3c9b2a4d5e6f70";
const evidenceDirectory = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"evidence"
);

const idleRoom: RoomSnapshot = {
	contract: "rooms/1",
	schemaVersion: 1,
	id: roomId,
	status: "idle",
	modelId: "llama3-8b",
	engine: "mesh-llm",
	messages: [],
	currentRun: null,
	participants: [
		{ id: "member_1", role: "host", displayName: "Host", online: true },
	],
	updatedAt: "2026-09-28T10:00:00.000Z",
};

const runningRoom: RoomSnapshot = {
	...idleRoom,
	status: "running",
	currentRun: {
		runId: "run_rooms_companion_1",
		requestId: "request_rooms_companion_1",
		partialText: "A local fixture is responding",
		status: "running",
		errorCode: null,
		errorMessage: null,
		idempotencyKey: "fixture-turn-1",
		startedAt: "2026-09-28T10:01:00.000Z",
		finishedAt: null,
	},
};

interface GuestRequest {
	body:
		| z.output<typeof guestExchangeRequestSchema>
		| z.output<typeof roomTurnRequestSchema>;
	path: string;
}

async function installHostFixture(
	page: import("@playwright/test").Page,
	rooms: RoomSnapshot[] = []
): Promise<void> {
	await page.addInitScript(
		({ origin, initialRooms, initialRoom, activeRun }) => {
			const fixture: HostFixture = {
				calls: [],
				rooms: initialRooms,
				inviteUrl: `${origin}/api/rooms/guest#invite=host-fixture-invite`,
			};
			window.__roomsHostFixture = fixture;
			window.__roomsCopiedText = null;
			Object.defineProperty(navigator, "clipboard", {
				configurable: true,
				value: {
					writeText: async (text: string) => {
						window.__roomsCopiedText = text;
					},
				},
			});
			const ryu: RyuRoomBridge = {
				app: {
					request: async ({
						body,
						method = "GET",
						path,
					}: HostRequest): Promise<RoomsAppResponse> => {
						fixture.calls.push({ body, method, path });
						if (method === "GET" && path === "/") {
							return { rooms: fixture.rooms };
						}
						if (method === "POST" && path === "/") {
							fixture.rooms = [initialRoom];
							return {
								joinUrl: fixture.inviteUrl,
								room: initialRoom,
							};
						}
						if (method === "POST" && path === `/${initialRoom.id}/turns`) {
							const snapshot: RoomSnapshot = {
								...initialRoom,
								messages: [
									{
										id: "fixture-user-message",
										role: "user",
										text: body && "text" in body ? body.text : "",
										participantId: "member_1",
										createdAt: "2026-09-28T10:02:00.000Z",
									},
									{
										id: "fixture-assistant-message",
										role: "assistant",
										text: "The local room fixture replied.",
										participantId: null,
										createdAt: "2026-09-28T10:02:01.000Z",
									},
								],
								currentRun: null,
								status: "idle",
							};
							fixture.rooms = [snapshot];
							return { snapshot };
						}
						if (method === "POST" && path === `/${initialRoom.id}/stop`) {
							const snapshot: RoomSnapshot = {
								...activeRun,
								status: "idle",
								currentRun: null,
							};
							fixture.rooms = [snapshot];
							return { snapshot };
						}
						if (method === "POST" && path === `/${initialRoom.id}/close`) {
							fixture.rooms = [];
							return {
								snapshot: { ...initialRoom, status: "closed" },
							};
						}
						if (method === "POST" && path === `/${initialRoom.id}/invite`) {
							return { joinUrl: fixture.inviteUrl };
						}
						if (
							method === "POST" &&
							path === `/${initialRoom.id}/invite/revoke`
						) {
							return {};
						}
						throw new Error(`Unexpected local Rooms request ${method} ${path}`);
					},
				},
				node: {
					shareOrigins: async () => [
						{ origin, source: "active", reachable: true },
					],
				},
				registry: {
					engineModels: async () => ({
						"mesh-llm": [
							{ id: "llama3-8b", name: "Llama 3 8B" },
							{ id: "qwen3-8b", name: "Qwen 3 8B" },
						],
					}),
				},
			};
			Object.defineProperty(window, "ryu", {
				configurable: true,
				writable: true,
				value: ryu,
			});
		},
		{
			origin: shareOrigin,
			initialRooms: rooms,
			initialRoom: idleRoom,
			activeRun: runningRoom,
		}
	);
}

async function readHostCalls(page: import("@playwright/test").Page) {
	return page.evaluate(() => window.__roomsHostFixture?.calls ?? []);
}

test("refreshes the room list and opens the selected room snapshot", async ({
	page,
}) => {
	await installHostFixture(page);
	await page.goto("/");
	await expect(page.getByText("No rooms yet")).toBeVisible();
	const readsBeforeRefresh = (await readHostCalls(page)).filter(
		(call) => call.method === "GET" && call.path === "/"
	).length;

	await page.evaluate(
		(room) => {
			const fixture = window.__roomsHostFixture;
			fixture.rooms = [room];
		},
		{
			...idleRoom,
			modelId: "qwen3-8b",
			messages: [
				{
					id: "fixture-existing-message",
					role: "assistant",
					text: "This saved snapshot loaded from the room list.",
					participantId: null,
					createdAt: "2026-09-28T10:00:00.000Z",
				},
			],
		} satisfies RoomSnapshot
	);
	await page.getByRole("button", { name: "Refresh" }).click();
	const savedRoom = page
		.getByRole("listbox", { name: "Your rooms" })
		.getByRole("option", { name: /qwen3-8b/ });
	await expect(savedRoom).toBeVisible();
	const readsAfterRefresh = (await readHostCalls(page)).filter(
		(call) => call.method === "GET" && call.path === "/"
	).length;
	expect(readsAfterRefresh).toBe(readsBeforeRefresh + 1);
	// Refresh rereads the active node's room collection and displays its new snapshot.
	await savedRoom.click();
	await expect(
		page.getByText("This saved snapshot loaded from the room list.")
	).toBeVisible();
	await expect(
		page.getByText(`${shareOrigin} · qwen3-8b`, { exact: true })
	).toBeVisible();
	await mkdir(evidenceDirectory, { recursive: true });
	await page.screenshot({
		animations: "disabled",
		fullPage: true,
		path: path.join(evidenceDirectory, "rooms-refreshed-selected-snapshot.png"),
	});
	// Selecting the row opens that exact snapshot's transcript and model.
	await expect(
		page.getByRole("heading", { name: roomId.slice(0, 18) })
	).toBeVisible();
});

test("guest joins with the edited name and sends a prompt through the room session", async ({
	page,
}) => {
	const requests: GuestRequest[] = [];
	const guestReply: RoomSnapshot = {
		...idleRoom,
		participants: [
			...idleRoom.participants,
			{
				id: "member_guest_1",
				role: "guest",
				displayName: "Guest Regression",
				online: true,
			},
		],
	};
	const guestTurnReply: RoomSnapshot = {
		...guestReply,
		messages: [
			{
				id: "guest-message-1",
				role: "user",
				text: "Summarize the room state",
				participantId: "member_guest_1",
				createdAt: "2026-09-28T10:03:00.000Z",
			},
		],
	};
	await page.addInitScript(() => {
		class FixtureEventSource {
			addEventListener(): void {}
			close(): void {}
		}
		Object.defineProperty(window, "EventSource", {
			configurable: true,
			value: FixtureEventSource,
		});
	});
	await page.route("**/api/rooms/guest/exchange", async (route) => {
		const body = guestExchangeRequestSchema.parse(
			route.request().postDataJSON()
		);
		requests.push({ body, path: new URL(route.request().url()).pathname });
		await route.fulfill({
			contentType: "application/json",
			body: JSON.stringify({ room: guestReply }),
		});
	});
	await page.route("**/api/rooms/guest/turns", async (route) => {
		const body = roomTurnRequestSchema.parse(route.request().postDataJSON());
		requests.push({ body, path: new URL(route.request().url()).pathname });
		await route.fulfill({
			contentType: "application/json",
			body: JSON.stringify({ snapshot: guestTurnReply }),
		});
	});

	await page.goto("/e2e/guest-room.html");
	await expect(
		page.getByRole("heading", { name: "Join the conversation." })
	).toBeVisible();
	const name = page.getByLabel("Your name");
	await name.fill("Guest Regression");
	// Name edits feed the actual invite exchange body.
	await expect(name).toHaveValue("Guest Regression");
	const joinButton = page.getByRole("button", { name: "Join room" });
	await expect(joinButton).toBeEnabled();
	await joinButton.click();
	await expect(
		page.getByRole("heading", { name: "Room conversation" })
	).toBeVisible();
	expect(requests[0]).toEqual({
		path: "/api/rooms/guest/exchange",
		body: {
			displayName: "Guest Regression",
			invite: "rooms-companion-invite-token",
		},
	});
	// The Join room button's native form action exchanges the invite and reveals the selected room.
	await expect(page.getByText("llama3-8b", { exact: true })).toBeVisible();

	const prompt = page.getByPlaceholder("Ask the room…");
	await prompt.fill("  Summarize the room state  ");
	// Guest composer input retains the edited prompt until it is accepted.
	await expect(prompt).toHaveValue("  Summarize the room state  ");
	await prompt.press("Enter");
	await expect(prompt).toHaveValue("");
	await expect(
		page.getByText("Summarize the room state", { exact: true })
	).toBeVisible();
	const turnCall = requests.find(
		(request) => request.path === "/api/rooms/guest/turns"
	);
	expect(roomTurnRequestSchema.parse(turnCall?.body).text).toBe(
		"Summarize the room state"
	);
	expect(roomTurnRequestSchema.parse(turnCall?.body).idempotencyKey).toMatch(
		/^guest-\d+(?:-[a-z0-9]+)?$/
	);
	await mkdir(evidenceDirectory, { recursive: true });
	await page.screenshot({
		animations: "disabled",
		fullPage: true,
		path: path.join(evidenceDirectory, "rooms-guest-prompt-synchronized.png"),
	});
});

test("standalone invite page joins and renders the synchronized guest prompt", async ({
	page,
}) => {
	const requests: GuestRequest[] = [];
	const joinedRoom: RoomSnapshot = {
		...idleRoom,
		participants: [
			...idleRoom.participants,
			{
				id: "member_standalone_guest",
				role: "guest",
				displayName: "Standalone Guest",
				online: true,
			},
		],
	};
	const submittedRoom: RoomSnapshot = {
		...joinedRoom,
		messages: [
			{
				id: "standalone-guest-message",
				role: "user",
				text: "Show the shared transcript",
				participantId: "member_standalone_guest",
				createdAt: "2026-09-28T10:04:00.000Z",
			},
		],
	};
	await page.addInitScript(() => {
		const sources: FixtureEventSource[] = [];
		class FixtureEventSource extends EventTarget {
			constructor() {
				super();
				sources.push(this);
			}
			close(): void {}
			emit(name: string, data: GuestRoomEvent): void {
				this.dispatchEvent(
					new MessageEvent(name, { data: JSON.stringify(data) })
				);
			}
		}
		Object.defineProperty(window, "EventSource", {
			configurable: true,
			value: FixtureEventSource,
		});
		window.__roomsEmitEvent = (name: string, data: GuestRoomEvent) => {
			for (const source of sources) {
				source.emit(name, data);
			}
		};
	});
	await page.route("**/api/rooms/guest/exchange", async (route) => {
		requests.push({
			body: guestExchangeRequestSchema.parse(route.request().postDataJSON()),
			path: new URL(route.request().url()).pathname,
		});
		await route.fulfill({
			contentType: "application/json",
			body: JSON.stringify({ room: joinedRoom }),
		});
	});
	await page.route("**/api/rooms/guest/turns", async (route) => {
		requests.push({
			body: roomTurnRequestSchema.parse(route.request().postDataJSON()),
			path: new URL(route.request().url()).pathname,
		});
		await route.fulfill({
			contentType: "application/json",
			body: JSON.stringify({ snapshot: submittedRoom }),
		});
	});

	await page.goto("/guest.html#invite=rooms-standalone-token");
	await page.getByLabel("Your name").fill("Standalone Guest");
	await page.getByRole("button", { name: "Join room" }).click();
	await expect(
		page.getByRole("heading", { name: roomId.slice(0, 16) })
	).toBeVisible();
	expect(requests[0]).toEqual({
		path: "/api/rooms/guest/exchange",
		body: {
			invite: "rooms-standalone-token",
			displayName: "Standalone Guest",
		},
	});
	expect(page.url()).not.toContain("#invite=");

	const prompt = page.getByLabel("Message to room");
	await prompt.fill("Show the shared transcript");
	await page.getByRole("button", { name: "Send", exact: true }).click();
	await expect(prompt).toHaveValue("");
	const turnCall = requests.find(
		(request) => request.path === "/api/rooms/guest/turns"
	);
	expect(roomTurnRequestSchema.parse(turnCall?.body).text).toBe(
		"Show the shared transcript"
	);
	await page.evaluate((snapshot) => {
		window.__roomsEmitEvent("turn.accepted", { snapshot });
	}, submittedRoom);
	await expect(
		page.getByText("Show the shared transcript", { exact: true })
	).toBeVisible();
	await mkdir(evidenceDirectory, { recursive: true });
	await page.screenshot({
		animations: "disabled",
		fullPage: true,
		path: path.join(evidenceDirectory, "rooms-standalone-guest-prompt.png"),
	});
});

test("host submits a prompt and copies the active invite URL", async ({
	page,
}) => {
	await installHostFixture(page);
	await page.goto("/");
	await page.getByRole("button", { name: "Start room" }).click();
	await expect(
		page.getByText("Your room is ready for its first prompt.")
	).toBeVisible();

	const prompt = page.getByPlaceholder("Ask the room…");
	await prompt.fill("  Explain the saved room state  ");
	// The controlled InputBar change becomes the exact prompt submitted by the host.
	await expect(prompt).toHaveValue("  Explain the saved room state  ");
	await prompt.press("Enter");
	await expect(page.getByText("The local room fixture replied.")).toBeVisible();
	await expect(prompt).toHaveValue("");
	const calls = await readHostCalls(page);
	expect(calls).toContainEqual({
		method: "POST",
		path: `/${roomId}/turns`,
		body: {
			text: "Explain the saved room state",
			idempotencyKey: expect.stringMatching(/^host-/),
		},
	});

	await page.getByRole("button", { name: "Copy link" }).click();
	await expect(page.getByRole("button", { name: "Copied" })).toBeVisible();
	const copiedText = await page.evaluate(() => window.__roomsCopiedText);
	expect(copiedText).toBe(
		`${shareOrigin}/api/rooms/guest#invite=host-fixture-invite`
	);
	await mkdir(evidenceDirectory, { recursive: true });
	await page.screenshot({
		animations: "disabled",
		fullPage: true,
		path: path.join(
			evidenceDirectory,
			"rooms-host-prompt-and-copied-invite.png"
		),
	});
});

test("stops the active generation and confirms closing the room", async ({
	page,
}) => {
	await installHostFixture(page, [runningRoom]);
	await page.goto("/");
	await page
		.getByRole("listbox", { name: "Your rooms" })
		.getByRole("option", { name: /llama3-8b/ })
		.click();
	await expect(
		page.getByRole("button", { name: "Stop generation" })
	).toBeVisible();
	await page.getByRole("button", { name: "Stop generation" }).click();
	await expect(
		page.getByRole("button", { name: "Stop generation" })
	).toHaveCount(0);
	const stopCall = (await readHostCalls(page)).find(
		(call) => call.method === "POST" && call.path === `/${roomId}/stop`
	);
	expect(stopCall).toEqual({
		method: "POST",
		path: `/${roomId}/stop`,
		body: { runId: "run_rooms_companion_1" },
	});

	await page.getByRole("button", { name: "Close room", exact: true }).click();
	const dialog = page.getByRole("alertdialog");
	await expect(dialog).toContainText("Close this room?");
	// The room close confirmation leaves the room available until this action is confirmed.
	await mkdir(evidenceDirectory, { recursive: true });
	await page.screenshot({
		animations: "disabled",
		fullPage: true,
		path: path.join(
			evidenceDirectory,
			"rooms-close-confirmation-after-stop.png"
		),
	});
	await dialog.getByRole("button", { name: "Close room", exact: true }).click();
	await expect(page.getByText("No rooms yet")).toBeVisible();
	const calls = await readHostCalls(page);
	expect(calls).toContainEqual({
		method: "POST",
		path: `/${roomId}/close`,
		body: undefined,
	});
});
