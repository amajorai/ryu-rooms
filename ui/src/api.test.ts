import { afterEach, expect, test } from "bun:test";
import {
	closeRoom,
	createRoom,
	getRoom,
	issueInvite,
	listMeshModels,
	listRooms,
	resolveShareOrigins,
	revokeInvite,
	stopRun,
	submitTurn,
} from "./api.ts";
import type { RyuRoomBridge } from "./ryu.d.ts";
import type { RoomSnapshot } from "./types.ts";

type RoomRequest = Parameters<RyuRoomBridge["app"]["request"]>[0];
const previousWindow = globalThis.window;

afterEach(() => {
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: previousWindow,
	});
});

const room: RoomSnapshot = {
	contract: "rooms/1",
	schemaVersion: 1,
	id: "room_7f3c9b2a4d5e6f70",
	status: "idle",
	modelId: "qwen3-8b",
	engine: "mesh-llm",
	messages: [],
	currentRun: null,
	participants: [],
	updatedAt: "2026-08-30T08:00:00Z",
};

test("Room API uses relative own-app paths and keeps the host bridge opaque", async () => {
	const calls: RoomRequest[] = [];
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: {
			ryu: {
				app: {
					request: async (input: RoomRequest) => {
						calls.push(input);
						if (calls.length === 1) {
							return { rooms: [room] };
						}
						if (calls.length === 2) {
							return {
								room,
								joinUrl: "http://node.example/api/rooms/guest#invite=opaque",
							};
						}
						return { snapshot: room };
					},
				},
				node: {
					shareOrigins: async () => [
						{
							origin: "http://node.example:7980",
							source: "active",
							reachable: true,
						},
					],
				},
			},
		},
	});

	expect(await listRooms()).toHaveLength(1);
	expect(
		await createRoom("qwen3-8b", "http://node.example:7980")
	).toMatchObject({ joinUrl: expect.stringContaining("#invite=") });
	await submitTurn(room.id, "hello", "turn-1");
	expect(await resolveShareOrigins()).toEqual([
		{ origin: "http://node.example:7980", source: "active", reachable: true },
	]);
	expect(calls).toEqual([
		{ method: "GET", path: "/", body: undefined },
		{
			body: { modelId: "qwen3-8b", shareOrigin: "http://node.example:7980" },
			method: "POST",
			path: "/",
		},
		{
			body: { idempotencyKey: "turn-1", text: "hello" },
			method: "POST",
			path: "/room_7f3c9b2a4d5e6f70/turns",
		},
	]);
});

test("Room controls encode identifiers and preserve exact sidecar verbs and payloads", async () => {
	const calls: RoomRequest[] = [];
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: {
			ryu: {
				app: {
					request: async (input: RoomRequest) => {
						calls.push(input);
						if (input.path.endsWith("/invite")) {
							return { joinUrl: "https://node.example/guest#invite=next" };
						}
						if (input.path.endsWith("/invite/revoke")) {
							return {};
						}
						return input.path === "/room%2Fwith%20spaces"
							? room
							: { snapshot: room };
					},
				},
			},
		},
	});

	const id = "room/with spaces";
	expect(await getRoom(id)).toEqual(room);
	await stopRun(id, "run-7");
	await closeRoom(id);
	expect(await issueInvite(id)).toEqual({
		joinUrl: "https://node.example/guest#invite=next",
	});
	await revokeInvite(id);
	expect(calls).toEqual([
		{ body: undefined, method: "GET", path: "/room%2Fwith%20spaces" },
		{
			body: { runId: "run-7" },
			method: "POST",
			path: "/room%2Fwith%20spaces/stop",
		},
		{ body: undefined, method: "POST", path: "/room%2Fwith%20spaces/close" },
		{ body: undefined, method: "POST", path: "/room%2Fwith%20spaces/invite" },
		{
			body: undefined,
			method: "POST",
			path: "/room%2Fwith%20spaces/invite/revoke",
		},
	]);
});

test("Room API refuses missing host capability and malformed required responses", async () => {
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: {},
	});
	await expect(listRooms()).rejects.toThrow("Rooms host bridge is unavailable");

	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: { ryu: { app: { request: async () => ({ rooms: "invalid" }) } } },
	});
	expect(await listRooms()).toEqual([]);
	await expect(createRoom("model", "https://node.example")).rejects.toThrow(
		"returned no invite link"
	);
	await expect(getRoom("missing")).rejects.toThrow("invalid room snapshot");
});

test("Mesh model discovery filters malformed entries and degrades only registry failures", async () => {
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: {
			ryu: {
				registry: {
					engineModels: async () => ({
						"mesh-llm": [
							{ id: "model-a", name: "Model A" },
							{ id: 7, name: "bad id" },
							{ id: "bad-name", name: null },
						],
					}),
				},
			},
		},
	});
	expect(await listMeshModels()).toEqual([{ id: "model-a", name: "Model A" }]);

	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: {
			ryu: {
				registry: {
					engineModels: async () => Promise.reject(new Error("offline")),
				},
			},
		},
	});
	expect(await listMeshModels()).toEqual([]);
});
