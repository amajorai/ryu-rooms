import type { RyuNodeShareOrigin } from "@ryu/app-host/app-bridge";
import type { RoomSnapshot } from "./types.ts";
import { parseRoomSnapshot } from "./types.ts";

type AppMethod = "DELETE" | "GET" | "PATCH" | "POST" | "PUT";

async function requestApp<T>(
	path: string,
	method: AppMethod = "GET",
	body?: unknown
): Promise<T> {
	const request = window.ryu?.app?.request;
	if (!request) {
		throw new Error("The Rooms host bridge is unavailable.");
	}
	return (await request({ body, method, path })) as T;
}

function parseRequiredRoom(value: unknown): RoomSnapshot {
	const room = parseRoomSnapshot(value);
	if (!room) {
		throw new Error("The Rooms sidecar returned an invalid room snapshot.");
	}
	return room;
}

export async function listRooms(): Promise<RoomSnapshot[]> {
	const value = await requestApp<{ rooms?: unknown[] }>("/");
	if (!Array.isArray(value.rooms)) {
		return [];
	}
	return value.rooms
		.map(parseRoomSnapshot)
		.filter((room): room is RoomSnapshot => room !== null);
}

export async function createRoom(
	modelId: string,
	shareOrigin: string
): Promise<{ joinUrl: string; room: RoomSnapshot }> {
	const value = await requestApp<{ joinUrl?: unknown; room?: unknown }>(
		"/",
		"POST",
		{
			modelId,
			shareOrigin,
		}
	);
	if (typeof value.joinUrl !== "string") {
		throw new Error("The Rooms sidecar returned no invite link.");
	}
	return { joinUrl: value.joinUrl, room: parseRequiredRoom(value.room) };
}

export async function getRoom(roomId: string): Promise<RoomSnapshot> {
	return parseRequiredRoom(await requestApp(`/${encodeURIComponent(roomId)}`));
}

export async function submitTurn(
	roomId: string,
	text: string,
	idempotencyKey: string
): Promise<RoomSnapshot> {
	const value = await requestApp<{ snapshot?: unknown }>(
		`/${encodeURIComponent(roomId)}/turns`,
		"POST",
		{ idempotencyKey, text }
	);
	return parseRequiredRoom(value.snapshot);
}

export async function stopRun(
	roomId: string,
	runId: string
): Promise<RoomSnapshot> {
	const value = await requestApp<{ snapshot?: unknown }>(
		`/${encodeURIComponent(roomId)}/stop`,
		"POST",
		{ runId }
	);
	return parseRequiredRoom(value.snapshot);
}

export async function closeRoom(roomId: string): Promise<RoomSnapshot> {
	const value = await requestApp<{ snapshot?: unknown }>(
		`/${encodeURIComponent(roomId)}/close`,
		"POST"
	);
	return parseRequiredRoom(value.snapshot);
}

export async function revokeInvite(roomId: string): Promise<void> {
	await requestApp(`/${encodeURIComponent(roomId)}/invite/revoke`, "POST");
}

export async function issueInvite(
	roomId: string
): Promise<{ joinUrl: string }> {
	const value = await requestApp<{ joinUrl?: unknown }>(
		`/${encodeURIComponent(roomId)}/invite`,
		"POST"
	);
	if (typeof value.joinUrl !== "string") {
		throw new Error("The Rooms sidecar returned no invite link.");
	}
	return { joinUrl: value.joinUrl };
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
	const registryModels = window.ryu?.registry?.engineModels;
	if (registryModels) {
		try {
			const catalog = await registryModels();
			for (const model of catalog["mesh-llm"] ?? []) {
				if (typeof model.id === "string" && typeof model.name === "string") {
					models.push(model);
				}
			}
		} catch {
			// The UI leaves Start disabled when Mesh LLM discovery is unavailable.
		}
	}
	return models;
}
