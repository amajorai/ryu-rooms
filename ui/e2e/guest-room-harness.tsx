import { subscribeCompanionTheme } from "@ryu/app-host/companion-theme";
import { RyuAppShell } from "@ryu/blocks/companion/app-ui";
import { createRoot } from "react-dom/client";
import { GuestRoom } from "../src/GuestRoom.tsx";
import "../src/rooms.css";

const container = document.getElementById("guest-room-test-root");
if (!container) {
	throw new Error("Guest Room test harness root is missing.");
}

subscribeCompanionTheme();
createRoot(container).render(
	<RyuAppShell>
		<GuestRoom invite="rooms-companion-invite-token" />
	</RyuAppShell>
);
