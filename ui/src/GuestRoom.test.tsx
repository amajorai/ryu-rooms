import { expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { GuestRoom } from "./GuestRoom.tsx";

test("guest view starts with a display-name form and no host controls", () => {
	const markup = renderToStaticMarkup(<GuestRoom invite="opaque" />);
	expect(markup).toContain("Join the conversation");
	expect(markup).toContain("Your name");
	expect(markup).not.toContain("Close room");
	expect(markup).not.toContain("nodeToken");
});
