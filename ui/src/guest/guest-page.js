// The Rooms guest invite page (`GET /api/rooms/guest`). A plain script: the
// Rooms backend inlines it into `guest.html` with the catalog slice
// (`guest-catalog.generated.json`), so the public page needs no bundle, no
// host bridge and no Companion runtime. Copy goes through `tl()` (one
// double-quoted literal) so the i18n extractor and
// `scripts/sync-guest-catalog.mjs` see every sentence; the language follows
// `navigator.languages`.
(() => {
	const api = "/api/rooms";
	const RTL_LANGUAGES = new Set(["ar", "fa", "he", "ur"]);

	const readCatalogs = () => {
		try {
			const node = document.querySelector("#rooms-guest-catalog");
			const value = JSON.parse(node?.textContent || "{}");
			return value && typeof value === "object" ? value : {};
		} catch {
			return {};
		}
	};

	/** The catalog whose language and script match `tag`, `en`, or null. */
	const catalogLocaleFor = (tag, catalogs) => {
		if (Object.hasOwn(catalogs, tag)) {
			return tag;
		}
		try {
			const wanted = new Intl.Locale(tag).maximize();
			if (wanted.language === "en") {
				return "en";
			}
			for (const candidate of Object.keys(catalogs)) {
				const official = new Intl.Locale(candidate).maximize();
				if (
					official.language === wanted.language &&
					official.script === wanted.script
				) {
					return candidate;
				}
			}
		} catch {
			// An invalid tag matches no catalog.
		}
		return null;
	};

	const pickLocale = (catalogs) => {
		const preferred = navigator.languages?.length
			? navigator.languages
			: [navigator.language];
		for (const tag of preferred) {
			const locale = tag ? catalogLocaleFor(tag, catalogs) : null;
			if (locale) {
				return locale;
			}
		}
		return "en";
	};

	const catalogs = readCatalogs();
	const locale = pickLocale(catalogs);
	const messages = catalogs[locale] || {};
	/** Translate one English sentence; a sentence the slice lacks stays English. */
	const tl = (source) =>
		typeof messages[source] === "string" ? messages[source] : source;
	const language = locale.split("-")[0];
	document.documentElement.lang = locale;
	document.documentElement.dir = RTL_LANGUAGES.has(language) ? "rtl" : "ltr";

	const copy = {
		brand: tl("Ryu Room"),
		footNote: tl(
			"The host's active node runs the model. This device is a client; it does not download weights or compute the response."
		),
		joinButton: tl("Join room"),
		joinHeading: tl("Join the conversation."),
		joinIntro: tl(
			"This room runs on the host's active Ryu node. Your prompt and the synchronized transcript travel through this session; node credentials stay with the host."
		),
		nameLabel: tl("Your name"),
		roomTitle: tl("Room"),
		sendButton: tl("Send"),
	};
	const emptyText = tl("The room is ready. Send the first prompt.");
	const statusLabels = {
		closed: tl("Closed"),
		failed: tl("Failed"),
		idle: tl("Idle"),
		running: tl("Running"),
	};
	/** Messages for the backend's stable error `code`s. */
	const errorMessages = {
		conflict: tl("The room changed while you were sending. Try again."),
		forbidden: tl("You do not have access to this room."),
		inviteUnavailable: tl("This invite is invalid, expired, or revoked."),
		invalidArgs: tl("Check what you entered and try again."),
		roomBusy: tl("Another prompt is already running. Wait for it to finish."),
		roomClosed: tl("This room is closed."),
		roomNotFound: tl("This room no longer exists."),
		storeUnavailable: tl("The room is temporarily unavailable. Try again."),
		unauthorized: tl(
			"Your guest session has ended. Open the invite link again."
		),
	};

	document.title = copy.brand;
	for (const node of document.querySelectorAll("[data-copy]")) {
		const text = copy[node.dataset.copy];
		if (typeof text === "string") {
			node.textContent = text;
		}
	}
	document.querySelector("#name").placeholder = tl("e.g. Jiawei…");
	const promptInput = document.querySelector("#prompt");
	promptInput.placeholder = tl("Ask the room…");
	promptInput.setAttribute("aria-label", tl("Message to room"));

	const join = document.querySelector("#join");
	const room = document.querySelector("#room");
	const transcript = document.querySelector("#transcript");
	const connection = document.querySelector("#connection");
	const status = document.querySelector("#room-status");
	connection.textContent = tl("Invite-only");
	let current = null;
	let partial = "";
	let source = null;

	/** An error carrying the backend's code, shown through `errorMessages`. */
	class RoomRequestError extends Error {
		constructor(code) {
			super(code);
			this.code = code;
		}
	}
	const errorText = (node, text) => {
		node.textContent = text || "";
	};
	const failureText = (cause, fallback) =>
		(cause instanceof RoomRequestError && errorMessages[cause.code]) ||
		fallback;
	const statusText = (value) => statusLabels[value] || value;

	const call = async (path, init) => {
		const response = await fetch(api + path, {
			credentials: "include",
			...init,
			headers: {
				"content-type": "application/json",
				...(init?.headers || {}),
			},
		});
		const value = await response.json().catch(() => ({}));
		if (!response.ok) {
			throw new RoomRequestError(
				typeof value?.code === "string" ? value.code : ""
			);
		}
		return value;
	};
	const message = (role, text) => {
		const item = document.createElement("article");
		item.className = `message ${role}`;
		const avatar = document.createElement("div");
		avatar.className = "avatar";
		avatar.textContent = role === "user" ? tl("You") : tl("AI");
		const wrap = document.createElement("div");
		const label = document.createElement("div");
		label.className = "role";
		label.textContent = role === "user" ? tl("Prompt") : tl("Response");
		const body = document.createElement("div");
		body.className = "text";
		body.dir = "auto";
		body.textContent = text;
		wrap.append(label, body);
		item.append(avatar, wrap);
		return item;
	};
	const render = () => {
		if (!current) {
			return;
		}
		document.querySelector("#room-title").textContent = current.id.slice(0, 16);
		document.querySelector("#model").textContent = current.modelId;
		status.textContent = statusText(current.status);
		transcript.replaceChildren();
		if (current.messages.length || partial) {
			for (const item of current.messages) {
				transcript.append(message(item.role, item.text));
			}
			if (partial) {
				transcript.append(message("assistant", partial));
			}
		} else {
			const empty = document.createElement("div");
			empty.className = "empty";
			empty.textContent = emptyText;
			transcript.append(empty);
		}
		transcript.scrollTop = transcript.scrollHeight;
	};
	const applySnapshot = (value) => {
		if (value && value.contract === "rooms/1") {
			current = value;
			partial = "";
			render();
		}
	};
	const connect = () => {
		source = new EventSource(`${api}/guest/events`);
		source.addEventListener("open", () => {
			connection.textContent = tl("Connected");
			connection.style.color = "var(--primary)";
		});
		source.addEventListener("error", () => {
			connection.textContent = tl("Reconnecting…");
		});
		source.addEventListener("snapshot", (event) =>
			applySnapshot(JSON.parse(event.data))
		);
		source.addEventListener("turn.accepted", (event) => {
			const value = JSON.parse(event.data);
			applySnapshot(value.snapshot);
		});
		source.addEventListener("turn.delta", (event) => {
			const value = JSON.parse(event.data);
			partial = value.partialText || "";
			status.textContent = statusText("running");
			render();
		});
		for (const name of [
			"turn.completed",
			"turn.failed",
			"turn.canceled",
			"room.closed",
		]) {
			source.addEventListener(name, (event) => {
				const value = JSON.parse(event.data);
				if (value.snapshot) {
					applySnapshot(value.snapshot);
				}
			});
		}
	};
	const inviteFromHash = () =>
		new URLSearchParams(location.hash.slice(1)).get("invite");

	document
		.querySelector("#join-form")
		.addEventListener("submit", async (event) => {
			event.preventDefault();
			const button = document.querySelector("#join-button");
			const joinError = document.querySelector("#join-error");
			button.disabled = true;
			errorText(joinError, "");
			try {
				const invite = inviteFromHash();
				if (!invite) {
					errorText(joinError, tl("This link does not contain an invite."));
					return;
				}
				const value = await call("/guest/exchange", {
					method: "POST",
					body: JSON.stringify({
						invite,
						displayName: document.querySelector("#name").value,
					}),
				});
				history.replaceState(null, "", location.pathname);
				join.hidden = true;
				room.hidden = false;
				applySnapshot(value.room);
				connect();
			} catch (cause) {
				errorText(
					joinError,
					failureText(cause, tl("The invite could not be used."))
				);
			} finally {
				button.disabled = false;
			}
		});
	document
		.querySelector("#turn-form")
		.addEventListener("submit", async (event) => {
			event.preventDefault();
			const text = promptInput.value.trim();
			if (!(text && current) || current.status === "running") {
				return;
			}
			const button = document.querySelector("#send");
			const turnError = document.querySelector("#turn-error");
			button.disabled = true;
			errorText(turnError, "");
			try {
				await call("/guest/turns", {
					method: "POST",
					body: JSON.stringify({
						text,
						idempotencyKey: `guest-${Date.now()}-${Math.random().toString(36).slice(2)}`,
					}),
				});
				promptInput.value = "";
			} catch (cause) {
				errorText(
					turnError,
					failureText(cause, tl("The prompt was not accepted."))
				);
			} finally {
				button.disabled = false;
				promptInput.focus();
			}
		});
	if (inviteFromHash()) {
		document.querySelector("#name").focus();
	}
})();
