import { afterEach, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { exchangeInvite, submitGuestTurn } from "./guest-api.ts";
import {
	guestRoomEventSchema,
	meshModelCatalogBoundarySchema,
	roomCreateBoundarySchema,
	roomFailureBoundarySchema,
	roomListResponseSchema,
	roomSnapshotSchema,
	roomTurnBoundarySchema,
} from "./schemas.ts";

const room = roomSnapshotSchema.parse(
	JSON.parse(
		readFileSync(
			new URL("../../contracts/rooms-v1.example.json", import.meta.url),
			"utf8"
		)
	)
);
const originalFetch = globalThis.fetch;
afterEach(() => {
	Object.defineProperty(globalThis, "fetch", {
		configurable: true,
		value: originalFetch,
	});
});

test("Rust response owners retain complete rooms and optional legacy envelope fields", () => {
	expect(
		roomCreateBoundarySchema.parse({
			room,
			joinUrl: "https://node/guest#invite=token",
		})
	).toEqual({ room, joinUrl: "https://node/guest#invite=token" });
	expect(
		roomTurnBoundarySchema.parse({ snapshot: room, requestId: "request-1" })
			.snapshot
	).toEqual(room);
	expect(
		roomListResponseSchema.parse({ rooms: [room, {}, null] }).rooms
	).toEqual([room]);
	expect(() =>
		roomCreateBoundarySchema.parse({ joinUrl: 7, room: null })
	).toThrow("returned no invite link");
	expect(() =>
		roomCreateBoundarySchema.parse({ joinUrl: "valid", room: null })
	).toThrow("invalid room snapshot");
});

test("guest SSE accepts raw snapshots, envelope snapshots and future JSON metadata", () => {
	expect(guestRoomEventSchema.parse(JSON.stringify(room))).toEqual({
		snapshot: room,
		partialText: null,
		clearPartial: true,
	});
	expect(
		guestRoomEventSchema.parse(
			JSON.stringify({ snapshot: room, partialText: "" })
		)
	).toEqual({ snapshot: room, partialText: "", clearPartial: true });
	expect(
		guestRoomEventSchema.parse(
			JSON.stringify({
				snapshot: room,
				partialText: "streaming",
				future: { counter: 4 },
			})
		)
	).toEqual({ snapshot: room, partialText: "streaming", clearPartial: false });
	expect(
		guestRoomEventSchema.parse('{"partialText":7,"__proto__":{"safe":true}}')
	).toEqual({ snapshot: null, partialText: null, clearPartial: false });
	expect(() => guestRoomEventSchema.parse("invalid json")).toThrow();
});

test("guest failure decoding preserves native error messages and legacy fallback", () => {
	expect(
		roomFailureBoundarySchema.parse({
			code: "denied",
			message: "The invite expired.",
			requestId: "request-1",
		})
	).toBe("The invite expired.");
	expect(roomFailureBoundarySchema.parse({ message: 7 })).toBe(
		"The room request was not accepted."
	);
	expect(roomFailureBoundarySchema.parse(null)).toBe(
		"The room request was not accepted."
	);
});

test("guest operations preserve exact cookie-bearing request bodies and reject malformed snapshots", async () => {
	const calls: { url: string; init: RequestInit | undefined }[] = [];
	const replies = [
		{ room },
		{ snapshot: room },
		{ room: null },
		{ snapshot: null },
	];
	Object.defineProperty(globalThis, "fetch", {
		configurable: true,
		value: async (input: Parameters<typeof fetch>[0], init?: RequestInit) => {
			calls.push({ url: String(input), init });
			return new Response(JSON.stringify(replies.shift()), {
				status: 200,
				headers: { "content-type": "application/json" },
			});
		},
	});
	expect(await exchangeInvite("opaque", "Guest")).toEqual(room);
	expect(await submitGuestTurn("Hello", "guest-1")).toEqual(room);
	expect(calls).toEqual([
		{
			url: "/api/rooms/guest/exchange",
			init: {
				credentials: "include",
				method: "POST",
				body: '{"displayName":"Guest","invite":"opaque"}',
				headers: { "content-type": "application/json" },
			},
		},
		{
			url: "/api/rooms/guest/turns",
			init: {
				credentials: "include",
				method: "POST",
				body: '{"idempotencyKey":"guest-1","text":"Hello"}',
				headers: { "content-type": "application/json" },
			},
		},
	]);
	await expect(exchangeInvite("invalid", "Guest")).rejects.toThrow(
		"The invite response was not a valid room."
	);
	await expect(submitGuestTurn("Hello", "guest-2")).rejects.toThrow(
		"The turn response was not a valid room."
	);
});

test("guest non-success HTTP responses surface the genuine server failure", async () => {
	Object.defineProperty(globalThis, "fetch", {
		configurable: true,
		value: async () =>
			new Response(
				'{"code":"invite_expired","message":"Invite expired","requestId":"req-1"}',
				{ status: 403 }
			),
	});
	await expect(exchangeInvite("expired", "Guest")).rejects.toThrow(
		"Invite expired"
	);
});

test("Mesh catalog parsing retains full validated native row identity", () => {
	const row = {
		id: "mesh-model",
		name: "Model",
		nativeMetadata: { contextWindow: 32_000 },
	};
	const catalog = meshModelCatalogBoundarySchema.parse({
		"mesh-llm": [row, { id: 7, name: "bad" }],
	});
	expect(catalog).toEqual([row]);
	expect(Object.is(catalog[0], row)).toBe(true);
});
