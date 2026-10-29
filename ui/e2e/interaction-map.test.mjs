import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import path from "node:path";
import {
	namedAssertionIsPresent,
	reconcileLocalInteractionMap,
} from "../../../../apps-store/companion-suite.mjs";

const e2eDirectory = import.meta.dirname;
const repositoryRoot = path.resolve(e2eDirectory, "../../../../");
const censusPath = path.join(
	repositoryRoot,
	"docs/proof/test-coverage-sweep-20260924/companion-interaction-inventory.tsv"
);
const map = JSON.parse(
	readFileSync(path.join(e2eDirectory, "companion-interactions.json"), "utf8")
);
const spec = readFileSync(
	path.join(e2eDirectory, "rooms-companion.spec.ts"),
	"utf8"
);

function parseTsv(source) {
	const [headerLine, ...lines] = source.trimEnd().split("\n");
	const headers = headerLine?.split("\t") ?? [];
	return lines.map((line) => {
		const values = line.split("\t");
		return Object.fromEntries(
			headers.map((header, index) => [header, values[index] ?? ""])
		);
	});
}

const censusRows = parseTsv(readFileSync(censusPath, "utf8"));

describe("Rooms package-owned Companion interaction map", () => {
	test("joins each of the 10 exact uncovered controls to a named browser assertion", () => {
		expect(map.scope).toBe("partial");
		expect(map.expectedRows).toBe(10);
		const reconciled = reconcileLocalInteractionMap("rooms", censusRows, map);
		expect(reconciled).toHaveLength(10);
		for (const { assertion } of reconciled) {
			expect(
				namedAssertionIsPresent(
					spec,
					assertion.testTitle,
					assertion.assertionMarker
				)
			).toBe(true);
		}
	});

	test("rejects stale and duplicated interaction identities", () => {
		const first = map.assertions[0];
		expect(first).toBeDefined();
		expect(() =>
			reconcileLocalInteractionMap("rooms", censusRows, {
				...map,
				assertions: [first, { ...first }],
				expectedRows: 2,
			})
		).toThrow("duplicates an interaction identity");
		const stale = { ...first, row: { ...first.row, line: "99999" } };
		expect(() =>
			reconcileLocalInteractionMap("rooms", censusRows, {
				...map,
				assertions: [stale],
				expectedRows: 1,
			})
		).toThrow("does not match exactly one current central census row");
	});
});
