#!/usr/bin/env node

/**
 * Write the catalog slice the Rooms guest invite page ships.
 *
 * The guest page is public and runs without the Ryu host, so instead of
 * importing `@ryu/i18n` it carries `src/guest/guest-catalog.generated.json`:
 * `{ <locale>: { <English source>: <translation> } }` for exactly the
 * `tl("…")` sentences in `src/guest/guest-page.js`, taken from
 * `packages/i18n/locales/<locale>.json` in the Ryu monorepo. Untranslated
 * sentences, and translations that drop or rename an ICU argument, are left
 * out and render in English.
 *
 *   node apps-store/rooms/ui/scripts/sync-guest-catalog.mjs           # write
 *   node apps-store/rooms/ui/scripts/sync-guest-catalog.mjs --check   # fail on drift
 */

import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { UI_ROOT } from "./guest-page.mjs";

export const SOURCE_FILE = join(UI_ROOT, "src/guest/guest-page.js");
export const OUTPUT_FILE = join(
	UI_ROOT,
	"src/guest/guest-catalog.generated.json"
);
export const CATALOG_DIR = resolve(UI_ROOT, "../../../packages/i18n/locales");
const NON_LOCALE_CATALOGS = new Set(["en", "untranslated"]);

const TL_CALL = /\btl\(\s*("(?:[^"\\]|\\.)*")\s*[,)]/gu;
const ANY_TL_CALL = /\btl\((?!\))/gu;

/** The English `tl("…")` sources of the page, in order. */
export function parseSources(source) {
	const literals = [...source.matchAll(TL_CALL)];
	const calls = (source.match(ANY_TL_CALL) ?? []).length;
	if (calls !== literals.length) {
		throw new Error(
			`guest-page.js: parsed ${literals.length} of ${calls} tl() calls; pass one double-quoted literal.`
		);
	}
	return [...new Set(literals.map(([, literal]) => JSON.parse(literal)))];
}

/** Sorted ICU argument names (`{name}`, `{count, plural, …}`) of a message. */
function argumentNames(text) {
	return [
		...new Set(
			[...text.matchAll(/\{\s*([A-Za-z_]\w*)\s*[,}]/gu)].map(
				(match) => match[1]
			)
		),
	].sort();
}

/** `{ locale: { source: translation } }`, keeping only usable translations. */
export function buildSlice(sentences, catalogsByLocale) {
	const slice = {};
	for (const [locale, catalog] of Object.entries(catalogsByLocale)) {
		const entries = {};
		for (const source of [...sentences].sort()) {
			const translation = catalog[source];
			if (
				typeof translation === "string" &&
				translation.trim() &&
				JSON.stringify(argumentNames(translation)) ===
					JSON.stringify(argumentNames(source))
			) {
				entries[source] = translation;
			}
		}
		if (Object.keys(entries).length > 0) {
			slice[locale] = entries;
		}
	}
	return slice;
}

export function expectedContent(catalogDir = CATALOG_DIR) {
	// `tl` is the page's translator; its own definition is not copy.
	const source = readFileSync(SOURCE_FILE, "utf8").replace(
		/const tl = \(source\) =>/u,
		""
	);
	const catalogsByLocale = {};
	for (const name of readdirSync(catalogDir).sort()) {
		const locale = name.replace(/\.json$/u, "");
		if (name.endsWith(".json") && !NON_LOCALE_CATALOGS.has(locale)) {
			catalogsByLocale[locale] = JSON.parse(
				readFileSync(join(catalogDir, name), "utf8")
			);
		}
	}
	return `${JSON.stringify(buildSlice(parseSources(source), catalogsByLocale), null, "\t")}\n`;
}

function main() {
	const content = expectedContent();
	let current = null;
	try {
		current = readFileSync(OUTPUT_FILE, "utf8");
	} catch {
		current = null;
	}
	if (current === content) {
		return;
	}
	if (process.argv.includes("--check")) {
		process.stderr.write(
			"Stale Rooms guest catalog (run node apps-store/rooms/ui/scripts/sync-guest-catalog.mjs).\n"
		);
		process.exit(1);
	}
	writeFileSync(OUTPUT_FILE, content);
	process.stdout.write(`wrote ${OUTPUT_FILE}\n`);
}

if (
	process.argv[1] &&
	resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
	main();
}
