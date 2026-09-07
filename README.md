<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./icon-dark.png" />
    <img src="./icon-light.png" alt="Rooms" width="144" />
  </picture>
</p>

<div align="center">

# Rooms

</div>

Shared model rooms hosted by the active Ryu node: QR invites, synchronized transcripts, and governed Mesh LLM inference across devices.

> **The public home of `ryu-rooms`.** Source, builds, and releases live here —
> binaries for every platform are attached to each release.
>
> This tree is generated from the Ryu monorepo, so commits pushed here
> directly are replaced on the next sync. **Pull requests are welcome** —
> open them here and they are ported into the monorepo, then flow back out.
> Ryu as a whole: https://github.com/amajorai/ryu

## Install

**App:** [Install](ryu://apps/@ryu/rooms) (opens the Ryu desktop app and asks you to confirm)

**CLI:**

```bash
ryu apps add @ryu/rooms
```

**Crate:**

```bash
cargo install ryu-rooms
```

Prebuilt binaries for every platform are attached to [each release](https://github.com/amajorai/ryu/releases).

## License

Apache-2.0 — see [LICENSE](./LICENSE).

## Build

Build the companion first so the sidecar embeds the generated guest carriage, then
run the sidecar checks:

```sh
bun run --cwd apps-store/rooms/ui build
cargo test --manifest-path apps-store/rooms/backend/Cargo.toml
cargo build --manifest-path apps-store/rooms/backend/Cargo.toml --bin ryu-rooms
```

The protected host API is mounted at `/api/rooms`; the public guest carriage and
session endpoints use `/api/rooms/guest`. The sidecar binds to loopback and accepts
only Core-issued ext-proxy requests.
