import { expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { GuestRoom } from "./GuestRoom.tsx";
import { GuestRequestError, submitGuestTurn } from "./guest-api.ts";

test("guest view starts with a display-name form and no host controls", () => {
	const markup = renderToStaticMarkup(<GuestRoom invite="opaque" />);
	expect(markup).toContain("Join the conversation");
	expect(markup).toContain("Your name");
	expect(markup).not.toContain("Close room");
	expect(markup).not.toContain("nodeToken");
});

test("guest requests keep the backend error code for localized copy", async () => {
	const realFetch = globalThis.fetch;
	globalThis.fetch = (async () =>
		Response.json(
			{ code: "roomBusy", message: "another turn is already running" },
			{ status: 409 }
		)) as unknown as typeof fetch;
	try {
		const failure = await submitGuestTurn("hi", "key").catch(
			(cause: unknown) => cause
		);
		expect(failure).toBeInstanceOf(GuestRequestError);
		expect((failure as GuestRequestError).code).toBe("roomBusy");
	} finally {
		globalThis.fetch = realFetch;
	}
});
