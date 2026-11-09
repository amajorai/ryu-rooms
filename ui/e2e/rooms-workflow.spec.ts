import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";
import type { RyuRoomBridge } from "../src/ryu.d.ts";
import type { RoomsAppResponse } from "../src/schemas.ts";
import type { RoomSnapshot } from "../src/types.ts";
import type { HostCall, HostRequest } from "./fixture-types.ts";

const shareOrigin = "http://192.168.1.20:7980";
const roomId = "room_7f3c9b2a4d5e6f70";
const evidenceDirectory = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"evidence"
);

test("selects a model, rotates the invite, confirms revocation, and closes a room", async ({
	page,
}) => {
	const pageErrors: string[] = [];
	page.on("pageerror", (error) => pageErrors.push(error.message));
	await page.addInitScript(
		({ origin, id }: { origin: string; id: string }) => {
			const room: RoomSnapshot = {
				contract: "rooms/1",
				schemaVersion: 1,
				id,
				status: "idle",
				modelId: "llama3-8b",
				engine: "mesh-llm",
				messages: [],
				currentRun: null,
				participants: [
					{ id: "member_1", role: "host", displayName: "Host", online: true },
				],
				updatedAt: "2026-09-28T10:00:00.000Z",
			};
			const closedRoom: RoomSnapshot = { ...room, status: "closed" };
			const calls: HostCall[] = [];
			let rooms: RoomSnapshot[] = [];
			let inviteUrl = `${origin}/api/rooms/guest#invite=first-token`;
			window.__roomCalls = calls;
			const ryu: RyuRoomBridge = {
				app: {
					request: async ({
						path,
						method = "GET",
						body,
					}: HostRequest): Promise<RoomsAppResponse> => {
						calls.push({ body, method, path });
						if (method === "GET" && path === "/") {
							return { rooms };
						}
						if (method === "POST" && path === "/") {
							rooms = [room];
							return { joinUrl: inviteUrl, room };
						}
						if (method === "POST" && path === `/${id}/invite`) {
							inviteUrl = `${origin}/api/rooms/guest#invite=rotated-token`;
							return { joinUrl: inviteUrl };
						}
						if (method === "POST" && path === `/${id}/invite/revoke`) {
							return {};
						}
						if (method === "POST" && path === `/${id}/close`) {
							rooms = [];
							return { snapshot: closedRoom };
						}
						throw new Error(`Unexpected local Rooms request ${method} ${path}`);
					},
				},
				node: {
					shareOrigins: async () => [
						{ origin, source: "active", reachable: true },
					],
				},
				registry: {
					engineModels: async () => ({
						"mesh-llm": [
							{ id: "qwen3-8b", name: "Qwen 3 8B" },
							{ id: "llama3-8b", name: "Llama 3 8B" },
						],
					}),
				},
			};
			Object.defineProperty(window, "ryu", {
				configurable: true,
				writable: true,
				value: ryu,
			});
		},
		{ origin: shareOrigin, id: roomId }
	);

	await page.goto("/");
	await expect(page).toHaveTitle("Rooms");
	await expect(
		page.getByRole("heading", { name: "Share a model session" })
	).toBeVisible();
	await page.getByLabel("Mesh LLM model").selectOption("llama3-8b");
	await page.getByRole("button", { name: "Start room" }).click();
	await expect(
		page.getByText("Your room is ready for its first prompt.")
	).toBeVisible();
	await expect(
		page.getByText(`${shareOrigin}/api/rooms/guest#invite=first-token`)
	).toBeVisible();
	await page.getByRole("button", { name: "New invite" }).click();
	await expect(
		page.getByText(`${shareOrigin}/api/rooms/guest#invite=rotated-token`)
	).toBeVisible();

	await page.getByRole("button", { name: "Revoke", exact: true }).click();
	const revokeDialog = page.getByRole("alertdialog");
	await expect(revokeDialog).toContainText("Revoke guest access?");
	await mkdir(evidenceDirectory, { recursive: true });
	await page.screenshot({
		animations: "disabled",
		fullPage: true,
		path: path.resolve(evidenceDirectory, "rooms-revoke-confirmation.png"),
	});
	await revokeDialog.getByRole("button", { name: "Revoke access" }).click();
	await expect(page.getByText("No active invite.")).toBeVisible();

	await page.getByRole("button", { name: "Close room", exact: true }).click();
	const closeDialog = page.getByRole("alertdialog");
	await expect(closeDialog).toContainText("Close this room?");
	await page.screenshot({
		animations: "disabled",
		fullPage: true,
		path: path.resolve(evidenceDirectory, "rooms-close-confirmation.png"),
	});
	await closeDialog.getByRole("button", { name: "Close room" }).click();
	await expect(page.getByText("No rooms yet")).toBeVisible();

	const calls = await page.evaluate(() => window.__roomCalls);
	expect(calls).toContainEqual({
		body: { modelId: "llama3-8b", shareOrigin },
		method: "POST",
		path: "/",
	});
	expect(calls).toContainEqual({
		method: "POST",
		path: `/${roomId}/invite/revoke`,
	});
	expect(calls).toContainEqual({ method: "POST", path: `/${roomId}/close` });
	expect(pageErrors).toEqual([]);
});
