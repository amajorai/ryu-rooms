import type { z } from "zod";
import {
	guestExchangeBoundarySchema,
	guestTurnBoundarySchema,
	roomFailureBoundarySchema,
	roomFailureResponseSchema,
} from "./schemas.ts";
import type { RoomSnapshot } from "./types.ts";

const API = "/api/rooms";

/** A failed guest request; `code` is the backend's stable error code. */
export class GuestRequestError extends Error {
	readonly code: string;

	constructor(message: string, code: string) {
		super(message);
		this.name = "GuestRequestError";
		this.code = code;
	}
}

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
		const code = roomFailureResponseSchema.safeParse(value).data?.code ?? "";
		throw new GuestRequestError(roomFailureBoundarySchema.parse(value), code);
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
