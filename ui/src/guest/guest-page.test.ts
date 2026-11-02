import { afterAll, describe, expect, mock, test } from "bun:test";
import { existsSync } from "node:fs";
import { GlobalRegistrator } from "@happy-dom/global-registrator";
import catalogs from "./guest-catalog.generated.json" with { type: "json" };

if (!GlobalRegistrator.isRegistered) {
	GlobalRegistrator.register({ url: "http://node.example/api/rooms/guest" });
}

const { assembleGuestPage } = await import("../../scripts/guest-page.mjs");
const { CATALOG_DIR, expectedContent } = await import(
	"../../scripts/sync-guest-catalog.mjs"
);

const ar: Record<string, string> = catalogs.ar;
/** The Arabic page text for `source` (English until the catalog has it). */
const arabic = (source: string) => ar[source] ?? source;
const page = new DOMParser().parseFromString(assembleGuestPage(), "text/html");
document.body.innerHTML = page.body.innerHTML;
Object.defineProperty(navigator, "languages", {
	configurable: true,
	value: ["ar-EG", "en-US"],
});
const fetchMock = mock(async () =>
	Response.json(
		{ code: "inviteUnavailable", message: "the invite is invalid" },
		{ status: 400 }
	)
);
globalThis.fetch = fetchMock as unknown as typeof fetch;
location.hash = "#invite=opaque";
// The page is a plain script; importing it runs it against this document.
await import("./guest-page.js");

afterAll(async () => {
	await GlobalRegistrator.unregister();
});

describe("guest invite page", () => {
	test("follows navigator.languages: Arabic renders right to left", () => {
		expect(document.documentElement.lang).toBe("ar");
		expect(document.documentElement.dir).toBe("rtl");
		expect(document.querySelector("#join-button")?.textContent).toBe(
			arabic("Join room")
		);
		expect(document.querySelector("h1")?.textContent).toBe(
			ar["Join the conversation."]
		);
		expect(document.querySelector<HTMLInputElement>("#name")?.placeholder).toBe(
			ar["e.g. Jiawei…"]
		);
	});

	test("sentences without a translation fall back to English", () => {
		expect(document.querySelector("#prompt")?.getAttribute("aria-label")).toBe(
			arabic("Message to room")
		);
	});

	test("maps backend error codes to page copy instead of English causes", async () => {
		document.querySelector<HTMLInputElement>("#name")!.value = "Guest";
		document
			.querySelector("#join-form")
			?.dispatchEvent(new Event("submit", { cancelable: true }));
		await new Promise((resolve) => setTimeout(resolve, 0));
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(fetchMock).toHaveBeenCalledTimes(1);
		const shown = document.querySelector("#join-error")?.textContent;
		expect(shown).toBe(arabic("This invite is invalid, expired, or revoked."));
		expect(shown).not.toContain("the invite is invalid");
	});

	// The official catalogs exist only in the Ryu monorepo, not the satellite mirror.
	test.if(existsSync(CATALOG_DIR))(
		"the committed catalog slice matches the official catalogs",
		() => {
			expect(`${JSON.stringify(catalogs, null, "\t")}\n`).toBe(
				expectedContent()
			);
		}
	);
});
