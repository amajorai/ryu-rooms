import type { RyuNodeShareOrigin } from "@ryu/app-host/app-bridge";
import type { z } from "zod";
import {
	meshModelCatalogBoundarySchema,
	type RoomsAppRequestBody,
	roomCreateBoundarySchema,
	roomHostBridgeSchema,
	roomInviteBoundarySchema,
	roomListResponseSchema,
	roomRequiredSnapshotSchema,
	roomRevokeResponseSchema,
	roomTurnBoundarySchema,
} from "./schemas.ts";
import type { RoomSnapshot } from "./types.ts";

type AppMethod = "DELETE" | "GET" | "PATCH" | "POST" | "PUT";

async function requestApp<T>(
	schema: z.ZodType<T>,
	path: string,
	method: AppMethod = "GET",
	body?: RoomsAppRequestBody
): Promise<T> {
	const request = roomHostBridgeSchema.parse(window.ryu).app?.request;
	if (!request) {
		throw new Error("The Rooms host bridge is unavailable.");
	}
	return schema.parse(await request({ body, method, path }));
}

export async function listRooms(): Promise<RoomSnapshot[]> {
	return (await requestApp(roomListResponseSchema, "/")).rooms;
}

export async function createRoom(
	modelId: string,
	shareOrigin: string
): Promise<{ joinUrl: string; room: RoomSnapshot }> {
	const value = await requestApp(roomCreateBoundarySchema, "/", "POST", {
		modelId,
		shareOrigin,
	});
	return value;
}

export async function getRoom(roomId: string): Promise<RoomSnapshot> {
	return requestApp(
		roomRequiredSnapshotSchema,
		`/${encodeURIComponent(roomId)}`
	);
}

export async function submitTurn(
	roomId: string,
	text: string,
	idempotencyKey: string
): Promise<RoomSnapshot> {
	const value = await requestApp(
		roomTurnBoundarySchema,
		`/${encodeURIComponent(roomId)}/turns`,
		"POST",
		{ idempotencyKey, text }
	);
	return value.snapshot;
}

export async function stopRun(
	roomId: string,
	runId: string
): Promise<RoomSnapshot> {
	const value = await requestApp(
		roomTurnBoundarySchema,
		`/${encodeURIComponent(roomId)}/stop`,
		"POST",
		{ runId }
	);
	return value.snapshot;
}

export async function closeRoom(roomId: string): Promise<RoomSnapshot> {
	const value = await requestApp(
		roomTurnBoundarySchema,
		`/${encodeURIComponent(roomId)}/close`,
		"POST"
	);
	return value.snapshot;
}

export async function revokeInvite(roomId: string): Promise<void> {
	await requestApp(
		roomRevokeResponseSchema,
		`/${encodeURIComponent(roomId)}/invite/revoke`,
		"POST"
	);
}

export async function issueInvite(
	roomId: string
): Promise<{ joinUrl: string }> {
	const value = await requestApp(
		roomInviteBoundarySchema,
		`/${encodeURIComponent(roomId)}/invite`,
		"POST"
	);
	return value;
}

export async function resolveShareOrigins(): Promise<RyuNodeShareOrigin[]> {
	const shareOrigins = window.ryu?.node?.shareOrigins;
	if (!shareOrigins) {
		return [];
	}
	return shareOrigins();
}

export async function listMeshModels(): Promise<
	{ id: string; name: string }[]
> {
	const models: Array<{ id: string; name: string }> = [];
	const registryModels = roomHostBridgeSchema.parse(window.ryu).registry
		?.engineModels;
	if (registryModels) {
		try {
			const catalog = await registryModels();
			models.push(...meshModelCatalogBoundarySchema.parse(catalog));
		} catch {
			// The UI leaves Start disabled when Mesh LLM discovery is unavailable.
		}
	}
	return models;
}
