import {
	RyuAppActions,
	RyuAppField,
	RyuAppForm,
	RyuAppSection,
} from "@ryu/blocks/companion/app-ui";
import { Input } from "@ryu/blocks/companion/controls";
import { InputBar } from "@ryu/blocks/desktop/agent-elements/input-bar";
import { useLiteralMessages } from "@ryu/i18n/react";
import { Button } from "@ryu/ui/components/button.tsx";
import { useEffect, useState } from "react";
import {
	exchangeInvite,
	GuestRequestError,
	submitGuestTurn,
} from "./guest-api.ts";
import { guestRoomEventSchema } from "./schemas.ts";
import type { RoomSnapshot } from "./types.ts";

/** React guest view kept alongside the standalone guest carriage for hosts that
 * choose to mount the guest experience in their own web shell. */
export function GuestRoom({ invite }: { invite: string }) {
	const tl = useLiteralMessages();
	/** Page copy for the backend's stable error `code`, never its English message. */
	const failureText = (cause: unknown, fallback: string): string => {
		const code = cause instanceof GuestRequestError ? cause.code : "";
		switch (code) {
			case "conflict":
				return tl("The room changed while you were sending. Try again.");
			case "forbidden":
				return tl("You do not have access to this room.");
			case "inviteUnavailable":
				return tl("This invite is invalid, expired, or revoked.");
			case "invalidArgs":
				return tl("Check what you entered and try again.");
			case "roomBusy":
				return tl("Another prompt is already running. Wait for it to finish.");
			case "roomClosed":
				return tl("This room is closed.");
			case "roomNotFound":
				return tl("This room no longer exists.");
			case "storeUnavailable":
				return tl("The room is temporarily unavailable. Try again.");
			case "unauthorized":
				return tl("Your guest session has ended. Open the invite link again.");
			default:
				return fallback;
		}
	};
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
			const value = guestRoomEventSchema.parse(event.data);
			const snapshot = value.snapshot;
			if (snapshot) {
				setRoom(snapshot);
				if (value.clearPartial) {
					setPartial(null);
				}
			}
			if (value.partialText !== null) {
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
						setError(failureText(cause, tl("The invite could not be used.")));
					} finally {
						setJoining(false);
					}
				}}
			>
				<h1>{tl("Join the conversation.")}</h1>
				<RyuAppField label={tl("Your name")}>
					<Input
						aria-label={tl("Your name")}
						maxLength={80}
						name="displayName"
						onChange={(event) => setName(event.target.value)}
						placeholder={tl("e.g. Jiawei…")}
						required
						spellCheck={false}
						type="text"
						value={name}
					/>
				</RyuAppField>
				<RyuAppActions>
					<Button disabled={joining} type="submit">
						{joining ? tl("Joining…") : tl("Join room")}
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
			<h1>{tl("Room conversation")}</h1>
			<p className="rooms-muted">{room.modelId}</p>
			<div className="room-transcript">
				{room.messages.map((message) => (
					<article className={`room-message ${message.role}`} key={message.id}>
						<div className="room-message-avatar">
							{message.role === "user" ? tl("You") : tl("AI")}
						</div>
						<div>
							<p className="room-message-label">
								{message.role === "user" ? tl("Prompt") : tl("Response")}
							</p>
							<div className="room-message-text" dir="auto">
								{message.text}
							</div>
						</div>
					</article>
				))}
				{partial ? (
					<article className="room-message">
						<div className="room-message-avatar">{tl("AI")}</div>
						<div className="room-message-text" dir="auto">
							{partial}
						</div>
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
						setError(failureText(cause, tl("The prompt was not accepted.")));
					}
				}}
				onStop={() => undefined}
				placeholder={tl("Ask the room…")}
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
