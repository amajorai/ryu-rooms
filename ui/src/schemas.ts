import { jsonResponseDocumentSchema } from "@ryu/app-host/json-values";
import { z } from "zod";

export const roomStatusSchema = z.enum(["idle", "running", "failed", "closed"]);
export const runStatusSchema = z.enum([
	"queued",
	"running",
	"completed",
	"failed",
	"canceled",
]);
export const roomMessageSchema = z.object({
	createdAt: z.string().min(1),
	id: z.string().min(1),
	participantId: z.string().nullable(),
	role: z.enum(["user", "assistant"]),
	text: z.string(),
});
const nullableRunTextSchema = z.string().nullable().catch(null);
export const roomRunSchema = z.object({
	errorCode: nullableRunTextSchema,
	errorMessage: nullableRunTextSchema,
	finishedAt: nullableRunTextSchema,
	idempotencyKey: nullableRunTextSchema,
	partialText: z.string(),
	requestId: z.string().min(1),
	runId: z.string().min(1),
	startedAt: nullableRunTextSchema,
	status: runStatusSchema,
});
export const roomParticipantSchema = z.object({
	displayName: z.string().min(1),
	id: z.string().min(1),
	online: z.boolean(),
	role: z.enum(["host", "guest", "viewer"]),
});
export const roomSnapshotSchema = z.object({
	contract: z.literal("rooms/1"),
	currentRun: roomRunSchema.nullable(),
	engine: z.literal("mesh-llm"),
	id: z.string().min(1),
	messages: z.array(roomMessageSchema),
	modelId: z.string().min(1),
	participants: z.array(roomParticipantSchema),
	schemaVersion: z.literal(1),
	status: roomStatusSchema,
	updatedAt: z.string().min(1),
});
export const roomSnapshotBoundarySchema = z.unknown().transform((value) => {
	const parsed = roomSnapshotSchema.safeParse(value);
	return parsed.success ? parsed.data : null;
});
const EVENT_NAMES = new Set([
	"room.closed",
	"snapshot",
	"turn.accepted",
	"turn.canceled",
	"turn.completed",
	"turn.delta",
	"turn.failed",
]);
export const roomEventBoundarySchema = z.unknown().transform((value) => {
	const parsed = z
		.object({ data: jsonResponseDocumentSchema, name: z.string().min(1) })
		.safeParse(value);
	return parsed.success
		? {
				data: parsed.data.data,
				known: EVENT_NAMES.has(parsed.data.name),
				name: parsed.data.name,
			}
		: null;
});

/** Actual Rust endpoint envelopes; optional auxiliary fields retain older clients. */
export const roomCreateRequestSchema = z.object({
	modelId: z.string(),
	shareOrigin: z.string(),
});
export const roomTurnRequestSchema = z.object({
	idempotencyKey: z.string(),
	text: z.string(),
});
export const roomStopRequestSchema = z.object({ runId: z.string() });
export const guestExchangeRequestSchema = z.object({
	displayName: z.string(),
	invite: z.string(),
});
export const roomsAppRequestBodySchema = z.union([
	roomCreateRequestSchema,
	roomTurnRequestSchema,
	roomStopRequestSchema,
]);
export type RoomsAppRequestBody = z.output<typeof roomsAppRequestBodySchema>;
export const roomListResponseSchema = z
	.object({ rooms: z.array(roomSnapshotBoundarySchema).catch([]) })
	.transform((value) => ({
		rooms: value.rooms.filter((room) => room !== null),
	}));
