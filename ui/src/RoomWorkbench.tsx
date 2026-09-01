import { RyuAppActions } from "@ryu/blocks/companion/app-ui";
import { InputBar } from "@ryu/blocks/desktop/agent-elements/input-bar";
import { Button } from "@ryu/ui/components/button.tsx";
import { ExpandableQRCode } from "@ryu/ui/components/qr-code.tsx";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
	closeRoom,
	getRoom,
	issueInvite,
	revokeInvite,
	stopRun,
	submitTurn,
} from "./api.ts";
import {
	isRoomRunning,
	parseRoomEvent,
	parseRoomSnapshot,
	type RoomSnapshot,
} from "./types.ts";

interface RoomWorkbenchProps {
	activeOrigin: string | null;
	initialJoinUrl: string | null;
	onRoomChanged(room: RoomSnapshot): void;
	onRoomClosed(): void;
	room: RoomSnapshot;
}

export function RoomWorkbench({
	activeOrigin,
	initialJoinUrl,
	onRoomChanged,
	onRoomClosed,
	room,
}: RoomWorkbenchProps) {
	const [joinUrl, setJoinUrl] = useState(initialJoinUrl);
	const [draft, setDraft] = useState("");
	const [partialText, setPartialText] = useState<string | null>(null);
	const [connection, setConnection] = useState<
		"connecting" | "connected" | "snapshot"
	>("connecting");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [copied, setCopied] = useState(false);

	useEffect(() => setJoinUrl(initialJoinUrl), [initialJoinUrl]);

	const refresh = useCallback(async () => {
		try {
			onRoomChanged(await getRoom(room.id));
			setError(null);
		} catch (cause) {
			setError(errorMessage(cause));
		}
	}, [onRoomChanged, room.id]);

	useEffect(() => {
		const realtime = window.ryu?.realtime;
		if (!realtime) {
			setConnection("snapshot");
			return;
		}
		let active = true;
		let current: { close(): Promise<void> } | null = null;
		void realtime
			.connect(
				{ roomId: room.id },
				{
					onClose: () => {
						if (active) {
							setConnection("snapshot");
						}
					},
					onError: () => {
						if (active) {
							setConnection("snapshot");
						}
					},
					onEvent: (incoming) => {
						const event = parseRoomEvent(incoming);
						if (!event?.known) {
							return;
						}
						const snapshot = parseRoomSnapshot(event.data.snapshot);
						if (snapshot) {
							onRoomChanged(snapshot);
							if (event.name !== "turn.delta") {
								setPartialText(null);
							}
						}
						if (event.name === "turn.delta") {
							const next = event.data.partialText;
							setPartialText(typeof next === "string" ? next : null);
						}
						if (
							[
								"turn.completed",
								"turn.failed",
								"turn.canceled",
								"room.closed",
							].includes(event.name)
						) {
							setPartialText(null);
						}
					},
					onResyncRequired: () => {
						void refresh();
					},
				}
			)
			.then((next) => {
				if (!active) {
					void next.close();
					return;
				}
				current = next;
				setConnection("connected");
			})
			.catch(() => {
				if (active) {
					setConnection("snapshot");
				}
			});
		return () => {
			active = false;
			if (current) {
				void current.close();
			}
		};
	}, [onRoomChanged, refresh, room.id]);

	const running = isRoomRunning(room) || busy;
	const onlineCount = useMemo(
		() => room.participants.filter((participant) => participant.online).length,
		[room.participants]
	);

	const send = async (message: { content: string; role: "user" }) => {
		const text = message.content.trim();
		if (!text || running) {
			return;
		}
		setBusy(true);
		setError(null);
		try {
			const next = await submitTurn(
				room.id,
				text,
				`host-${Date.now()}-${crypto.randomUUID?.() ?? Math.random()}`
			);
			onRoomChanged(next);
			setDraft("");
			setPartialText(null);
		} catch (cause) {
			setError(errorMessage(cause));
		} finally {
			setBusy(false);
		}
	};

	const stop = async () => {
		if (!room.currentRun) {
			return;
		}
		setBusy(true);
		try {
			onRoomChanged(await stopRun(room.id, room.currentRun.runId));
			setPartialText(null);
		} catch (cause) {
			setError(errorMessage(cause));
		} finally {
			setBusy(false);
		}
	};

	const close = async () => {
		if (
			!window.confirm("Close this room and revoke its current guest sessions?")
		) {
			return;
		}
		setBusy(true);
		try {
			onRoomChanged(await closeRoom(room.id));
			onRoomClosed();
		} catch (cause) {
			setError(errorMessage(cause));
		} finally {
			setBusy(false);
		}
	};

	const rotateInvite = async () => {
		setBusy(true);
		try {
			const next = await issueInvite(room.id);
			setJoinUrl(next.joinUrl);
		} catch (cause) {
			setError(errorMessage(cause));
		} finally {
			setBusy(false);
		}
	};

	const revoke = async () => {
		if (
			!window.confirm("Revoke the guest invite and existing guest sessions?")
		) {
			return;
		}
		setBusy(true);
		try {
			await revokeInvite(room.id);
			setJoinUrl(null);
		} catch (cause) {
			setError(errorMessage(cause));
		} finally {
			setBusy(false);
		}
	};

	const copy = async () => {
		if (!(joinUrl && navigator.clipboard)) {
			return;
		}
		await navigator.clipboard.writeText(joinUrl);
		setCopied(true);
		window.setTimeout(() => setCopied(false), 1600);
	};

	return (
		<section className="room-workbench">
			<header className="room-workbench-header">
				<div>
					<p className="rooms-kicker">Live room</p>
					<h2>{room.id.slice(0, 18)}</h2>
					<p className="rooms-muted">
						{activeOrigin ?? "Active node origin unavailable"} · {room.modelId}
					</p>
				</div>
				<div className="room-header-actions">
					<span
						className={`rooms-connection ${connection === "connected" ? "is-connected" : ""}`}
					>
						<span className="rooms-status-dot" />{" "}
						{connection === "connected"
							? "Live"
							: connection === "connecting"
								? "Connecting"
								: "Snapshot"}
					</span>
					<span className="rooms-status-pill">{room.status}</span>
				</div>
			</header>

			{error ? (
				<div className="rooms-alert" role="alert">
					{error}
				</div>
			) : null}

			<div className="room-workbench-grid">
				<section className="room-transcript-panel">
					<div className="room-panel-meta">
						<span>
							{onlineCount} online · {room.participants.length} participant
							{room.participants.length === 1 ? "" : "s"}
						</span>
						<span>{room.currentRun ? room.currentRun.status : "ready"}</span>
					</div>
					<div className="room-transcript">
						{room.messages.length === 0 && partialText === null ? (
							<div className="room-transcript-empty">
								Your room is ready for its first prompt.
							</div>
						) : null}
						{room.messages.map((message) => (
							<article
								className={`room-message ${message.role}`}
								key={message.id}
							>
								<div className="room-message-avatar">
									{message.role === "user" ? "You" : "AI"}
								</div>
								<div>
									<p className="room-message-label">
										{message.role === "user" ? "Prompt" : "Response"}
									</p>
									<div className="room-message-text">{message.text}</div>
								</div>
							</article>
						))}
						{partialText === null ? null : (
							<article className="room-message assistant live-message">
								<div className="room-message-avatar">AI</div>
								<div>
									<p className="room-message-label">Generating</p>
									<div className="room-message-text">
										{partialText}
										<span className="room-cursor" />
									</div>
								</div>
							</article>
						)}
					</div>
					<InputBar
						className="room-composer-inputbar"
						disabled={room.status === "closed"}
						onChange={setDraft}
						onSend={send}
						onStop={() => {
							if (room.currentRun) {
								void stop();
							}
						}}
						placeholder={running ? "The room is generating…" : "Ask the room…"}
						status={
							room.status === "closed"
								? "error"
								: running
									? "streaming"
									: "ready"
						}
						value={draft}
					/>
					<RyuAppActions className="room-run-actions">
						{room.currentRun && running ? (
							<Button
								disabled={busy}
								onClick={() => void stop()}
								size="sm"
								variant="outline"
							>
								Stop generation
							</Button>
						) : null}
						<Button
							disabled={busy || room.status === "closed"}
							onClick={() => void close()}
							size="sm"
							variant="ghost"
						>
							Close room
						</Button>
					</RyuAppActions>
				</section>

				<aside className="room-share-panel">
					<div>
						<p className="rooms-kicker">Invite a device</p>
						<h3>Scan to join</h3>
						<p className="rooms-muted">
							This phone sends prompts to the active Ryu node. It is not running
							the model.
						</p>
					</div>
					{joinUrl ? (
						<ExpandableQRCode
							aria-label="Room invite QR code"
							className="room-qr"
							containerClassName="room-qr-container"
							size={188}
							value={joinUrl}
						/>
					) : (
						<div className="room-qr-empty">
							No active invite.
							<br />
							Issue a new one below.
						</div>
					)}
					{joinUrl ? (
						<div className="room-join-url" title={joinUrl}>
							{joinUrl}
						</div>
					) : null}
					<RyuAppActions className="room-share-actions">
						<Button
							disabled={busy}
							onClick={() => void rotateInvite()}
							size="sm"
						>
							{joinUrl ? "New invite" : "Issue invite"}
						</Button>
						{joinUrl ? (
							<Button
								disabled={busy}
								onClick={() => void copy()}
								size="sm"
								variant="outline"
							>
								{copied ? "Copied" : "Copy link"}
							</Button>
						) : null}
						{joinUrl ? (
							<Button
								disabled={busy}
								onClick={() => void revoke()}
								size="sm"
								variant="ghost"
							>
								Revoke
							</Button>
						) : null}
					</RyuAppActions>
					<p className="room-share-footnote">
						Invite tokens live in the URL fragment and are exchanged for an
						HttpOnly session cookie. The node never puts a provider key in this
						room.
					</p>
				</aside>
			</div>
		</section>
	);
}

function errorMessage(cause: unknown): string {
	return cause instanceof Error ? cause.message : "The Rooms request failed.";
}
