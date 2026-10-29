import type { z } from "zod";
import type {
	roomMessageSchema,
	roomParticipantSchema,
	roomRunSchema,
	roomSnapshotSchema,
	roomStatusSchema,
	runStatusSchema,
} from "./schemas.ts";
import {
	roomEventBoundarySchema,
	roomSnapshotBoundarySchema,
} from "./schemas.ts";
export type RoomStatus = z.output<typeof roomStatusSchema>;
export type RunStatus = z.output<typeof runStatusSchema>;
export type RoomMessage = z.output<typeof roomMessageSchema>;
export type RoomRun = z.output<typeof roomRunSchema>;
export type RoomParticipant = z.output<typeof roomParticipantSchema>;
export type RoomSnapshot = z.output<typeof roomSnapshotSchema>;
export type RoomEvent = NonNullable<z.output<typeof roomEventBoundarySchema>>;

export function parseRoomSnapshot(
	value: z.input<typeof roomSnapshotBoundarySchema>
): RoomSnapshot | null {
	return roomSnapshotBoundarySchema.parse(value);
}
export function parseRoomEvent(
	value: z.input<typeof roomEventBoundarySchema>
): RoomEvent | null {
	return roomEventBoundarySchema.parse(value);
}
export function isRoomRunning(room: RoomSnapshot): boolean {
	return room.status === "running" || room.currentRun?.status === "running";
}
