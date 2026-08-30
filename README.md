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

# Rooms

Rooms is a Ryu App for sharing one model session across devices. The current
active Ryu node owns the room and runs its selected Mesh LLM model; a phone or
browser joins with a short-lived QR invite and receives the same transcript.

The first version is client-only. The phone does not download model weights or
contribute compute. Rooms use private reachability by default and keep node
credentials out of invite links and guest pages.

The app owns its room state in an out-of-process sidecar and reaches Core only
through the generic app HTTP, realtime, and governed model-stream seams. The
sidecar has no dependency on `apps/core`.
