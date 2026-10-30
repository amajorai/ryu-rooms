import type { z } from "zod";
import {
	guestExchangeBoundarySchema,
	guestTurnBoundarySchema,
	roomFailureBoundarySchema,
} from "./schemas.ts";
import type { RoomSnapshot } from "./types.ts";

const API = "/api/rooms";

async function request<T>(
	schema: z.ZodType<T>,
	path: string,
	init?: RequestInit
): Promise<T> {
	const response = await fetch(`${API}${path}`, {
		credentials: "include",
		...init,
		headers: {
			"content-type": "application/json",
			...init?.headers,
		},
	});
	const value = await response.json().catch(() => ({}));
	if (!response.ok) {
		throw new Error(roomFailureBoundarySchema.parse(value));
	}
	return schema.parse(value);
}

export function exchangeInvite(
	invite: string,
	displayName: string
): Promise<RoomSnapshot> {
	return request(guestExchangeBoundarySchema, "/guest/exchange", {
		body: JSON.stringify({ displayName, invite }),
		method: "POST",
	});
}

export function submitGuestTurn(
	text: string,
	idempotencyKey: string
): Promise<RoomSnapshot> {
	return request(guestTurnBoundarySchema, "/guest/turns", {
		body: JSON.stringify({ idempotencyKey, text }),
		method: "POST",
	});
}
