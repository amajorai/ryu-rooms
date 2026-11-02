export const SOURCE_FILE: string;
export const OUTPUT_FILE: string;
export const CATALOG_DIR: string;
export function parseSources(source: string): string[];
export function buildSlice(
	sentences: readonly string[],
	catalogsByLocale: Readonly<Record<string, Readonly<Record<string, unknown>>>>
): Record<string, Record<string, string>>;
export function expectedContent(catalogDir?: string): string;
