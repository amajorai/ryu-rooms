import { parseRoomSnapshot, type RoomSnapshot } from "./types.ts";

const API = "/api/rooms";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
	const response = await fetch(`${API}${path}`, {
		credentials: "include",
		...init,
		headers: {
			"content-type": "application/json",
			...(init?.headers ?? {}),
		},
	});
	const value = await response.json().catch(() => ({}));
	if (!response.ok) {
		const message =
			typeof value.message === "string"
				? value.message
				: "The room request was not accepted.";
		throw new Error(message);
	}
	return value as T;
}

export async function exchangeInvite(
	invite: string,
	displayName: string
): Promise<RoomSnapshot> {
	const value = await request<{ room?: unknown }>("/guest/exchange", {
		body: JSON.stringify({ displayName, invite }),
		method: "POST",
	});
	const room = parseRoomSnapshot(value.room);
	if (!room) {
		throw new Error("The invite response was not a valid room.");
	}
	return room;
}

export async function submitGuestTurn(
	text: string,
	idempotencyKey: string
): Promise<RoomSnapshot> {
	const value = await request<{ snapshot?: unknown }>("/guest/turns", {
		body: JSON.stringify({ idempotencyKey, text }),
		method: "POST",
	});
	const room = parseRoomSnapshot(value.snapshot);
	if (!room) {
		throw new Error("The turn response was not a valid room.");
	}
	return room;
}
