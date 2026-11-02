import { expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { RoomWorkbench } from "./RoomWorkbench.tsx";

const room = {
	contract: "rooms/1" as const,
	currentRun: null,
	engine: "mesh-llm" as const,
	id: "room_7f3c9b2a4d5e6f70",
	messages: [],
	modelId: "qwen3-8b",
	participants: [
		{
			displayName: "Host",
			id: "member_1",
			online: true,
			role: "host" as const,
		},
	],
	schemaVersion: 1 as const,
	status: "idle" as const,
	updatedAt: "2026-08-30T08:00:00Z",
};

test("workbench renders truthful node and phone-compute copy", () => {
	const markup = renderToStaticMarkup(
		<RoomWorkbench
			activeOrigin="http://192.168.1.20:7980"
			initialJoinUrl="http://192.168.1.20:7980/api/rooms/guest#invite=opaque"
			onRoomChanged={() => undefined}
			onRoomClosed={() => undefined}
			room={room}
		/>
	);
	expect(markup).toContain("qwen3-8b");
	expect(markup).toContain(
		"This phone sends prompts to the active Ryu node. It is not running the model."
	);
	expect(markup).toContain("Scan to join");
});
