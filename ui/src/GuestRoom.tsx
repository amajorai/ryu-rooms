import {
	RyuAppActions,
	RyuAppField,
	RyuAppForm,
	RyuAppSection,
} from "@ryu/blocks/companion/app-ui";
import { Input } from "@ryu/blocks/companion/controls";
import { InputBar } from "@ryu/blocks/desktop/agent-elements/input-bar";
import { Button } from "@ryu/ui/components/button.tsx";
import { useEffect, useState } from "react";
import { exchangeInvite, submitGuestTurn } from "./guest-api.ts";
import { parseRoomSnapshot, type RoomSnapshot } from "./types.ts";

/** React guest view kept alongside the standalone guest carriage for hosts that
 * choose to mount the guest experience in their own web shell. */
export function GuestRoom({ invite }: { invite: string }) {
	const [name, setName] = useState("");
	const [room, setRoom] = useState<RoomSnapshot | null>(null);
	const [draft, setDraft] = useState("");
	const [partial, setPartial] = useState<string | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [joining, setJoining] = useState(false);

	useEffect(() => {
		if (!room) {
			return;
		}
		const source = new EventSource("/api/rooms/guest/events");
		const apply = (event: MessageEvent<string>) => {
			const value = JSON.parse(event.data) as Record<string, unknown>;
			const snapshot = parseRoomSnapshot(value.snapshot ?? value);
			if (snapshot) {
				setRoom(snapshot);
				if (!value.partialText) {
					setPartial(null);
				}
			}
			if (typeof value.partialText === "string") {
				setPartial(value.partialText);
			}
		};
		source.addEventListener("snapshot", apply);
		source.addEventListener("turn.accepted", apply);
		source.addEventListener("turn.delta", apply);
		source.addEventListener("turn.completed", apply);
		source.addEventListener("turn.failed", apply);
		source.addEventListener("turn.canceled", apply);
		return () => source.close();
	}, [room]);

	if (!room) {
		return (
			<RyuAppForm
				className="rooms-guest-card"
				onSubmit={async (event) => {
					event.preventDefault();
					setJoining(true);
					setError(null);
					try {
						setRoom(await exchangeInvite(invite, name));
					} catch (cause) {
						setError(
							cause instanceof Error
								? cause.message
								: "The invite could not be used."
						);
					} finally {
						setJoining(false);
					}
				}}
			>
				<p className="rooms-kicker">Shared model room</p>
				<h1>Join the conversation.</h1>
				<RyuAppField label="Your name">
					<Input
						aria-label="Your name"
						maxLength={80}
						name="displayName"
						onChange={(event) => setName(event.target.value)}
						placeholder="e.g. Jiawei…"
						required
						spellCheck={false}
						type="text"
						value={name}
					/>
				</RyuAppField>
				<RyuAppActions>
					<Button disabled={joining} type="submit">
						{joining ? "Joining…" : "Join room"}
					</Button>
				</RyuAppActions>
				{error ? (
					<p className="rooms-alert" role="alert">
						{error}
					</p>
				) : null}
			</RyuAppForm>
		);
	}

	return (
		<RyuAppSection className="rooms-guest-card">
			<p className="rooms-kicker">{room.modelId}</p>
			<h1>Room conversation</h1>
			<div className="room-transcript">
				{room.messages.map((message) => (
					<article className={`room-message ${message.role}`} key={message.id}>
						<div className="room-message-avatar">
							{message.role === "user" ? "You" : "AI"}
						</div>
						<div>
							<p className="room-message-label">{message.role}</p>
							<div className="room-message-text">{message.text}</div>
						</div>
					</article>
				))}
				{partial ? (
					<article className="room-message assistant">
						<div className="room-message-avatar">AI</div>
						<div className="room-message-text">{partial}</div>
					</article>
				) : null}
			</div>
			<InputBar
				className="room-composer-inputbar"
				disabled={room.status === "closed"}
				onChange={setDraft}
				onSend={async ({ content }) => {
					const text = content.trim();
					if (!text || room.status === "running") {
						return;
					}
					try {
						setRoom(await submitGuestTurn(text, `guest-${Date.now()}`));
						setDraft("");
					} catch (cause) {
						setError(
							cause instanceof Error
								? cause.message
								: "The prompt was not accepted."
						);
					}
				}}
				onStop={() => undefined}
				placeholder="Ask the room…"
				status={room.status === "running" ? "streaming" : "ready"}
				value={draft}
			/>
			{error ? (
				<p className="rooms-alert" role="alert">
					{error}
				</p>
			) : null}
		</RyuAppSection>
	);
}
