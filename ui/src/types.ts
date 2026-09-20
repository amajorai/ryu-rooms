export type RoomStatus = "idle" | "running" | "failed" | "closed";
export type RunStatus =
	| "queued"
	| "running"
	| "completed"
	| "failed"
	| "canceled";

export interface RoomMessage {
	createdAt: string;
	id: string;
	participantId: string | null;
	role: "user" | "assistant";
	text: string;
}

export interface RoomRun {
	errorCode: string | null;
	errorMessage: string | null;
	finishedAt: string | null;
	idempotencyKey: string | null;
	partialText: string;
	requestId: string;
	runId: string;
	startedAt: string | null;
	status: RunStatus;
}

export interface RoomParticipant {
	displayName: string;
	id: string;
	online: boolean;
	role: "host" | "guest" | "viewer";
}

export interface RoomSnapshot {
	contract: "rooms/1";
	currentRun: RoomRun | null;
	engine: "mesh-llm";
	id: string;
	messages: RoomMessage[];
	modelId: string;
	participants: RoomParticipant[];
	schemaVersion: 1;
	status: RoomStatus;
	updatedAt: string;
}

export interface RoomEvent {
	data: Record<string, unknown>;
	known: boolean;
	name: string;
}

const ROOM_STATUSES = new Set<RoomStatus>([
	"idle",
	"running",
	"failed",
	"closed",
]);
const RUN_STATUSES = new Set<RunStatus>([
	"queued",
	"running",
	"completed",
	"failed",
	"canceled",
]);
const EVENT_NAMES = new Set([
	"room.closed",
	"snapshot",
	"turn.accepted",
	"turn.canceled",
	"turn.completed",
	"turn.delta",
	"turn.failed",
]);

function record(value: unknown): Record<string, unknown> | null {
	return typeof value === "object" && value !== null && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: null;
}

function stringValue(value: unknown): string | null {
	return typeof value === "string" ? value : null;
}

function nullableString(value: unknown): string | null {
	return value === null || typeof value === "string" ? value : null;
}

function parseMessage(value: unknown): RoomMessage | null {
	const item = record(value);
	if (!item) {
		return null;
	}
	const role =
		item.role === "user" || item.role === "assistant" ? item.role : null;
	const id = stringValue(item.id);
	const text = stringValue(item.text);
	const createdAt = stringValue(item.createdAt);
	if (!(role && id) || text === null || !createdAt) {
		return null;
	}
	const participantId = nullableString(item.participantId);
	if (item.participantId !== null && typeof item.participantId !== "string") {
		return null;
	}
	return { createdAt, id, participantId, role, text };
}

function parseRun(value: unknown): RoomRun | null {
	if (value === null) {
		return null;
	}
	const item = record(value);
	if (!item) {
		return null;
	}
	const status = stringValue(item.status);
	const runId = stringValue(item.runId);
	const requestId = stringValue(item.requestId);
	const partialText = stringValue(item.partialText);
	if (
		!(runId && requestId) ||
		partialText === null ||
		!status ||
		!RUN_STATUSES.has(status as RunStatus)
	) {
		return null;
	}
	return {
		errorCode: nullableString(item.errorCode),
		errorMessage: nullableString(item.errorMessage),
		finishedAt: nullableString(item.finishedAt),
		idempotencyKey: nullableString(item.idempotencyKey),
		partialText,
		requestId,
		runId,
		startedAt: nullableString(item.startedAt),
		status: status as RunStatus,
	};
}

function parseParticipant(value: unknown): RoomParticipant | null {
	const item = record(value);
	if (!item) {
		return null;
	}
	const role = item.role;
	const id = stringValue(item.id);
	const displayName = stringValue(item.displayName);
	if (
		!(id && displayName) ||
		(role !== "host" && role !== "guest" && role !== "viewer") ||
		typeof item.online !== "boolean"
	) {
		return null;
	}
	return { displayName, id, online: item.online, role };
}

export function parseRoomSnapshot(value: unknown): RoomSnapshot | null {
	const item = record(value);
	if (item?.contract !== "rooms/1" || item?.schemaVersion !== 1) {
		return null;
	}
	const status = stringValue(item.status);
	const id = stringValue(item.id);
	const modelId = stringValue(item.modelId);
	const updatedAt = stringValue(item.updatedAt);
	if (
		!(id && modelId && updatedAt) ||
		item.engine !== "mesh-llm" ||
		!status ||
		!ROOM_STATUSES.has(status as RoomStatus) ||
		!Array.isArray(item.messages) ||
		!Array.isArray(item.participants)
	) {
		return null;
	}
	const messages = item.messages.map(parseMessage);
	const participants = item.participants.map(parseParticipant);
	if (
		messages.some((message) => !message) ||
		participants.some((participant) => !participant)
	) {
		return null;
	}
	const currentRun = parseRun(item.currentRun);
	if (item.currentRun !== null && !currentRun) {
		return null;
	}
	return {
		contract: "rooms/1",
		currentRun,
		engine: "mesh-llm",
		id,
		messages: messages as RoomMessage[],
		modelId,
		participants: participants as RoomParticipant[],
		schemaVersion: 1,
		status: status as RoomStatus,
		updatedAt,
	};
}

export function parseRoomEvent(value: unknown): RoomEvent | null {
	const item = record(value);
	const name = stringValue(item?.name);
	const data = record(item?.data);
	if (!(name && data)) {
		return null;
	}
	return { data, known: EVENT_NAMES.has(name), name };
}

export function isRoomRunning(room: RoomSnapshot): boolean {
	return room.status === "running" || room.currentRun?.status === "running";
}
