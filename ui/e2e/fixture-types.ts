import type { RyuRoomBridge } from "../src/ryu.d.ts";
import type { RoomsAppRequestBody } from "../src/schemas.ts";
import type { RoomSnapshot } from "../src/types.ts";
export type HostRequest = Parameters<RyuRoomBridge["app"]["request"]>[0];
export interface HostCall {
	body?: RoomsAppRequestBody;
	method: string;
	path: string;
}
export interface HostFixture {
	calls: HostCall[];
	inviteUrl: string;
	rooms: RoomSnapshot[];
}
export interface GuestRoomEvent {
	snapshot: RoomSnapshot;
}
declare global {
	interface Window {
		__roomCalls: HostCall[];
		__roomsCopiedText: string | null;
		__roomsEmitEvent(name: string, data: GuestRoomEvent): void;
		__roomsHostFixture: HostFixture;
	}
}
