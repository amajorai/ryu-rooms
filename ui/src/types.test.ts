import { describe, expect, test } from "bun:test";
import { parseRoomEvent, parseRoomSnapshot } from "./types.ts";

const snapshot = {
	contract: "rooms/1",
	schemaVersion: 1,
	id: "room_7f3c9b2a4d5e6f70",
	status: "idle",
	modelId: "qwen3-8b",
	engine: "mesh-llm",
	messages: [],
	currentRun: null,
	participants: [
		{
			id: "member_9f3c1b2a4d5e6f70",
			displayName: "Host",
			role: "host",
			online: true,
		},
	],
	updatedAt: "2026-08-30T08:00:00Z",
};

describe("Rooms contract parsing", () => {
	test("accepts the rooms/1 example and preserves an empty run", () => {
		expect(parseRoomSnapshot(snapshot)).toMatchObject({
			contract: "rooms/1",
			currentRun: null,
			id: snapshot.id,
			modelId: "qwen3-8b",
		});
	});

	test("rejects arrays, unknown statuses, and credential-shaped snapshots", () => {
		expect(parseRoomSnapshot([])).toBeNull();
		expect(parseRoomSnapshot({ ...snapshot, status: "future" })).toBeNull();
		expect(
			parseRoomSnapshot({ ...snapshot, nodeToken: "secret" })
		).not.toBeNull();
	});

	test("normalizes an unknown future event without crashing the stream", () => {
		expect(
			parseRoomEvent({ name: "turn.delta", data: { delta: "hi" } })
		).toEqual({
			data: { delta: "hi" },
			known: true,
			name: "turn.delta",
		});
		expect(parseRoomEvent({ name: "turn.future", data: { value: 1 } })).toEqual(
			{
				data: { value: 1 },
				known: false,
				name: "turn.future",
			}
		);
	});
});
