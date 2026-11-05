// Assemble the standalone Rooms guest page: `guest.html` with the catalog
// slice and `src/guest/guest-page.js` inlined. The Rooms backend does the same
// exact replacements in `backend/src/guest_html.rs`; keep the two in step.

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const UI_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const CATALOG_TAG =
	'<script type="application/json" id="rooms-guest-catalog"></script>';
export const SCRIPT_TAG = '<script src="src/guest/guest-page.js"></script>';

/** Replace the one `marker` in `text`, failing when it is missing. */
function replaceOnce(text, marker, replacement) {
	const index = text.indexOf(marker);
	if (index < 0) {
		throw new Error(`guest.html is missing ${marker}`);
	}
	return text.slice(0, index) + replacement + text.slice(index + marker.length);
}

/** JSON safe inside a `<script>` element (`<` cannot open a closing tag). */
export function scriptSafeJson(json) {
	return json.trim().replaceAll("<", "\\u003c");
}

/** The served guest page. */
export function assembleGuestPage(root = UI_ROOT) {
	const template = readFileSync(resolve(root, "guest.html"), "utf8");
	const catalog = readFileSync(
		resolve(root, "src/guest/guest-catalog.generated.json"),
		"utf8"
	);
	const script = readFileSync(resolve(root, "src/guest/guest-page.js"), "utf8");
	if (/<\/script|<!--/iu.test(script)) {
		throw new Error(
			"guest-page.js cannot be inlined: it contains </script or <!--"
		);
	}
	const withCatalog = replaceOnce(
		template,
		CATALOG_TAG,
		`<script type="application/json" id="rooms-guest-catalog">${scriptSafeJson(catalog)}</script>`
	);
	return replaceOnce(withCatalog, SCRIPT_TAG, `<script>\n${script}</script>`);
}
