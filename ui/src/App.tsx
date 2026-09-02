import {
	RyuAppEmpty,
	RyuAppField,
	RyuAppList,
	RyuAppListItem,
	RyuAppMain,
	RyuAppSection,
} from "@ryu/blocks/companion/app-ui";
import { Button } from "@ryu/ui/components/button.tsx";
import { Spinner } from "@ryu/ui/components/spinner.tsx";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
	createRoom,
	listMeshModels,
	listRooms,
	resolveShareOrigins,
} from "./api.ts";
import { RoomWorkbench } from "./RoomWorkbench.tsx";
import type { RoomSnapshot } from "./types.ts";

export function App() {
	const [rooms, setRooms] = useState<RoomSnapshot[]>([]);
	const [selectedId, setSelectedId] = useState<string | null>(null);
	const [joinLinks, setJoinLinks] = useState<Record<string, string>>({});
	const [origins, setOrigins] = useState<
		Awaited<ReturnType<typeof resolveShareOrigins>>
	>([]);
	const [models, setModels] = useState<{ id: string; name: string }[]>([]);
	const [selectedModel, setSelectedModel] = useState("");
	const [loading, setLoading] = useState(true);
	const [creating, setCreating] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const refresh = useCallback(async () => {
		setLoading(true);
		const [roomsResult, originsResult, modelsResult] = await Promise.allSettled(
			[listRooms(), resolveShareOrigins(), listMeshModels()]
		);
		if (roomsResult.status === "fulfilled") {
			setRooms(roomsResult.value);
			setSelectedId((current) =>
				current && roomsResult.value.some((room) => room.id === current)
					? current
					: null
			);
		} else {
			setError(errorMessage(roomsResult.reason));
		}
		if (originsResult.status === "fulfilled") {
			setOrigins(originsResult.value);
		}
		if (modelsResult.status === "fulfilled") {
			setModels(modelsResult.value);
			setSelectedModel((current) => current || modelsResult.value[0]?.id || "");
		}
		setLoading(false);
	}, []);

	useEffect(() => {
		void refresh();
	}, [refresh]);

	const selected = useMemo(
		() => rooms.find((room) => room.id === selectedId) ?? null,
		[rooms, selectedId]
	);
	const activeOrigin = origins[0]?.origin ?? null;
	const canStart = Boolean(activeOrigin && selectedModel && models.length > 0);

	const create = useCallback(async () => {
		if (!(activeOrigin && selectedModel)) {
			return;
		}
		setCreating(true);
		setError(null);
		try {
			const created = await createRoom(selectedModel, activeOrigin);
			setRooms((current) => [created.room, ...current]);
			setJoinLinks((current) => ({
				...current,
				[created.room.id]: created.joinUrl,
			}));
			setSelectedId(created.room.id);
		} catch (cause) {
			setError(errorMessage(cause));
		} finally {
			setCreating(false);
		}
	}, [activeOrigin, selectedModel]);

	const updateRoom = useCallback((next: RoomSnapshot) => {
		setRooms((current) =>
			current.some((room) => room.id === next.id)
				? current.map((room) => (room.id === next.id ? next : room))
				: [next, ...current]
		);
	}, []);

	return (
		<RyuAppMain className="rooms-shell">
			<header className="rooms-header">
				<div>
					<p className="rooms-kicker">Ryu / Rooms</p>
					<h1>Share a model session</h1>
					<p className="rooms-subtitle">
						One active node, one synchronized transcript, devices invited by QR.
					</p>
				</div>
				<div className="rooms-node-status">
					<span
						className={`rooms-status-dot ${activeOrigin ? "is-ready" : ""}`}
					/>
					<span>{activeOrigin ? "Shareable node" : "Loopback-only node"}</span>
				</div>
			</header>

			{error ? (
				<div className="rooms-alert" role="alert">
					{error}
				</div>
			) : null}

			{selected ? (
				<RoomWorkbench
					activeOrigin={activeOrigin}
					initialJoinUrl={joinLinks[selected.id] ?? null}
					onRoomChanged={updateRoom}
					onRoomClosed={() => {
						setSelectedId(null);
						void refresh();
					}}
					room={selected}
				/>
			) : (
				<>
					<RyuAppSection className="rooms-create-card">
						<div>
							<p className="rooms-kicker">New room</p>
							<h2>Use the Mesh LLM already active on this node.</h2>
							<p className="rooms-muted">
								The phone is a client. It does not download weights or run the
								model.
							</p>
						</div>
						<div className="rooms-create-grid">
							<RyuAppField className="rooms-field" label="Mesh LLM model">
								<select
									aria-label="Mesh LLM model"
									disabled={models.length === 0}
									onChange={(event) => setSelectedModel(event.target.value)}
									value={selectedModel}
								>
									{models.length === 0 ? (
										<option value="">No Mesh LLM models found</option>
									) : null}
									{models.map((model) => (
										<option key={model.id} value={model.id}>
											{model.name} · {model.id}
										</option>
									))}
								</select>
							</RyuAppField>
							<RyuAppField
								className="rooms-field rooms-origin-field"
								label="Invite origin"
							>
								<div className="rooms-origin-value">
									{activeOrigin ??
										"No LAN, mesh, or public origin is available"}
								</div>
							</RyuAppField>
							<Button
								disabled={!canStart || creating}
								onClick={() => void create()}
							>
								{creating ? "Starting…" : "Start room"}
							</Button>
						</div>
						{activeOrigin ? null : (
							<p className="rooms-note">
								Connect this node to a reachable LAN or private-network address
								before creating an invite.
							</p>
						)}
						{activeOrigin && models.length === 0 ? (
							<p className="rooms-note">
								Mesh LLM is not active or has not exposed a model catalog on
								this node.
							</p>
						) : null}
					</RyuAppSection>

					<RyuAppSection className="rooms-list-section">
						<div className="rooms-section-heading">
							<div>
								<p className="rooms-kicker">Active node</p>
								<h2>Your rooms</h2>
							</div>
							<Button onClick={() => void refresh()} size="sm" variant="ghost">
								Refresh
							</Button>
						</div>
						{loading ? (
							<div className="rooms-loading">
								<Spinner className="size-5" />
							</div>
						) : rooms.length === 0 ? (
							<RyuAppEmpty
								className="rooms-empty"
								description="Start a room when a reachable node and Mesh LLM model are ready."
								title="No rooms yet"
							/>
						) : (
							<RyuAppList aria-label="Your rooms" className="rooms-list">
								{rooms.map((room) => (
									<RyuAppListItem
										accessories={
											<span className="rooms-row-status">{room.status}</span>
										}
										className="rooms-list-row"
										icon={
											<span
												className={`rooms-status-dot ${room.status === "running" ? "is-running" : room.status === "failed" ? "is-failed" : ""}`}
											/>
										}
										key={room.id}
										onClick={() => setSelectedId(room.id)}
										selected={room.id === selectedId}
										subtitle={`${room.modelId} · ${room.participants.length} participant${room.participants.length === 1 ? "" : "s"}`}
										title={room.id.slice(0, 18)}
									/>
								))}
							</RyuAppList>
						)}
					</RyuAppSection>
				</>
			)}
		</RyuAppMain>
	);
}

function errorMessage(cause: unknown): string {
	return cause instanceof Error ? cause.message : "The Rooms request failed.";
}
