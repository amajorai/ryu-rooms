import type {
	RyuAppBridge,
	RyuNodeShareOrigin,
} from "@ryu/app-host/app-bridge";

import type { RealtimeAppApi } from "@ryu/app-host/realtime";
import type { RoomsAppRequestBody, RoomsAppResponse } from "./schemas.ts";

interface RyuRoomRegistry {
	engineModels(): Promise<Record<string, { id: string; name: string }[]>>;
}

export interface RyuRoomBridge extends RyuAppBridge {
	app: {
		request(input: {
			body?: RoomsAppRequestBody;
			method?: "DELETE" | "GET" | "PATCH" | "POST" | "PUT";
			path: string;
		}): Promise<RoomsAppResponse>;
	};
	node?: {
		shareOrigins(): Promise<RyuNodeShareOrigin[]>;
	};
	realtime?: RealtimeAppApi;
	registry?: RyuRoomRegistry;
}
