import type {
	RyuAppBridge,
	RyuNodeShareOrigin,
} from "@ryu/app-host/app-bridge";

interface RyuRoomRealtimeConnection {
	access: "read" | "write";
	close(): Promise<void>;
	memberId: string;
	publish(name: string, data: unknown): Promise<void>;
	publishPresence(data: unknown): Promise<void>;
	roomId: string;
}

interface RyuRoomRealtime {
	connect(
		input: { roomId: string },
		handlers?: {
			onClose?: (event: { code: number; reason: string }) => void;
			onError?: (error: unknown) => void;
			onEvent?: (event: { data: unknown; name: string }) => void;
			onResyncRequired?: (notice: { dropped?: number; reason: string }) => void;
		}
	): Promise<RyuRoomRealtimeConnection>;
}

interface RyuRoomRegistry {
	engineModels(): Promise<Record<string, { id: string; name: string }[]>>;
}

interface RyuRoomBridge extends RyuAppBridge {
	app: {
		request(input: {
			body?: unknown;
			method?: "DELETE" | "GET" | "PATCH" | "POST" | "PUT";
			path: string;
		}): Promise<unknown>;
	};
	node?: {
		shareOrigins(): Promise<RyuNodeShareOrigin[]>;
	};
	realtime?: RyuRoomRealtime;
	registry?: RyuRoomRegistry;
}

declare global {
	interface Window {
		ryu?: RyuRoomBridge;
	}
}
