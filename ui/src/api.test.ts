import { afterEach, expect, test } from "bun:test";
import {
	createRoom,
	listRooms,
	resolveShareOrigins,
	submitTurn,
} from "./api.ts";

const previousWindow = (globalThis as { window?: unknown }).window;

afterEach(() => {
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: previousWindow,
	});
});

const room = {
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
	const calls: unknown[] = [];
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: {
			ryu: {
				app: {
					request: async (input: unknown) => {
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