export const roomCreateResponseSchema = z.object({
	joinUrl: z.string(),
	room: roomSnapshotSchema,
});
export const roomTurnResponseSchema = z.object({
	snapshot: roomSnapshotSchema,
	requestId: z.string().optional(),
	run: roomRunSchema.optional(),
	message: roomMessageSchema.optional(),
});
export const roomInviteResponseSchema = z.object({
	joinUrl: z.string(),
	requestId: z.string().optional(),
});
export const roomRevokeResponseSchema = z.object({
	revoked: z.boolean().optional(),
	requestId: z.string().optional(),
});
export const guestExchangeResponseSchema = z.object({
	room: roomSnapshotSchema,
	participantId: z.string().optional(),
	displayName: z.string().optional(),
	role: z.enum(["host", "guest", "viewer"]).optional(),
	requestId: z.string().optional(),
});
export const roomFailureResponseSchema = z.object({
	code: z.string().optional(),
	message: z.string().catch("The room request was not accepted."),
	requestId: z.string().optional(),
});
export const roomsAppResponseSchema = z.union([
	roomSnapshotSchema,
	roomListResponseSchema,
	roomCreateResponseSchema,
	roomTurnResponseSchema,
	roomInviteResponseSchema,
	roomRevokeResponseSchema,
]);
export type RoomsAppResponse = z.output<typeof roomsAppResponseSchema>;
export const roomRequiredSnapshotSchema = roomSnapshotBoundarySchema.transform(
	(room) => {
		if (!room) {
			throw new Error("The Rooms sidecar returned an invalid room snapshot.");
		}
		return room;
	}
);
// Preserve the original errors' field-check order (invite link before room).
export const roomCreateBoundarySchema = z.unknown().transform((value) => {
	const reply = z
		.object({ joinUrl: z.string(), room: roomSnapshotBoundarySchema })
		.safeParse(value);
	const invite = z.object({ joinUrl: z.string() }).safeParse(value);
	if (!invite.success) {
		throw new Error("The Rooms sidecar returned no invite link.");
	}
	if (!(reply.success && reply.data.room)) {
		throw new Error("The Rooms sidecar returned an invalid room snapshot.");
	}
	return { joinUrl: reply.data.joinUrl, room: reply.data.room };
});
export const roomInviteBoundarySchema = z.unknown().transform((value) => {
	const reply = roomInviteResponseSchema.safeParse(value);
	if (!reply.success) {
		throw new Error("The Rooms sidecar returned no invite link.");
	}
	return { joinUrl: reply.data.joinUrl };
});
export const roomTurnBoundarySchema = z.object({
	snapshot: roomRequiredSnapshotSchema,
	requestId: z.string().optional(),
	run: roomRunSchema.optional(),
	message: roomMessageSchema.optional(),
});
const meshModelFieldsSchema = z.object({ id: z.string(), name: z.string() });
export const meshModelSchema = z.custom<z.output<typeof meshModelFieldsSchema>>(
	(value) => meshModelFieldsSchema.safeParse(value).success
);
export const meshModelCatalogBoundarySchema = z.unknown().transform((value) => {
	const catalog = z
		.object({ "mesh-llm": z.array(z.unknown()).catch([]) })
		.safeParse(value);
	return catalog.success
		? catalog.data["mesh-llm"].flatMap((value) => {
				const model = meshModelSchema.safeParse(value);
				return model.success ? [model.data] : [];
			})
		: [];
});
export const roomPartialTextSchema = z.string().nullable().catch(null);
export const roomRequestFailureSchema = z
	.instanceof(Error)
	.catch(new Error("The Rooms request failed."))
	.transform((error) => error.message);

export const guestExchangeBoundarySchema = z.unknown().transform((value) => {
	const reply = guestExchangeResponseSchema.safeParse(value);
	if (!reply.success) {
		throw new Error("The invite response was not a valid room.");
	}
	return reply.data.room;
});
export const guestTurnBoundarySchema = z.unknown().transform((value) => {
	const reply = roomTurnResponseSchema.safeParse(value);
	if (!reply.success) {
		throw new Error("The turn response was not a valid room.");
	}
	return reply.data.snapshot;
});
export const roomFailureBoundarySchema = z.unknown().transform((value) => {
	const reply = roomFailureResponseSchema.safeParse(value);
	return reply.success
		? reply.data.message
		: "The room request was not accepted.";
});
export const guestRoomEventSchema = z.string().transform((serialized) => {
	const value = jsonResponseDocumentSchema.parse(JSON.parse(serialized));
	return {
		snapshot: roomSnapshotBoundarySchema.parse(value.snapshot ?? value),
		partialText: roomPartialTextSchema.parse(value.partialText),
		clearPartial: !value.partialText,
	};
});

const roomAppRequestCallableSchema = z.function({
	input: [
		z.object({
			path: z.string(),
			method: z.enum(["DELETE", "GET", "PATCH", "POST", "PUT"]).optional(),
			body: roomsAppRequestBodySchema.optional(),
		}),
	],
	output: z.promise(roomsAppResponseSchema),
});
const roomRegistryCallableSchema = z.function({
	input: [],
	output: z.promise(
		z.record(
			z.string(),
			z.array(z.object({ id: z.string(), name: z.string() }))
		)
	),
});
export const roomHostBridgeSchema = z
	.object({
		app: z
			.object({
				request: z.custom<z.output<typeof roomAppRequestCallableSchema>>(
					(value) => roomAppRequestCallableSchema.safeParse(value).success
				),
			})
			.optional(),
		registry: z
			.object({
				engineModels: z.custom<z.output<typeof roomRegistryCallableSchema>>(
					(value) => roomRegistryCallableSchema.safeParse(value).success
				),
			})
			.optional(),
	})
	.catch({});
