import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { assembleGuestPage, UI_ROOT } from "./guest-page.mjs";

// The backend serves the same page assembled at runtime (guest_html.rs); this
// copy is for inspecting the built Companion output.
const dist = resolve(UI_ROOT, "dist", "guest.html");
mkdirSync(dirname(dist), { recursive: true });
writeFileSync(dist, assembleGuestPage());
